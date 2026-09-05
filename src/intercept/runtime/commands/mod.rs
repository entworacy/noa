use super::state::NEXT_COMMAND_ID;
use noa_agent_protocol::Channel;
mod queue;
use crate::failure::NoaError;
use base64::{Engine, engine::general_purpose::STANDARD};
pub(super) use queue::ChannelState;
use queue::Pending;
use serde::Deserialize;
use serde_json::Value;
use std::{
    sync::{Arc, OnceLock, atomic::Ordering, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tracing::warn;

pub(super) mod transport;
pub(super) struct KakaoCommand {
    id: u64,
    room_id: i64,
    action: KakaoAction,
    deadline: Instant,
}

enum KakaoAction {
    SendCustom {
        row_id: i64,
    },
    KickMember {
        user_id: i64,
    },
    HideMessage {
        log_id: i64,
        message_type: i32,
        message: String,
    },
    ChatOnRoom,
    LoadOpenChatMember {
        user_id: i64,
    },
    ShareOpenProfile,
    JoinOpenChat {
        url: String,
        profile_id: String,
        profile_kind: String,
        nickname: String,
        profile_image_url: Option<String>,
    },
    VoxStartCall {
        caller_id: i64,
        peer_ids: Vec<i64>,
        open_chat: bool,
        team_chat: bool,
        group_chat: bool,
    },
    VoxCreateRoom {
        title: String,
    },
    VoxJoinRoom {
        call_id: i64,
        host_v4: String,
        host_v6: String,
        port: i32,
    },
    VoxLeave {
        kind: String,
    },
    VoxStatus,
    VoxAudioStart {
        mode: String,
    },
    VoxAudioPush {
        encoded: String,
    },
    VoxAudioStop,
}

impl KakaoAction {
    fn channel(&self) -> Channel {
        match self {
            Self::VoxStartCall { .. }
            | Self::VoxCreateRoom { .. }
            | Self::VoxJoinRoom { .. }
            | Self::VoxLeave { .. } => Channel::Vox,
            Self::VoxStatus
            | Self::VoxAudioStart { .. }
            | Self::VoxAudioPush { .. }
            | Self::VoxAudioStop => Channel::Audio,
            _ => Channel::Control,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::SendCustom { .. } => "custom 발신",
            Self::KickMember { .. } => "참여자 강퇴",
            Self::HideMessage { .. } => "메시지 가리기",
            Self::ChatOnRoom => "CHATONROOM",
            Self::LoadOpenChatMember { .. } => "오픈채팅 멤버 프로필 조회",
            Self::ShareOpenProfile => "오픈프로필 공유 링크 조회",
            Self::JoinOpenChat { .. } => "오픈채팅 입장",
            Self::VoxStartCall { .. } => "VOX 보이스톡 시작",
            Self::VoxCreateRoom { .. } => "VOX 보이스룸 생성",
            Self::VoxJoinRoom { .. } => "VOX 보이스룸 입장",
            Self::VoxLeave { .. } => "VOX 세션 종료",
            Self::VoxStatus => "VOX 상태 조회",
            Self::VoxAudioStart { .. } => "VOX 오디오 송출 시작",
            Self::VoxAudioPush { .. } => "VOX PCM 전송",
            Self::VoxAudioStop => "VOX 오디오 송출 종료",
        }
    }

    fn timeout(&self) -> Duration {
        match self {
            Self::JoinOpenChat { .. }
            | Self::VoxStartCall { .. }
            | Self::VoxCreateRoom { .. }
            | Self::VoxJoinRoom { .. }
            | Self::VoxLeave { .. } => Duration::from_secs(40),
            Self::VoxAudioStart { .. }
            | Self::VoxAudioPush { .. }
            | Self::VoxAudioStop
            | Self::VoxStatus => Duration::from_secs(4),
            _ => Duration::from_secs(12),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenChatJoinResult {
    pub room_name: String,
    pub profile_applied: bool,
}
pub(super) static CHANNELS: OnceLock<[Arc<ChannelState<KakaoCommand>>; 3]> = OnceLock::new();

pub(super) fn fail_pending(message: &str) {
    if let Some(channels) = CHANNELS.get() {
        for state in channels {
            state.disconnect(message);
        }
    }
}
pub async fn send_custom(room_id: i64, row_id: i64) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || {
        send_command_blocking(room_id, KakaoAction::SendCustom { row_id })
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))??;
    Ok(())
}

pub async fn kick_member(room_id: i64, user_id: i64) -> Result<(), NoaError> {
    let started_at = unix_timestamp_millis();
    let result = tokio::task::spawn_blocking(move || {
        send_command_blocking(room_id, KakaoAction::KickMember { user_id })
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))?;
    match result {
        Ok(_) => Ok(()),
        Err(error) => {
            if let Some(detail) = wait_for_kick_rejection(room_id, user_id, started_at).await {
                warn!(room_id, user_id, %detail, "KakaoTalk 강퇴 서버 거부");
                Err(NoaError::Forbidden(detail))
            } else {
                Err(error)
            }
        }
    }
}

pub async fn hide_message(
    room_id: i64,
    log_id: i64,
    message_type: i32,
    message: String,
) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || {
        send_command_blocking(
            room_id,
            KakaoAction::HideMessage {
                log_id,
                message_type,
                message,
            },
        )
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))??;
    Ok(())
}

async fn wait_for_kick_rejection(room_id: i64, user_id: i64, since: i64) -> Option<String> {
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        if let Some(detail) = crate::intercept::loco::kick_failure_detail(room_id, user_id, since) {
            return Some(detail);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn unix_timestamp_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub async fn chat_on_room(room_id: i64) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || send_command_blocking(room_id, KakaoAction::ChatOnRoom))
        .await
        .map_err(|error| NoaError::Internal(error.to_string()))??;
    Ok(())
}

pub async fn load_open_chat_member(room_id: i64, user_id: i64) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || {
        send_command_blocking(room_id, KakaoAction::LoadOpenChatMember { user_id })
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))??;
    Ok(())
}

pub async fn share_open_profile(link_id: i64) -> Result<String, NoaError> {
    tokio::task::spawn_blocking(move || {
        send_command_blocking(link_id, KakaoAction::ShareOpenProfile)
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))??
    .ok_or_else(|| {
        NoaError::AndroidUnavailable(
            "KakaoTalk 후킹 에이전트가 공유 링크를 반환하지 않았습니다".to_string(),
        )
    })
}

pub async fn join_open_chat(
    url: String,
    profile_id: String,
    profile_kind: String,
    nickname: String,
    profile_image_url: Option<String>,
) -> Result<OpenChatJoinResult, NoaError> {
    let value = tokio::task::spawn_blocking(move || {
        send_command_blocking(
            0,
            KakaoAction::JoinOpenChat {
                url,
                profile_id,
                profile_kind,
                nickname,
                profile_image_url,
            },
        )
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))??
    .ok_or_else(|| {
        NoaError::AndroidUnavailable(
            "KakaoTalk 후킹 에이전트가 입장 결과를 반환하지 않았습니다".to_string(),
        )
    })?;
    let result: OpenChatJoinResult = serde_json::from_str(&value).map_err(|error| {
        NoaError::AndroidUnavailable(format!(
            "KakaoTalk 후킹 입장 결과가 잘못되었습니다: {error}"
        ))
    })?;
    if result.room_name.trim().is_empty() {
        return Err(NoaError::AndroidUnavailable(
            "KakaoTalk 후킹 입장 결과에 방 이름이 없습니다".to_string(),
        ));
    }
    Ok(result)
}

pub async fn vox_start_call(
    room_id: i64,
    caller_id: i64,
    peer_ids: Vec<i64>,
    open_chat: bool,
    team_chat: bool,
    group_chat: bool,
) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || {
        send_command_blocking(
            room_id,
            KakaoAction::VoxStartCall {
                caller_id,
                peer_ids,
                open_chat,
                team_chat,
                group_chat,
            },
        )
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))??;
    Ok(())
}

pub async fn vox_create_room(room_id: i64, title: String) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || {
        send_command_blocking(room_id, KakaoAction::VoxCreateRoom { title })
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))??;
    Ok(())
}

pub async fn vox_join_room(
    room_id: i64,
    call_id: i64,
    host_v4: String,
    host_v6: String,
    port: i32,
) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || {
        send_command_blocking(
            room_id,
            KakaoAction::VoxJoinRoom {
                call_id,
                host_v4,
                host_v6,
                port,
            },
        )
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))??;
    Ok(())
}

pub async fn vox_leave(room_id: i64, kind: String) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || {
        send_command_blocking(room_id, KakaoAction::VoxLeave { kind })
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))??;
    Ok(())
}

pub async fn vox_status() -> Result<Value, NoaError> {
    vox_json_command(0, KakaoAction::VoxStatus).await
}

pub async fn vox_audio_start(mode: String) -> Result<Value, NoaError> {
    vox_json_command(0, KakaoAction::VoxAudioStart { mode }).await
}

pub async fn vox_audio_push(bytes: Vec<u8>) -> Result<Value, NoaError> {
    if bytes.is_empty() {
        return Err(NoaError::BadRequest("PCM 청크가 비어 있습니다".to_string()));
    }
    let encoded = STANDARD.encode(bytes);
    vox_json_command(0, KakaoAction::VoxAudioPush { encoded }).await
}

pub async fn vox_audio_stop() -> Result<Value, NoaError> {
    vox_json_command(0, KakaoAction::VoxAudioStop).await
}

async fn vox_json_command(room_id: i64, action: KakaoAction) -> Result<Value, NoaError> {
    let value = tokio::task::spawn_blocking(move || send_command_blocking(room_id, action))
        .await
        .map_err(|error| NoaError::Internal(error.to_string()))??
        .ok_or_else(|| {
            NoaError::AndroidUnavailable("VOX 에이전트가 결과를 반환하지 않았습니다".to_string())
        })?;
    serde_json::from_str(&value).map_err(|error| {
        NoaError::AndroidUnavailable(format!("VOX 에이전트 응답이 잘못되었습니다: {error}"))
    })
}

pub(in crate::intercept) fn channel_active(channel: Channel) -> bool {
    CHANNELS
        .get()
        .is_some_and(|channels| channels[channel.index()].is_connected())
}

fn send_command_blocking(room_id: i64, action: KakaoAction) -> Result<Option<String>, NoaError> {
    let channel = action.channel();
    let state = CHANNELS
        .get()
        .map(|channels| &channels[channel.index()])
        .ok_or_else(|| {
            NoaError::AndroidUnavailable("네이티브 명령 채널이 준비되지 않았습니다".to_string())
        })?;
    let id = NEXT_COMMAND_ID.fetch_add(1, Ordering::Relaxed);
    let timeout = action.timeout();
    let label = action.label();
    let deadline = Instant::now() + timeout;
    let (response_sender, response_receiver) = mpsc::sync_channel(1);
    state
        .enqueue(
            id,
            Pending {
                sender: response_sender,
            },
            KakaoCommand {
                id,
                room_id,
                action,
                deadline,
            },
        )
        .map_err(|error| {
            NoaError::AndroidUnavailable(format!("{} 채널: {error}", channel.name()))
        })?;
    let result = response_receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()));
    state.remove(id);
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(message)) => Err(NoaError::AndroidUnavailable(message)),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(NoaError::AndroidUnavailable(format!(
            "KakaoTalk 후킹 {label} 호출 시간이 초과되었습니다"
        ))),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(NoaError::AndroidUnavailable(format!(
            "KakaoTalk 후킹 {label} 응답 채널이 종료되었습니다"
        ))),
    }
}
