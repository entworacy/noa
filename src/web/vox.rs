use std::{collections::HashSet, sync::Arc};

use actix_web::{HttpRequest, Responder, web};
use serde::Deserialize;

use super::{AppState, authorize};
use crate::{failure::NoaError, kakao::RoomCatalog, model::Room};

pub(super) fn configure(config: &mut web::ServiceConfig) {
    config
        .route("/api/vox/status", web::get().to(status))
        .route("/api/vox/voice-talk", web::post().to(start_voice_talk))
        .route("/api/vox/voice-rooms", web::post().to(create_voice_room))
        .route("/api/vox/voice-rooms/join", web::post().to(join_voice_room))
        .route("/api/vox/leave", web::post().to(leave));
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct VoiceTalkRequest {
    chat_id: String,
    peer_ids: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct VoiceRoomRequest {
    chat_id: String,
    title: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct JoinVoiceRoomRequest {
    chat_id: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum VoxKind {
    Cecall,
    Voiceroom,
}

impl VoxKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Cecall => "cecall",
            Self::Voiceroom => "voiceroom",
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct LeaveRequest {
    chat_id: String,
    kind: VoxKind,
}

async fn status(req: HttpRequest, state: web::Data<AppState>) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(status_action(&state).await?))
}

pub(super) async fn status_action(state: &AppState) -> Result<serde_json::Value, NoaError> {
    ensure_vox_enabled(state)?;
    crate::intercept::vox_status().await
}

async fn start_voice_talk(
    req: HttpRequest,
    body: web::Json<VoiceTalkRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(
        start_voice_talk_action(body.into_inner(), &state).await?,
    ))
}

pub(super) async fn start_voice_talk_action(
    body: VoiceTalkRequest,
    state: &AppState,
) -> Result<serde_json::Value, NoaError> {
    ensure_vox_enabled(state)?;
    let chat_id = parse_positive_id("chatId", &body.chat_id)?;
    let catalog = catalog(state)?;
    let room = room_snapshot(catalog.clone(), chat_id).await?;
    if !matches!(
        room.room_type.as_str(),
        "DirectChat" | "MultiChat" | "OD" | "OM"
    ) {
        return Err(NoaError::BadRequest(format!(
            "보이스톡을 시작할 수 없는 채팅방 유형입니다: {}",
            room.room_type
        )));
    }
    let caller_id = catalog.current_user_id();
    if caller_id <= 0 {
        return Err(NoaError::Database(
            "KakaoTalk 현재 사용자 ID를 확인할 수 없습니다".to_string(),
        ));
    }
    let peer_ids = resolve_peer_ids(&room, caller_id, body.peer_ids)?;
    let open_chat = is_open_chat(&room);
    let group_chat = is_group_chat(&room);
    crate::intercept::vox_start_call(
        chat_id,
        caller_id,
        peer_ids.clone(),
        open_chat,
        false,
        group_chat,
    )
    .await?;
    Ok(serde_json::json!({
        "ok": true,
        "chatId": chat_id.to_string(),
        "peerIds": peer_ids.into_iter().map(|id| id.to_string()).collect::<Vec<_>>(),
        "openChat": open_chat,
        "groupChat": group_chat
    }))
}

async fn create_voice_room(
    req: HttpRequest,
    body: web::Json<VoiceRoomRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(
        create_voice_room_action(body.into_inner(), &state).await?,
    ))
}

pub(super) async fn create_voice_room_action(
    body: VoiceRoomRequest,
    state: &AppState,
) -> Result<serde_json::Value, NoaError> {
    ensure_vox_enabled(state)?;
    let chat_id = parse_positive_id("chatId", &body.chat_id)?;
    let room = room_snapshot(catalog(state)?, chat_id).await?;
    require_open_multi(&room)?;
    let title = body
        .title
        .unwrap_or_else(|| room.name.clone())
        .trim()
        .to_string();
    if title.is_empty() || title.chars().count() > 100 {
        return Err(NoaError::BadRequest(
            "title은 1자 이상 100자 이하여야 합니다".to_string(),
        ));
    }
    crate::intercept::vox_create_room(chat_id, title.clone()).await?;
    Ok(serde_json::json!({
        "ok": true,
        "chatId": chat_id.to_string(),
        "title": title
    }))
}

async fn join_voice_room(
    req: HttpRequest,
    body: web::Json<JoinVoiceRoomRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(
        join_voice_room_action(body.into_inner(), &state).await?,
    ))
}

pub(super) async fn join_voice_room_action(
    body: JoinVoiceRoomRequest,
    state: &AppState,
) -> Result<serde_json::Value, NoaError> {
    ensure_vox_enabled(state)?;
    let chat_id = parse_positive_id("chatId", &body.chat_id)?;
    let catalog = catalog(state)?;
    let room = room_snapshot(catalog.clone(), chat_id).await?;
    require_open_multi(&room)?;
    let info_catalog = catalog.clone();
    let info = tokio::task::spawn_blocking(move || info_catalog.voiceroom_join_info(chat_id))
        .await
        .map_err(|error| {
            NoaError::Internal(format!("보이스룸 접속정보 조회 작업 실패: {error}"))
        })??;
    crate::intercept::vox_join_room(
        info.chat_id,
        info.call_id,
        info.host_v4,
        info.host_v6,
        info.port,
    )
    .await?;
    Ok(serde_json::json!({
        "ok": true,
        "chatId": chat_id.to_string(),
        "callId": info.call_id.to_string()
    }))
}

async fn leave(
    req: HttpRequest,
    body: web::Json<LeaveRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(leave_action(body.into_inner(), &state).await?))
}

pub(super) async fn leave_action(
    body: LeaveRequest,
    state: &AppState,
) -> Result<serde_json::Value, NoaError> {
    ensure_vox_enabled(state)?;
    let chat_id = parse_positive_id("chatId", &body.chat_id)?;
    room_snapshot(catalog(state)?, chat_id).await?;
    crate::intercept::vox_leave(chat_id, body.kind.as_str().to_string()).await?;
    Ok(serde_json::json!({
        "ok": true,
        "chatId": chat_id.to_string(),
        "kind": body.kind.as_str()
    }))
}

fn resolve_peer_ids(
    room: &Room,
    caller_id: i64,
    requested: Option<Vec<String>>,
) -> Result<Vec<i64>, NoaError> {
    let active = room
        .members
        .iter()
        .filter_map(|member| member.user_id.parse::<i64>().ok())
        .collect::<HashSet<_>>();
    if !active.contains(&caller_id) {
        return Err(NoaError::BadRequest(
            "현재 사용자가 채팅방의 활성 참여자가 아닙니다".to_string(),
        ));
    }
    let candidates = requested.unwrap_or_else(|| {
        room.members
            .iter()
            .filter(|member| !member.is_mine)
            .map(|member| member.user_id.clone())
            .collect()
    });
    let mut seen = HashSet::new();
    let mut peers = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let peer = parse_positive_id("peerIds", &candidate)?;
        if peer == caller_id {
            return Err(NoaError::BadRequest(
                "peerIds에는 현재 사용자를 넣을 수 없습니다".to_string(),
            ));
        }
        if !active.contains(&peer) {
            return Err(NoaError::BadRequest(format!(
                "활성 참여자가 아닌 peerId입니다: {peer}"
            )));
        }
        if seen.insert(peer) {
            peers.push(peer);
        }
    }
    if peers.is_empty() {
        return Err(NoaError::BadRequest(
            "보이스톡을 걸 활성 상대가 없습니다".to_string(),
        ));
    }
    Ok(peers)
}

pub(super) fn ensure_vox_enabled(state: &AppState) -> Result<(), NoaError> {
    if state.config.kakao_hook_enabled {
        Ok(())
    } else {
        Err(NoaError::AndroidUnavailable(
            "KAKAO_HOOK_ENABLED=false에서는 VOX 후킹을 사용할 수 없습니다".to_string(),
        ))
    }
}

fn catalog(state: &AppState) -> Result<Arc<RoomCatalog>, NoaError> {
    state.catalog.clone().ok_or_else(|| {
        NoaError::Database("VOX 대상을 검증할 KakaoTalk 데이터베이스가 없습니다".to_string())
    })
}

async fn room_snapshot(catalog: Arc<RoomCatalog>, chat_id: i64) -> Result<Room, NoaError> {
    tokio::task::spawn_blocking(move || catalog.room_snapshot(chat_id))
        .await
        .map_err(|error| NoaError::Internal(format!("VOX 채팅방 조회 작업 실패: {error}")))?
}

fn parse_positive_id(name: &str, value: &str) -> Result<i64, NoaError> {
    value
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| NoaError::BadRequest(format!("{name}는 0보다 큰 정수여야 합니다")))
}

fn is_open_chat(room: &Room) -> bool {
    matches!(room.room_type.as_str(), "OD" | "OM")
}

fn is_group_chat(room: &Room) -> bool {
    matches!(room.room_type.as_str(), "MultiChat" | "SMultiChat" | "OM")
}

fn require_open_multi(room: &Room) -> Result<(), NoaError> {
    if room.room_type == "OM" {
        Ok(())
    } else {
        Err(NoaError::BadRequest(format!(
            "오픈채팅 보이스룸은 OpenMulti(OM) 방에서만 사용할 수 있습니다: {}",
            room.room_type
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Member;

    #[test]
    fn peers_are_derived_from_active_room_members() {
        let room = Room {
            members: vec![
                Member {
                    user_id: "10".to_string(),
                    is_mine: true,
                    ..Member::default()
                },
                Member {
                    user_id: "20".to_string(),
                    ..Member::default()
                },
            ],
            ..Room::default()
        };
        assert_eq!(resolve_peer_ids(&room, 10, None).unwrap(), vec![20]);
        assert!(resolve_peer_ids(&room, 10, Some(vec!["30".to_string()])).is_err());
    }

    #[test]
    fn group_call_flag_follows_room_type_not_selected_peer_count() {
        let mut room = Room {
            room_type: "OM".to_string(),
            ..Room::default()
        };
        assert!(is_group_chat(&room));

        room.room_type = "DirectChat".to_string();
        assert!(!is_group_chat(&room));
    }
}
