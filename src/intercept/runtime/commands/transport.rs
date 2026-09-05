use super::super::state::{KAKAO_FATAL_PID, KAKAO_TARGET_PID};
use super::{ChannelState, KakaoAction, KakaoCommand};
use crate::intercept::set_kakao_active;
use noa_agent_protocol::{Channel, VERSION};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, atomic::Ordering, mpsc},
    thread,
    time::{Duration, Instant},
};
use tracing::{info, warn};
struct NativeConnection {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    pid: u32,
}

impl NativeConnection {
    // Only the owning worker reads this socket; don't wait for another command to
    // notice an idle peer closing its channel. No unsolicited replies are valid.
    fn idle(&self) -> bool {
        if !self.reader.buffer().is_empty() || self.stream.set_nonblocking(true).is_err() {
            return false;
        }
        let result = self.stream.peek(&mut [0_u8; 1]);
        let restored = self.stream.set_nonblocking(false).is_ok();
        restored && matches!(result, Err(e) if e.kind() == std::io::ErrorKind::WouldBlock)
    }
}
pub(in crate::intercept::runtime) fn launch_kakao_bridge(
    listener: TcpListener,
    commands: mpsc::Receiver<KakaoCommand>,
    token: String,
    channel: Channel,
    state: Arc<ChannelState<KakaoCommand>>,
) -> Result<(), String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    thread::Builder::new()
        .name(format!("noa-kakao-{}", channel.name()))
        .spawn(move || kakao_bridge_loop(listener, commands, token, channel, state))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn kakao_bridge_loop(
    listener: TcpListener,
    commands: mpsc::Receiver<KakaoCommand>,
    token: String,
    channel: Channel,
    state: Arc<ChannelState<KakaoCommand>>,
) {
    let mut connection: Option<NativeConnection> = None;
    loop {
        if connection.as_ref().is_some_and(|c| {
            c.pid != KAKAO_TARGET_PID.load(Ordering::Acquire) || !state.is_connected() || !c.idle()
        }) {
            connection = None;
            state.disconnect("KakaoTalk 에이전트 채널 연결이 종료되거나 변경되었습니다");
            if channel == Channel::Control {
                set_kakao_active(false);
            }
        }
        if connection.is_none() {
            match listener.accept() {
                Ok((stream, _)) => match accept_kakao_connection(stream, &token, channel) {
                    Ok(accepted) => {
                        let pid = accepted.pid;
                        connection = Some(accepted);
                        state.connected();
                        if channel == Channel::Control {
                            set_kakao_active(true);
                        }
                        info!(
                            pid,
                            channel = channel.name(),
                            "KakaoTalk 에이전트 채널 준비 완료"
                        );
                    }
                    Err(message) => {
                        warn!(channel = channel.name(), error = %message, "에이전트 채널 연결 거부")
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => warn!(%error, "네이티브 연결 수락 실패"),
            }
        }
        match commands.recv_timeout(Duration::from_millis(10)) {
            Ok(command) => {
                if !state.contains(command.id) {
                    continue;
                }
                if Instant::now() >= command.deadline {
                    state.complete(
                        command.id,
                        Err("대기 중 명령 유효 시간이 지났습니다".into()),
                    );
                    continue;
                }
                let Some(active) = connection.as_mut() else {
                    state.complete(
                        command.id,
                        Err("에이전트 채널이 연결되지 않았습니다".into()),
                    );
                    continue;
                };
                if let Err(message) = transact_kakao(active, &token, &command, channel, &state) {
                    warn!(pid = active.pid, channel = channel.name(), error = %message, "에이전트 채널 연결 종료");
                    connection = None;
                    state.disconnect(&message);
                    if channel == Channel::Control {
                        set_kakao_active(false);
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn accept_kakao_connection(
    stream: TcpStream,
    token: &str,
    channel: Channel,
) -> Result<NativeConnection, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    stream.set_nodelay(true).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
        return Err("준비 응답 없이 연결이 종료되었습니다".into());
    }
    let hello: Value = serde_json::from_str(line.trim()).map_err(|e| e.to_string())?;
    if hello.get("token").and_then(Value::as_str) != Some(token)
        || hello.get("protocol").and_then(Value::as_u64) != Some(VERSION as u64)
        || hello.get("channel").and_then(Value::as_str) != Some(channel.name())
    {
        return Err("에이전트 인증, 프로토콜 또는 채널이 올바르지 않습니다".into());
    }
    let pid = hello
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .filter(|pid| *pid != 0 && *pid == KAKAO_TARGET_PID.load(Ordering::Acquire))
        .ok_or_else(|| "대상 KakaoTalk 프로세스 PID가 일치하지 않습니다".to_string())?;
    if hello.get("event").and_then(Value::as_str) == Some("error") {
        if channel == Channel::Control
            && hello.get("retryable").and_then(Value::as_bool) != Some(true)
        {
            KAKAO_FATAL_PID.store(pid, Ordering::Release);
        }
        return Err(format!(
            "네이티브 에이전트 초기화 실패: {}",
            hello
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ));
    }
    if hello.get("event").and_then(Value::as_str) != Some("ready") {
        return Err("에이전트 준비 응답이 아닙니다".into());
    }
    if channel == Channel::Control {
        KAKAO_FATAL_PID.store(0, Ordering::Release);
    }
    Ok(NativeConnection {
        stream,
        reader,
        pid,
    })
}

fn transact_kakao(
    connection: &mut NativeConnection,
    token: &str,
    command: &KakaoCommand,
    channel: Channel,
    state: &ChannelState<KakaoCommand>,
) -> Result<(), String> {
    if !state.contains(command.id) {
        return Ok(());
    }
    if Instant::now() >= command.deadline {
        state.complete(command.id, Err("명령 유효 시간이 지났습니다".into()));
        return Ok(());
    }
    let request = match &command.action {
        KakaoAction::SendCustom { row_id } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "send-custom",
            "room": command.room_id,
            "row": row_id,
        }),
        KakaoAction::KickMember { user_id } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "kick-member",
            "room": command.room_id,
            "user": user_id,
        }),
        KakaoAction::HideMessage {
            log_id,
            message_type,
            message,
        } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "hide-message",
            "room": command.room_id,
            "log": log_id,
            "logType": message_type,
            "message": message,
        }),
        KakaoAction::ChatOnRoom => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "chat-on-room",
            "room": command.room_id,
        }),
        KakaoAction::LoadOpenChatMember { user_id } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "load-open-chat-member",
            "room": command.room_id,
            "user": user_id,
        }),
        KakaoAction::ShareOpenProfile => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "share-open-profile",
            "link": command.room_id,
        }),
        KakaoAction::JoinOpenChat {
            url,
            profile_id,
            profile_kind,
            nickname,
            profile_image_url,
        } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "join-open-chat",
            "url": url,
            "profileId": profile_id,
            "profileKind": profile_kind,
            "nickname": nickname,
            "profileImageUrl": profile_image_url,
        }),
        KakaoAction::VoxStartCall {
            caller_id,
            peer_ids,
            open_chat,
            team_chat,
            group_chat,
        } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "vox-start-call",
            "room": command.room_id,
            "caller": caller_id,
            "peers": peer_ids,
            "openChat": open_chat,
            "teamChat": team_chat,
            "groupChat": group_chat,
        }),
        KakaoAction::VoxCreateRoom { title } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "vox-create-room",
            "room": command.room_id,
            "title": title,
        }),
        KakaoAction::VoxJoinRoom {
            call_id,
            host_v4,
            host_v6,
            port,
        } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "vox-join-room",
            "room": command.room_id,
            "call": call_id,
            "hostV4": host_v4,
            "hostV6": host_v6,
            "port": port,
        }),
        KakaoAction::VoxLeave { kind } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "vox-leave",
            "room": command.room_id,
            "kind": kind,
        }),
        KakaoAction::VoxStatus => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "vox-status",
        }),
        KakaoAction::VoxAudioStart { mode } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "vox-audio-start",
            "mode": mode,
        }),
        KakaoAction::VoxAudioPush { encoded } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "vox-audio-push",
            "audio": encoded,
        }),
        KakaoAction::VoxAudioStop => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "vox-audio-stop",
        }),
    };
    let remaining = command.deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("명령 유효 시간이 지났습니다".into());
    }
    connection
        .stream
        .set_read_timeout(Some(remaining))
        .map_err(|error| error.to_string())?;
    connection
        .stream
        .set_write_timeout(Some(remaining))
        .map_err(|error| error.to_string())?;
    if Channel::for_action(request["action"].as_str().unwrap()) != Some(channel) {
        return Err("명령과 전송 채널이 일치하지 않습니다".into());
    }
    serde_json::to_writer(&mut connection.stream, &request).map_err(|error| error.to_string())?;
    connection
        .stream
        .write_all(b"\n")
        .map_err(|error| error.to_string())?;
    connection
        .stream
        .flush()
        .map_err(|error| error.to_string())?;
    let remaining = command.deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("명령 응답 시간이 초과되었습니다".into());
    }
    connection
        .stream
        .set_read_timeout(Some(remaining))
        .map_err(|e| e.to_string())?;
    let mut line = String::new();
    if connection
        .reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Err("네이티브 에이전트 응답 없이 연결이 종료되었습니다".to_string());
    }
    let response: Value = serde_json::from_str(line.trim()).map_err(|error| error.to_string())?;
    if response.get("id").and_then(Value::as_u64) != Some(command.id) {
        return Err("네이티브 에이전트 응답 ID가 일치하지 않습니다".to_string());
    }
    let result = if response.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(response
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_string))
    } else {
        Err(response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("KakaoTalk Rust 에이전트 호출 실패")
            .to_string())
    };
    state.complete(command.id, result);
    Ok(())
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
