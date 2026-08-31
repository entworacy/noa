use std::{
    path::Path,
    pin::pin,
    sync::Arc,
    time::{Duration, Instant},
};

use actix_multipart::Multipart;
use actix_web::{
    HttpRequest, HttpResponse, Responder,
    http::header,
    web::{self, BytesMut, Payload},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast};

use crate::{
    asset::{self, PreparedAsset},
    audit::AuditLog,
    device::KakaoRelay,
    failure::NoaError,
    kakao::{CustomMessageDraft, DeliveryState, OwnedProfile, OwnedProfileKind, RoomCatalog},
    model::{Member, Room, RoomEvent},
    settings::Settings,
};

mod iris_endpoint;
mod vox;
mod vox_audio;

const DASHBOARD: &str = include_str!("../../assets/dashboard.html");
const LOCO_DASHBOARD: &str = include_str!("../../assets/loco.html");
const KICK_VERIFICATION_TIMEOUT: Duration = Duration::from_secs(8);
const KICK_VERIFICATION_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Settings>,
    pub catalog: Option<Arc<RoomCatalog>>,
    pub audit: AuditLog,
    pub relay: Option<KakaoRelay>,
    pub rooms: Arc<RwLock<Vec<Room>>>,
    pub live_events: broadcast::Sender<RoomEvent>,
}

pub fn configure(config: &mut web::ServiceConfig) {
    config
        .route("/", web::get().to(dashboard))
        .route("/dashboard", web::get().to(dashboard))
        .route("/loco", web::get().to(loco_dashboard))
        .route("/health", web::get().to(health))
        .route("/api/status", web::get().to(status))
        .route("/api/rooms", web::get().to(rooms))
        .route("/api/rooms/{chat_id}", web::get().to(room))
        .route("/api/rooms/{chat_id}/leave", web::post().to(leave_chat))
        .route("/api/rooms/{chat_id}/kick", web::post().to(kick_member))
        .route("/api/open-chat/profiles", web::get().to(open_chat_profiles))
        .route(
            "/api/open-chat/profiles/share",
            web::post().to(share_open_profile),
        )
        .route(
            "/api/open-chat/profiles/share-member",
            web::post().to(share_member_open_profile),
        )
        .route("/api/open-chat/join", web::post().to(join_open_chat))
        .route("/api/events", web::get().to(events))
        .route("/api/events/stream", web::get().to(event_stream))
        .route("/api/loco", web::get().to(loco_packets))
        .route("/send", web::post().to(send))
        .route("/send/text", web::post().to(send_text))
        .route("/internal/iris/reply", web::post().to(iris_reply))
        .configure(iris_endpoint::configure)
        .configure(vox::configure)
        .configure(vox_audio::configure);
}

async fn loco_dashboard() -> impl Responder {
    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "text/html; charset=utf-8"))
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .body(LOCO_DASHBOARD)
}

#[derive(Deserialize)]
struct LocoQuery {
    limit: Option<usize>,
}

async fn loco_packets(
    req: HttpRequest,
    query: web::Query<LocoQuery>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(crate::intercept::loco_packets(
        query.limit.unwrap_or(500),
    )))
}

async fn dashboard() -> impl Responder {
    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "text/html; charset=utf-8"))
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .body(DASHBOARD)
}

async fn health() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "ok": true,
        "service": "noa",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResponse {
    version: &'static str,
    revision: &'static str,
    database_available: bool,
    android_available: bool,
    authentication_enabled: bool,
    room_count: usize,
    current_user_id: Option<String>,
    max_upload_bytes: usize,
    iris_hook_enabled: bool,
    iris_hook_active: bool,
    iris_endpoint_prefix: String,
    kakao_hook_enabled: bool,
    kakao_hook_active: bool,
}

async fn status(req: HttpRequest, state: web::Data<AppState>) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(StatusResponse {
        version: env!("CARGO_PKG_VERSION"),
        revision: env!("NOA_BUILD_REVISION"),
        database_available: state.catalog.is_some(),
        android_available: state.relay.is_some(),
        authentication_enabled: state.config.api_token.is_some(),
        room_count: state.rooms.read().await.len(),
        current_user_id: state
            .catalog
            .as_ref()
            .map(|catalog| catalog.current_user_id().to_string()),
        max_upload_bytes: state.config.max_upload_bytes,
        iris_hook_enabled: state.config.iris_hook.enabled,
        iris_hook_active: crate::intercept::active(),
        iris_endpoint_prefix: state.config.iris_hook.endpoint_prefix.clone(),
        kakao_hook_enabled: state.config.kakao_hook_enabled,
        kakao_hook_active: crate::intercept::kakao_active(),
    }))
}

async fn rooms(req: HttpRequest, state: web::Data<AppState>) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(state.rooms.read().await.clone()))
}

async fn room(
    req: HttpRequest,
    chat_id: web::Path<String>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    let chat_id = chat_id.into_inner();
    state
        .rooms
        .read()
        .await
        .iter()
        .find(|room| room.chat_id == chat_id)
        .cloned()
        .map(web::Json)
        .ok_or_else(|| NoaError::NotFound(format!("채팅방을 찾을 수 없습니다: {chat_id}")))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OpenChatJoinRequest {
    url: String,
    profile_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OpenProfileShareRequest {
    link_id: String,
    mode: Option<OpenProfileShareMode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OpenMemberProfileShareRequest {
    chat_id: String,
    user_id: serde_json::Value,
    mode: Option<OpenProfileShareMode>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum OpenProfileShareMode {
    Auto,
    Accessibility,
    Hook,
}

async fn open_chat_profiles(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(open_chat_profiles_action(&state).await?))
}

pub(super) async fn open_chat_profiles_action(
    state: &AppState,
) -> Result<serde_json::Value, NoaError> {
    Ok(serde_json::json!({
        "ok": true,
        "profiles": load_owned_profiles(state).await?
    }))
}

async fn share_open_profile(
    req: HttpRequest,
    body: web::Json<OpenProfileShareRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(
        share_open_profile_action(body.into_inner(), &state).await?,
    ))
}

async fn share_member_open_profile(
    req: HttpRequest,
    body: web::Json<OpenMemberProfileShareRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(
        share_member_open_profile_action(body.into_inner(), &state).await?,
    ))
}

pub(super) async fn share_open_profile_action(
    body: OpenProfileShareRequest,
    state: &AppState,
) -> Result<serde_json::Value, NoaError> {
    let use_hook = match body.mode.unwrap_or(OpenProfileShareMode::Auto) {
        OpenProfileShareMode::Auto => state.config.kakao_hook_enabled,
        OpenProfileShareMode::Accessibility => false,
        OpenProfileShareMode::Hook if state.config.kakao_hook_enabled => true,
        OpenProfileShareMode::Hook => {
            return Err(NoaError::BadRequest(
                "KAKAO_HOOK_ENABLED=false에서는 hook 모드를 사용할 수 없습니다".to_string(),
            ));
        }
    };
    let link_id_text = body.link_id.trim();
    if link_id_text.is_empty()
        || link_id_text.len() > 19
        || !link_id_text.bytes().all(|value| value.is_ascii_digit())
    {
        return Err(NoaError::BadRequest(
            "linkId는 0보다 큰 64비트 정수 문자열이어야 합니다".to_string(),
        ));
    }
    let link_id = link_id_text.parse::<i64>().map_err(|_| {
        NoaError::BadRequest("linkId는 0보다 큰 64비트 정수 문자열이어야 합니다".to_string())
    })?;
    if link_id <= 0 {
        return Err(NoaError::BadRequest(
            "linkId는 0보다 큰 64비트 정수 문자열이어야 합니다".to_string(),
        ));
    }

    let catalog = state.catalog.clone().ok_or_else(|| {
        NoaError::Database("KakaoTalk 프로필 데이터베이스를 사용할 수 없습니다".to_string())
    })?;
    let expected_url =
        tokio::task::spawn_blocking(move || catalog.open_profile_share_target(link_id))
            .await
            .map_err(|error| NoaError::Internal(error.to_string()))??;
    let has_database_url = expected_url.is_some();
    let relay = state.relay.clone().ok_or_else(|| {
        NoaError::AndroidUnavailable("Android JNI 계층이 초기화되지 않았습니다".to_string())
    })?;
    let url = relay
        .share_open_profile(link_id, expected_url, use_hook)
        .await?;
    let (mode, verification) = if use_hook && has_database_url {
        ("hook", "database+hook")
    } else if use_hook {
        ("hook", "database-member+hook")
    } else if has_database_url {
        ("accessibility", "database+ui")
    } else {
        ("accessibility", "database-member+ui+clipboard")
    };
    Ok(serde_json::json!({
        "ok": true,
        "linkId": link_id_text,
        "url": url,
        "mode": mode,
        "verified": true,
        "verification": verification
    }))
}

pub(super) async fn share_member_open_profile_action(
    body: OpenMemberProfileShareRequest,
    state: &AppState,
) -> Result<serde_json::Value, NoaError> {
    let chat_id = parse_chat_id(&body.chat_id)?;
    let user_id = parse_kick_user_id(Some(body.user_id))?
        .ok_or_else(|| NoaError::BadRequest("userId가 필요합니다".to_string()))?;
    let user_id_number = user_id
        .parse::<i64>()
        .map_err(|_| NoaError::BadRequest("userId는 0보다 큰 정수여야 합니다".to_string()))?;
    let catalog = state.catalog.clone().ok_or_else(|| {
        NoaError::Database("KakaoTalk 프로필 데이터베이스를 사용할 수 없습니다".to_string())
    })?;
    let room_catalog = catalog.clone();
    let room = tokio::task::spawn_blocking(move || room_catalog.room_snapshot(chat_id))
        .await
        .map_err(|error| NoaError::Internal(format!("멤버 프로필 조회 작업 실패: {error}")))??;
    let target = resolve_kick_target(&room, Some(&user_id), None)?;

    let mut profile_link_id =
        member_open_profile_link_id(catalog.clone(), chat_id, user_id_number).await?;
    let hook_requested = match body.mode.unwrap_or(OpenProfileShareMode::Auto) {
        OpenProfileShareMode::Auto => state.config.kakao_hook_enabled,
        OpenProfileShareMode::Accessibility => false,
        OpenProfileShareMode::Hook if state.config.kakao_hook_enabled => true,
        OpenProfileShareMode::Hook => {
            return Err(NoaError::BadRequest(
                "KAKAO_HOOK_ENABLED=false에서는 hook 모드를 사용할 수 없습니다".to_string(),
            ));
        }
    };
    if profile_link_id.is_none() && hook_requested {
        crate::intercept::load_open_chat_member(chat_id, user_id_number).await?;
        for _ in 0..20 {
            profile_link_id =
                member_open_profile_link_id(catalog.clone(), chat_id, user_id_number).await?;
            if profile_link_id.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
    if let Some(link_id) = profile_link_id {
        return share_open_profile_action(
            OpenProfileShareRequest {
                link_id: link_id.to_string(),
                mode: body.mode,
            },
            state,
        )
        .await;
    }
    if matches!(body.mode, Some(OpenProfileShareMode::Hook)) {
        return Err(NoaError::NotFound(
            "멤버 프로필을 서버에서 조회했지만 오픈프로필 linkId를 찾지 못했습니다".to_string(),
        ));
    }
    ensure_accessibility_profile_target_is_unique(&room, &target)?;
    let relay = state.relay.clone().ok_or_else(|| {
        NoaError::AndroidUnavailable("Android JNI 계층이 초기화되지 않았습니다".to_string())
    })?;
    let url = relay
        .share_member_open_profile(chat_id, room.name.clone(), target.nickname.clone())
        .await?;
    Ok(serde_json::json!({
        "ok": true,
        "chatId": body.chat_id,
        "userId": user_id,
        "linkId": serde_json::Value::Null,
        "url": url,
        "mode": "accessibility",
        "verified": true,
        "verification": "database-member+ui+clipboard"
    }))
}

async fn member_open_profile_link_id(
    catalog: Arc<RoomCatalog>,
    chat_id: i64,
    user_id: i64,
) -> Result<Option<i64>, NoaError> {
    tokio::task::spawn_blocking(move || catalog.member_open_profile_link_id(chat_id, user_id))
        .await
        .map_err(|error| NoaError::Internal(format!("멤버 linkId 조회 작업 실패: {error}")))?
}

async fn join_open_chat(
    req: HttpRequest,
    body: web::Json<OpenChatJoinRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(
        join_open_chat_action(body.into_inner(), &state).await?,
    ))
}

pub(super) async fn join_open_chat_action(
    body: OpenChatJoinRequest,
    state: &AppState,
) -> Result<serde_json::Value, NoaError> {
    let url = body.url.trim().to_string();
    if url.is_empty() || url.len() > 2048 {
        return Err(NoaError::BadRequest(
            "url은 1자 이상 2048자 이하여야 합니다".to_string(),
        ));
    }
    if !is_canonical_open_link_url(&url) {
        return Err(NoaError::BadRequest(
            "url은 https://open.kakao.com/o/... 형식이어야 합니다".to_string(),
        ));
    }
    let requested_profile_id = body
        .profile_id
        .map(|value| value.trim().to_string())
        .map(|value| {
            if value.is_empty() || value.chars().count() > 128 {
                Err(NoaError::BadRequest(
                    "profileId는 1자 이상 128자 이하여야 합니다".to_string(),
                ))
            } else {
                Ok(value)
            }
        })
        .transpose()?;
    let profiles = load_owned_profiles(state).await?;
    let selected = select_owned_profile(profiles, requested_profile_id.as_deref())?;
    let profile_kind = match selected.kind {
        OwnedProfileKind::Kakao => "kakao",
        OwnedProfileKind::OpenProfile => "open-profile",
    };
    tracing::info!(
        profile_id = selected.profile_id,
        profile_kind,
        "오픈채팅 입장 프로필 선택"
    );
    let relay = state.relay.clone().ok_or_else(|| {
        NoaError::AndroidUnavailable("Android JNI 계층이 초기화되지 않았습니다".to_string())
    })?;
    let (room_name, applied_profile) = relay
        .join_open_chat(
            url,
            selected.profile_id.clone(),
            profile_kind.to_string(),
            selected.nickname.clone(),
            selected.profile_image_url.clone(),
        )
        .await?;
    let profile_was_applied = applied_profile.is_some();
    Ok(serde_json::json!({
        "ok": true,
        "roomName": room_name,
        "profileId": profile_was_applied.then_some(selected.profile_id),
        "profile": applied_profile,
        "profileApplied": profile_was_applied,
        "mode": if state.config.kakao_hook_enabled { "hook" } else { "accessibility" },
        "message": "오픈채팅 입장을 완료했습니다"
    }))
}

fn is_canonical_open_link_url(value: &str) -> bool {
    value
        .strip_prefix("https://open.kakao.com/o/")
        .is_some_and(|token| {
            !token.is_empty()
                && token
                    .bytes()
                    .all(|character| character.is_ascii_alphanumeric())
        })
}

async fn load_owned_profiles(state: &AppState) -> Result<Vec<OwnedProfile>, NoaError> {
    let catalog = state.catalog.clone().ok_or_else(|| {
        NoaError::Database("KakaoTalk 프로필 데이터베이스를 사용할 수 없습니다".to_string())
    })?;
    tokio::task::spawn_blocking(move || catalog.owned_profiles())
        .await
        .map_err(|error| NoaError::Internal(error.to_string()))?
}

fn select_owned_profile(
    profiles: Vec<OwnedProfile>,
    requested_profile_id: Option<&str>,
) -> Result<OwnedProfile, NoaError> {
    match requested_profile_id {
        Some(profile_id) => profiles
            .into_iter()
            .find(|profile| profile.profile_id == profile_id)
            .ok_or_else(|| {
                NoaError::NotFound(format!("소유한 profileId를 찾지 못했습니다: {profile_id}"))
            }),
        None => profiles.into_iter().next().ok_or_else(|| {
            NoaError::NotFound("입장에 사용할 수 있는 소유 프로필이 없습니다".to_string())
        }),
    }
}

async fn leave_chat(
    req: HttpRequest,
    chat_id: web::Path<String>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(
        leave_chat_action(chat_id.into_inner(), &state).await?,
    ))
}

pub(super) async fn leave_chat_action(
    chat_id: String,
    state: &AppState,
) -> Result<serde_json::Value, NoaError> {
    let relay = state.relay.clone().ok_or_else(|| {
        NoaError::AndroidUnavailable("Android JNI 계층이 초기화되지 않았습니다".to_string())
    })?;
    let chat_id = parse_chat_id(&chat_id)?;
    let chat_id_text = chat_id.to_string();
    let room_name = state
        .rooms
        .read()
        .await
        .iter()
        .find(|room| room.chat_id == chat_id_text)
        .map(|room| room.name.clone())
        .ok_or_else(|| NoaError::NotFound(format!("채팅방을 찾을 수 없습니다: {chat_id}")))?;
    tracing::info!(chat_id, room_name, "채팅방 나가기 시작");
    relay.leave_chat(chat_id, room_name.clone()).await?;
    if let Some(catalog) = state.catalog.clone() {
        match tokio::task::spawn_blocking(move || catalog.snapshot()).await {
            Ok(Ok(rooms)) => *state.rooms.write().await = rooms,
            Ok(Err(error)) => tracing::warn!(%error, "퇴장 후 채팅방 목록 갱신 실패"),
            Err(error) => tracing::warn!(%error, "퇴장 후 채팅방 목록 작업 실패"),
        }
    }
    tracing::info!(chat_id, room_name, "채팅방 나가기 완료");
    Ok(serde_json::json!({
        "ok": true,
        "chatId": chat_id_text,
        "roomName": room_name,
        "message": "채팅방 나가기를 완료했습니다"
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KickMemberRequest {
    user_id: Option<serde_json::Value>,
    nickname: Option<String>,
}

async fn kick_member(
    req: HttpRequest,
    chat_id: web::Path<String>,
    body: web::Json<KickMemberRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(
        kick_member_action(chat_id.into_inner(), body.into_inner(), &state).await?,
    ))
}

pub(super) async fn kick_member_action(
    chat_id: String,
    body: KickMemberRequest,
    state: &AppState,
) -> Result<serde_json::Value, NoaError> {
    let relay = state.relay.clone().ok_or_else(|| {
        NoaError::AndroidUnavailable("Android JNI 계층이 초기화되지 않았습니다".to_string())
    })?;
    let chat_id = parse_chat_id(&chat_id)?;
    let user_id = parse_kick_user_id(body.user_id)?;
    let nickname = parse_kick_nickname(body.nickname)?;
    if user_id.is_none() && nickname.is_none() {
        return Err(NoaError::BadRequest(
            "nickname 또는 userId 중 하나가 필요합니다".to_string(),
        ));
    }
    let catalog = state.catalog.clone().ok_or_else(|| {
        NoaError::Database(
            "강퇴 대상과 결과를 검증할 KakaoTalk 데이터베이스가 없습니다".to_string(),
        )
    })?;
    let chat_id_text = chat_id.to_string();
    let target_catalog = catalog.clone();
    let room = tokio::task::spawn_blocking(move || target_catalog.room_snapshot(chat_id))
        .await
        .map_err(|error| NoaError::Internal(format!("강퇴 대상 조회 작업 실패: {error}")))??;
    let target = resolve_kick_target(&room, user_id.as_deref(), nickname.as_deref())?;
    if target.is_mine
        || target
            .user_id
            .parse::<i64>()
            .is_ok_and(|value| catalog.current_user_id() == value)
    {
        return Err(NoaError::BadRequest(
            "자기 자신은 강퇴할 수 없습니다".to_string(),
        ));
    }
    if !state.config.kakao_hook_enabled {
        ensure_accessibility_target_is_unique(&room, &target)?;
    }
    let target_user_id = target
        .user_id
        .parse::<i64>()
        .map_err(|_| NoaError::BadRequest(format!("올바르지 않은 userId: {}", target.user_id)))?;
    relay
        .kick_member(
            chat_id,
            room.name.clone(),
            target.nickname.clone(),
            target_user_id,
        )
        .await?;
    verify_kick_postcondition(catalog, state, chat_id, target_user_id).await?;
    Ok(serde_json::json!({
        "ok": true,
        "verified": true,
        "verification": "database",
        "chatId": chat_id_text,
        "roomName": room.name,
        "userId": target.user_id,
        "nickname": target.nickname,
        "message": "참여자 강퇴를 완료했습니다"
    }))
}

fn ensure_accessibility_target_is_unique(room: &Room, target: &Member) -> Result<(), NoaError> {
    if room
        .members
        .iter()
        .filter(|member| member.nickname == target.nickname)
        .count()
        == 1
    {
        return Ok(());
    }
    Err(NoaError::BadRequest(format!(
        "접근성 강퇴는 화면에서 userId를 구분할 수 없어 같은 닉네임의 참여자를 강퇴할 수 없습니다: {}",
        target.nickname
    )))
}

fn ensure_accessibility_profile_target_is_unique(
    room: &Room,
    target: &Member,
) -> Result<(), NoaError> {
    if room
        .members
        .iter()
        .filter(|member| member.nickname == target.nickname)
        .count()
        == 1
    {
        return Ok(());
    }
    Err(NoaError::BadRequest(format!(
        "접근성 프로필 공유는 화면에서 userId를 구분할 수 없어 같은 닉네임의 참여자를 선택할 수 없습니다: {}",
        target.nickname
    )))
}

async fn verify_kick_postcondition(
    catalog: Arc<RoomCatalog>,
    state: &AppState,
    chat_id: i64,
    user_id: i64,
) -> Result<(), NoaError> {
    let started = Instant::now();
    let deadline = started + KICK_VERIFICATION_TIMEOUT;
    let mut observed_present = false;
    let mut last_database_error: Option<String>;
    loop {
        let check_catalog = catalog.clone();
        match tokio::task::spawn_blocking(move || check_catalog.room_has_member(chat_id, user_id))
            .await
            .map_err(|error| NoaError::Internal(format!("강퇴 결과 검증 작업 실패: {error}")))?
        {
            Ok(false) => {
                let mut rooms = state.rooms.write().await;
                if let Some(room) = rooms
                    .iter_mut()
                    .find(|room| room.chat_id == chat_id.to_string())
                {
                    room.members
                        .retain(|member| member.user_id != user_id.to_string());
                    room.member_count = room.members.len();
                }
                tracing::info!(
                    chat_id,
                    user_id,
                    elapsed_ms = started.elapsed().as_millis(),
                    "강퇴 후 데이터베이스 참여자 제거 확인"
                );
                return Ok(());
            }
            Ok(true) => {
                observed_present = true;
                last_database_error = None;
            }
            Err(error) => last_database_error = Some(error.to_string()),
        }

        let now = Instant::now();
        if now >= deadline {
            break;
        }
        tokio::time::sleep(KICK_VERIFICATION_INTERVAL.min(deadline - now)).await;
    }

    if observed_present {
        let detail = last_database_error
            .map(|error| format!(" 마지막 조회 오류: {error}"))
            .unwrap_or_default();
        return Err(NoaError::AndroidUnavailable(format!(
            "강퇴 요청은 전달되었지만 {user_id} 참여자 제거를 데이터베이스에서 확인하지 못했습니다.{detail} 중복 실행 전에 실제 채팅방 상태를 확인하세요"
        )));
    }
    Err(NoaError::Database(format!(
        "강퇴 요청은 전달되었지만 결과 데이터베이스를 읽지 못했습니다: {}. 중복 실행 전에 실제 채팅방 상태를 확인하세요",
        last_database_error.unwrap_or_else(|| "알 수 없는 오류".to_string())
    )))
}

fn parse_kick_user_id(value: Option<serde_json::Value>) -> Result<Option<String>, NoaError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let raw = match value {
        serde_json::Value::String(value) => value.trim().to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        value => {
            return Err(NoaError::BadRequest(format!(
                "올바르지 않은 userId: {value}"
            )));
        }
    };
    raw.parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .map(|value| Some(value.to_string()))
        .ok_or_else(|| NoaError::BadRequest(format!("올바르지 않은 userId: {raw}")))
}

fn parse_kick_nickname(value: Option<String>) -> Result<Option<String>, NoaError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim().to_string();
    if value.is_empty() || value.chars().count() > 100 {
        return Err(NoaError::BadRequest(
            "nickname은 1자 이상 100자 이하여야 합니다".to_string(),
        ));
    }
    Ok(Some(value))
}

fn resolve_kick_target(
    room: &Room,
    user_id: Option<&str>,
    nickname: Option<&str>,
) -> Result<Member, NoaError> {
    if let Some(user_id) = user_id {
        let member = room
            .members
            .iter()
            .find(|member| member.user_id == user_id)
            .cloned()
            .ok_or_else(|| {
                NoaError::NotFound(format!(
                    "채팅방 {}에서 userId {user_id} 참여자를 찾지 못했습니다",
                    room.chat_id
                ))
            })?;
        if nickname.is_some_and(|nickname| nickname != member.nickname) {
            return Err(NoaError::BadRequest(format!(
                "userId {user_id}의 닉네임은 {}입니다",
                member.nickname
            )));
        }
        return Ok(member);
    }
    let nickname = nickname.ok_or_else(|| {
        NoaError::BadRequest("nickname 또는 userId 중 하나가 필요합니다".to_string())
    })?;
    let mut matches = room
        .members
        .iter()
        .filter(|member| member.nickname == nickname);
    let target = matches.next().cloned().ok_or_else(|| {
        NoaError::NotFound(format!(
            "채팅방 {}에서 nickname {nickname} 참여자를 찾지 못했습니다",
            room.chat_id
        ))
    })?;
    if matches.next().is_some() {
        return Err(NoaError::BadRequest(format!(
            "같은 닉네임의 참여자가 여러 명입니다. userId를 사용하세요: {nickname}"
        )));
    }
    Ok(target)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsQuery {
    chat_id: Option<String>,
    user_id: Option<String>,
    limit: Option<usize>,
}

async fn events(
    req: HttpRequest,
    query: web::Query<EventsQuery>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    let chat_id = query.chat_id.as_deref().map(parse_chat_id).transpose()?;
    let user_id = query.user_id.as_deref().map(parse_user_id).transpose()?;
    let limit = query.limit.unwrap_or(200);
    let events = match (chat_id, user_id) {
        (Some(chat_id), Some(user_id)) => state.audit.recent_for_user(chat_id, user_id, limit)?,
        (None, Some(_)) => {
            return Err(NoaError::BadRequest(
                "userId 필터에는 chatId가 필요합니다".to_string(),
            ));
        }
        (chat_id, None) => state.audit.recent(chat_id, limit)?,
    };
    Ok(web::Json(events))
}

async fn event_stream(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> Result<HttpResponse, NoaError> {
    authorize(&req, &state)?;
    let receiver = state.live_events.subscribe();
    let connected = stream::once(async {
        Ok::<_, actix_web::Error>(web::Bytes::from_static(b": connected\n\n"))
    });
    let updates = stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let payload =
                        serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
                    return Some((
                        Ok(web::Bytes::from(format!("data: {payload}\n\n"))),
                        receiver,
                    ));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Ok(HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "text/event-stream"))
        .insert_header((header::CACHE_CONTROL, "no-cache, no-transform"))
        .insert_header(("X-Accel-Buffering", "no"))
        .streaming(connected.chain(updates)))
}

#[derive(Default)]
struct UploadInput {
    chat_id: Option<i64>,
    file_name: Option<String>,
    mime_type: Option<String>,
    bytes: Vec<u8>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SendResponse {
    ok: bool,
    chat_id: String,
    file: PreparedAsset,
    message: &'static str,
}

async fn send(
    req: HttpRequest,
    payload: Payload,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    let relay = state.relay.clone().ok_or_else(|| {
        NoaError::AndroidUnavailable("Android JNI 계층이 초기화되지 않았습니다".to_string())
    })?;
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let upload = if content_type.starts_with("multipart/form-data") {
        read_multipart(&req, payload, state.config.max_upload_bytes).await?
    } else {
        read_raw(&req, payload, state.config.max_upload_bytes).await?
    };
    let chat_id = upload
        .chat_id
        .ok_or_else(|| NoaError::BadRequest("chatId가 필요합니다".to_string()))?;
    let file = asset::stage(
        &state.config,
        upload.bytes,
        upload.file_name.as_deref(),
        upload.mime_type.as_deref(),
    )
    .await?;
    relay.deliver_asset(chat_id, file.clone()).await?;
    Ok(web::Json(SendResponse {
        ok: true,
        chat_id: chat_id.to_string(),
        file,
        message: "KakaoTalk 공유 Intent를 실행했습니다",
    }))
}

async fn read_multipart(
    req: &HttpRequest,
    payload: Payload,
    limit: usize,
) -> Result<UploadInput, NoaError> {
    let mut multipart = Multipart::new(req.headers(), payload);
    let mut upload = UploadInput::default();
    while let Some(field) = multipart.next().await {
        let mut field =
            field.map_err(|error| NoaError::BadRequest(format!("multipart 해석 실패: {error}")))?;
        let disposition = field.content_disposition();
        let name = disposition
            .and_then(|value| value.get_name())
            .unwrap_or_default()
            .to_string();
        let filename = disposition
            .and_then(|value| value.get_filename())
            .map(str::to_string);
        let content_type = field.content_type().map(ToString::to_string);
        let mut value = BytesMut::new();
        while let Some(chunk) = field.next().await {
            let chunk = chunk
                .map_err(|error| NoaError::BadRequest(format!("업로드 읽기 실패: {error}")))?;
            if value.len() + chunk.len() > limit {
                return Err(NoaError::BadRequest(format!(
                    "파일이 {limit} bytes 제한을 초과했습니다"
                )));
            }
            value.extend_from_slice(&chunk);
        }
        match name.as_str() {
            "chatId" | "chat_id" => {
                let text = std::str::from_utf8(&value)
                    .map_err(|_| NoaError::BadRequest("chatId가 UTF-8이 아닙니다".to_string()))?;
                upload.chat_id = Some(parse_chat_id(text.trim())?);
            }
            "file" | "data" => {
                if !upload.bytes.is_empty() {
                    return Err(NoaError::BadRequest(
                        "한 요청에는 파일 하나만 허용됩니다".to_string(),
                    ));
                }
                upload.bytes = value.to_vec();
                upload.file_name = filename;
                upload.mime_type = content_type;
            }
            _ => {}
        }
    }
    if upload.bytes.is_empty() {
        return Err(NoaError::BadRequest("file 필드가 필요합니다".to_string()));
    }
    Ok(upload)
}

async fn read_raw(
    req: &HttpRequest,
    payload: Payload,
    limit: usize,
) -> Result<UploadInput, NoaError> {
    let query: std::collections::HashMap<String, String> =
        url::form_urlencoded::parse(req.query_string().as_bytes())
            .into_owned()
            .collect();
    let chat_id = query
        .get("chatId")
        .or_else(|| query.get("chat_id"))
        .map(|value| parse_chat_id(value))
        .transpose()?;
    let file_name = query.get("filename").cloned().or_else(|| {
        req.headers()
            .get("X-Filename")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    });
    let mime_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut bytes = BytesMut::new();
    let mut payload = pin!(payload);
    while let Some(chunk) = payload.next().await {
        let chunk =
            chunk.map_err(|error| NoaError::BadRequest(format!("업로드 읽기 실패: {error}")))?;
        if bytes.len() + chunk.len() > limit {
            return Err(NoaError::BadRequest(format!(
                "파일이 {limit} bytes 제한을 초과했습니다"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(UploadInput {
        chat_id,
        file_name,
        mime_type,
        bytes: bytes.to_vec(),
    })
}

#[derive(Deserialize)]
#[serde(untagged)]
enum IdInput {
    String(String),
    Number(i64),
}

impl IdInput {
    fn resolve(self) -> Result<i64, NoaError> {
        match self {
            Self::String(value) => parse_chat_id(&value),
            Self::Number(value) => Ok(value),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextRequest {
    chat_id: IdInput,
    text: String,
    thread_id: Option<IdInput>,
}

async fn send_text(
    req: HttpRequest,
    body: web::Json<TextRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    let relay = state.relay.clone().ok_or_else(|| {
        NoaError::AndroidUnavailable("Android JNI 계층이 초기화되지 않았습니다".to_string())
    })?;
    let body = body.into_inner();
    let chat_id = body.chat_id.resolve()?;
    let thread_id = body.thread_id.map(IdInput::resolve).transpose()?;
    if body.text.trim().is_empty() {
        return Err(NoaError::BadRequest(
            "text는 비어 있을 수 없습니다".to_string(),
        ));
    }
    relay.deliver_text(chat_id, body.text, thread_id).await?;
    Ok(web::Json(serde_json::json!({
        "ok": true, "chatId": chat_id.to_string(), "message": "KakaoTalk 답장 Intent를 실행했습니다"
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IrisReplyRequest {
    #[serde(rename = "type")]
    reply_type: String,
    room: IdInput,
    data: Option<serde_json::Value>,
    path: Option<String>,
}

#[derive(Serialize)]
struct IrisReplyResponse {
    success: bool,
    message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification: Option<&'static str>,
    #[serde(rename = "chatId", skip_serializing_if = "Option::is_none")]
    chat_id: Option<String>,
    #[serde(rename = "rowId", skip_serializing_if = "Option::is_none")]
    row_id: Option<i64>,
    #[serde(rename = "clientMessageId", skip_serializing_if = "Option::is_none")]
    client_message_id: Option<i64>,
}

async fn iris_reply(
    req: HttpRequest,
    body: web::Json<IrisReplyRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    let body = body.into_inner();
    let reply_type = body.reply_type.to_ascii_lowercase();
    if !state
        .config
        .iris_hook
        .types
        .iter()
        .any(|value| value == &reply_type)
    {
        return Err(NoaError::BadRequest(format!(
            "후킹하도록 설정되지 않은 reply type입니다: {reply_type}"
        )));
    }

    let custom = deliver_iris_extension(body, &state).await?;
    Ok(web::Json(IrisReplyResponse {
        success: true,
        message: "success",
        verified: custom.as_ref().map(|_| true),
        verification: custom.as_ref().map(|_| "database"),
        chat_id: custom.as_ref().map(|value| value.chat_id.to_string()),
        row_id: custom.as_ref().map(|value| value.row_id),
        client_message_id: custom.as_ref().map(|value| value.client_message_id),
    }))
}

struct IrisCustomResult {
    chat_id: i64,
    row_id: i64,
    client_message_id: i64,
}

async fn deliver_iris_extension(
    request: IrisReplyRequest,
    state: &AppState,
) -> Result<Option<IrisCustomResult>, NoaError> {
    match request.reply_type.to_ascii_lowercase().as_str() {
        "file" => deliver_iris_file(request, state).await.map(|_| None),
        "markdown" => deliver_iris_markdown(request, state).await.map(|_| None),
        "custom" => deliver_iris_custom(request, state).await.map(Some),
        reply_type => Err(NoaError::BadRequest(format!(
            "지원하지 않는 Iris reply type입니다: {reply_type}"
        ))),
    }
}

#[derive(Deserialize)]
struct IrisCustomData {
    #[serde(rename = "type")]
    message_type: i64,
    #[serde(default, alias = "chatId")]
    chat_id: Option<IdInput>,
    #[serde(default, alias = "threadId")]
    thread_id: Option<IdInput>,
    #[serde(default = "default_scope")]
    scope: i64,
    #[serde(default)]
    message: String,
    #[serde(default)]
    attachment: Option<serde_json::Value>,
    #[serde(default)]
    created_at: Option<i64>,
    #[serde(default)]
    client_message_id: Option<i64>,
    #[serde(default)]
    supplement: Option<serde_json::Value>,
    #[serde(default)]
    v: Option<serde_json::Value>,
    #[serde(default)]
    is_silence: i64,
}

fn default_scope() -> i64 {
    1
}

async fn deliver_iris_custom(
    request: IrisReplyRequest,
    state: &AppState,
) -> Result<IrisCustomResult, NoaError> {
    let outer_chat_id = request.room.resolve()?;
    let data: IrisCustomData = serde_json::from_value(request.data.ok_or_else(|| {
        NoaError::BadRequest("custom 요청에는 data 객체가 필요합니다".to_string())
    })?)
    .map_err(|error| {
        NoaError::BadRequest(format!("custom data 형식이 올바르지 않습니다: {error}"))
    })?;
    let inner_chat_id = data.chat_id.map(IdInput::resolve).transpose()?;
    if inner_chat_id.is_some_and(|value| value != outer_chat_id) {
        return Err(NoaError::BadRequest(
            "room과 data.chat_id가 서로 다릅니다".to_string(),
        ));
    }
    if !(1..=65_535).contains(&data.message_type) {
        return Err(NoaError::BadRequest(
            "custom data.type은 1부터 65535 사이여야 합니다".to_string(),
        ));
    }
    if data.message.len() > 1_000_000 {
        return Err(NoaError::BadRequest(
            "custom data.message가 너무 큽니다".to_string(),
        ));
    }
    if data.scope < 0 || data.is_silence < 0 {
        return Err(NoaError::BadRequest(
            "scope와 is_silence는 음수일 수 없습니다".to_string(),
        ));
    }
    let attachment =
        json_column(data.attachment, "attachment", Some("{}"))?.unwrap_or_else(|| "{}".to_string());
    let supplement = json_column(data.supplement, "supplement", None)?;
    let metadata = json_column(data.v, "v", None)?;
    let thread_id = data.thread_id.map(IdInput::resolve).transpose()?;
    let accessibility_message = data.message.clone();
    let accessibility_attachment = attachment.clone();
    let catalog = state.catalog.clone().ok_or_else(|| {
        NoaError::Database("KakaoTalk 데이터베이스를 사용할 수 없습니다".to_string())
    })?;
    let use_kakao_hook = state.config.kakao_hook_enabled;
    if use_kakao_hook && !crate::intercept::kakao_active() {
        return Err(NoaError::AndroidUnavailable(
            "KakaoTalk 후킹 에이전트가 준비되지 않았습니다".to_string(),
        ));
    }
    let accessibility_target = if use_kakao_hook {
        None
    } else {
        let relay = state.relay.clone().ok_or_else(|| {
            NoaError::AndroidUnavailable("Android JNI 계층이 초기화되지 않았습니다".to_string())
        })?;
        let room_catalog = catalog.clone();
        let room = tokio::task::spawn_blocking(move || room_catalog.room_snapshot(outer_chat_id))
            .await
            .map_err(|error| {
                NoaError::Internal(format!("재전송 채팅방 조회 작업 실패: {error}"))
            })??;
        Some((relay, room.name))
    };
    let draft = CustomMessageDraft {
        message_type: data.message_type,
        chat_id: outer_chat_id,
        thread_id,
        scope: data.scope,
        message: data.message,
        attachment,
        created_at: data.created_at,
        client_message_id: data.client_message_id,
        supplement,
        metadata,
        is_silence: data.is_silence,
    };
    let write_catalog = catalog.clone();
    let queued = tokio::task::spawn_blocking(move || write_catalog.enqueue_custom(draft))
        .await
        .map_err(|error| NoaError::Internal(error.to_string()))??;
    tracing::info!(
        chat_id = outer_chat_id,
        row_id = queued.row_id,
        client_message_id = queued.client_message_id,
        "custom 발신 대기 행 등록 완료"
    );
    if use_kakao_hook {
        crate::intercept::send_custom(outer_chat_id, queued.row_id).await?;
    } else {
        let (relay, room_name) = accessibility_target.unwrap();
        relay
            .resend_custom(
                outer_chat_id,
                room_name,
                accessibility_message,
                accessibility_attachment,
            )
            .await?;
    }
    for _ in 0..120 {
        let status_catalog = catalog.clone();
        let client_message_id = queued.client_message_id;
        let delivery =
            tokio::task::spawn_blocking(move || status_catalog.delivery_state(client_message_id))
                .await
                .map_err(|error| NoaError::Internal(error.to_string()))??;
        match delivery {
            DeliveryState::Delivered => {
                if use_kakao_hook {
                    tracing::info!(
                        chat_id = outer_chat_id,
                        row_id = queued.row_id,
                        client_message_id = queued.client_message_id,
                        "custom 후킹 발신 완료"
                    );
                } else {
                    tracing::info!(
                        chat_id = outer_chat_id,
                        row_id = queued.row_id,
                        client_message_id = queued.client_message_id,
                        "custom 접근성 발신 완료"
                    );
                }
                return Ok(IrisCustomResult {
                    chat_id: outer_chat_id,
                    row_id: queued.row_id,
                    client_message_id: queued.client_message_id,
                });
            }
            DeliveryState::Waiting | DeliveryState::Missing => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await
            }
        }
    }
    let method = if use_kakao_hook {
        "후킹"
    } else {
        "접근성"
    };
    Err(NoaError::AndroidUnavailable(format!(
        "DB 행 {}의 {method} 발신 완료를 확인하지 못했습니다",
        queued.row_id,
    )))
}

fn json_column(
    value: Option<serde_json::Value>,
    name: &str,
    default: Option<&str>,
) -> Result<Option<String>, NoaError> {
    let Some(value) = value else {
        return Ok(default.map(str::to_string));
    };
    if value.is_null() {
        return Ok(default.map(str::to_string));
    }
    if let Some(text) = value.as_str() {
        serde_json::from_str::<serde_json::Value>(text).map_err(|error| {
            NoaError::BadRequest(format!(
                "custom data.{name} JSON 문자열이 올바르지 않습니다: {error}"
            ))
        })?;
        return Ok(Some(text.to_string()));
    }
    serde_json::to_string(&value)
        .map(Some)
        .map_err(|error| NoaError::BadRequest(format!("custom data.{name} 변환 실패: {error}")))
}

async fn deliver_iris_markdown(
    request: IrisReplyRequest,
    state: &AppState,
) -> Result<(), NoaError> {
    let relay = state.relay.clone().ok_or_else(|| {
        NoaError::AndroidUnavailable("Android JNI 계층이 초기화되지 않았습니다".to_string())
    })?;
    let chat_id = request.room.resolve()?;
    let text = iris_markdown_text(request.data)?;
    relay.deliver_markdown(chat_id, text).await
}

fn iris_markdown_text(data: Option<serde_json::Value>) -> Result<String, NoaError> {
    let text = data
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or_else(|| {
            NoaError::BadRequest("markdown 요청의 data는 문자열이어야 합니다".to_string())
        })?;
    if text.trim().is_empty() {
        return Err(NoaError::BadRequest(
            "markdown 요청의 data는 비어 있을 수 없습니다".to_string(),
        ));
    }
    Ok(text)
}

async fn deliver_iris_file(request: IrisReplyRequest, state: &AppState) -> Result<(), NoaError> {
    let relay = state.relay.clone().ok_or_else(|| {
        NoaError::AndroidUnavailable("Android JNI 계층이 초기화되지 않았습니다".to_string())
    })?;
    let chat_id = request.room.resolve()?;
    let payload =
        iris_file_payload(request.data, request.path, state.config.max_upload_bytes).await?;
    let file = asset::stage(
        &state.config,
        payload.bytes,
        payload.file_name.as_deref().or(Some("iris-file")),
        payload.mime_type.as_deref(),
    )
    .await?;
    relay.deliver_asset(chat_id, file).await
}

async fn iris_file_payload(
    data: Option<serde_json::Value>,
    path: Option<String>,
    limit: usize,
) -> Result<IrisFilePayload, NoaError> {
    if let Some(data) = data {
        let encoded = data.as_str().ok_or_else(|| {
            NoaError::BadRequest("file 요청의 data는 Base64 문자열이어야 합니다".to_string())
        })?;
        decode_iris_file(encoded, limit)
    } else if let Some(path) = path {
        read_iris_path(&path, limit).await
    } else {
        Err(NoaError::BadRequest(
            "file 요청에는 data 또는 path가 필요합니다".to_string(),
        ))
    }
}

async fn read_iris_path(path: &str, limit: usize) -> Result<IrisFilePayload, NoaError> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(NoaError::BadRequest(
            "file 요청의 path는 절대 경로여야 합니다".to_string(),
        ));
    }
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| NoaError::BadRequest(format!("path 파일을 읽을 수 없습니다: {error}")))?;
    if !metadata.is_file() {
        return Err(NoaError::BadRequest(
            "path는 일반 파일이어야 합니다".to_string(),
        ));
    }
    if metadata.len() > limit as u64 {
        return Err(NoaError::BadRequest(format!(
            "파일이 {limit} bytes 제한을 초과했습니다"
        )));
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| NoaError::BadRequest(format!("path 파일을 읽을 수 없습니다: {error}")))?;
    if bytes.is_empty() {
        return Err(NoaError::BadRequest(
            "빈 파일은 전송할 수 없습니다".to_string(),
        ));
    }
    Ok(IrisFilePayload {
        bytes,
        file_name: path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned()),
        mime_type: mime_guess::from_path(path).first_raw().map(str::to_string),
    })
}

struct IrisFilePayload {
    bytes: Vec<u8>,
    file_name: Option<String>,
    mime_type: Option<String>,
}

fn decode_iris_file(value: &str, limit: usize) -> Result<IrisFilePayload, NoaError> {
    let (encoded, file_name, mime_type) = data_uri_parts(value)?;
    let compact: String = encoded
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    let estimated = compact.len().saturating_mul(3) / 4;
    if estimated > limit {
        return Err(NoaError::BadRequest(format!(
            "파일이 {limit} bytes 제한을 초과했습니다"
        )));
    }
    let bytes = STANDARD
        .decode(&compact)
        .or_else(|_| STANDARD_NO_PAD.decode(&compact))
        .map_err(|error| NoaError::BadRequest(format!("Base64 파일 해석 실패: {error}")))?;
    if bytes.is_empty() {
        return Err(NoaError::BadRequest(
            "빈 파일은 전송할 수 없습니다".to_string(),
        ));
    }
    if bytes.len() > limit {
        return Err(NoaError::BadRequest(format!(
            "파일이 {limit} bytes 제한을 초과했습니다"
        )));
    }
    Ok(IrisFilePayload {
        bytes,
        file_name,
        mime_type,
    })
}

fn data_uri_parts(value: &str) -> Result<(&str, Option<String>, Option<String>), NoaError> {
    let Some(value) = value.strip_prefix("data:") else {
        return Ok((value, None, None));
    };
    let (descriptor, encoded) = value
        .split_once(',')
        .ok_or_else(|| NoaError::BadRequest("data URI에 파일 데이터가 없습니다".to_string()))?;
    let mut fields = descriptor.split(';');
    let media_type = fields
        .next()
        .filter(|value| value.contains('/'))
        .map(str::to_string);
    let mut base64 = false;
    let mut file_name = None;
    for field in fields {
        if field.eq_ignore_ascii_case("base64") {
            base64 = true;
        } else if let Some((key, value)) = field.split_once('=')
            && matches!(key.to_ascii_lowercase().as_str(), "name" | "filename")
        {
            file_name = Some(decode_data_parameter(value));
        }
    }
    if !base64 {
        return Err(NoaError::BadRequest(
            "data URI는 base64 형식이어야 합니다".to_string(),
        ));
    }
    Ok((encoded, file_name, media_type))
}

fn decode_data_parameter(value: &str) -> String {
    let value = value.trim_matches(['"', '\'']);
    let encoded = format!("value={value}");
    url::form_urlencoded::parse(encoded.as_bytes())
        .next()
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| value.to_string())
}

fn parse_chat_id(value: &str) -> Result<i64, NoaError> {
    value
        .parse::<i64>()
        .map_err(|_| NoaError::BadRequest(format!("올바르지 않은 chatId: {value}")))
}

fn parse_user_id(value: &str) -> Result<i64, NoaError> {
    value
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| NoaError::BadRequest(format!("올바르지 않은 userId: {value}")))
}

pub(super) fn authorize(req: &HttpRequest, state: &AppState) -> Result<(), NoaError> {
    let Some(expected) = state.config.api_token.as_deref() else {
        return Ok(());
    };
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let custom = req
        .headers()
        .get("X-Noa-Token")
        .and_then(|value| value.to_str().ok());
    let query_token = url::form_urlencoded::parse(req.query_string().as_bytes())
        .find(|(key, _)| key == "token")
        .map(|(_, value)| value.into_owned());
    if bearer == Some(expected)
        || custom == Some(expected)
        || query_token.as_deref() == Some(expected)
    {
        Ok(())
    } else {
        Err(NoaError::Unauthorized)
    }
}

pub(super) fn authorize_iris_hook(req: &HttpRequest, state: &AppState) -> Result<(), NoaError> {
    if !state.config.iris_hook.enabled {
        return Err(NoaError::NotFound(
            "Iris 후킹 브리지가 비활성화되어 있습니다".to_string(),
        ));
    }
    let supplied = req
        .headers()
        .get("X-Noa-Hook-Token")
        .and_then(|value| value.to_str().ok());
    if supplied == Some(state.config.iris_hook.token.as_str()) {
        Ok(())
    } else {
        Err(NoaError::Unauthorized)
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{App, http::StatusCode, test};
    use tempfile::tempdir;

    use super::*;

    #[actix_web::test]
    async fn resolves_kick_target_by_nickname_or_user_id() {
        let room = Room {
            chat_id: "100".to_string(),
            name: "test".to_string(),
            members: vec![
                Member {
                    user_id: "10".to_string(),
                    nickname: "지민".to_string(),
                    ..Member::default()
                },
                Member {
                    user_id: "20".to_string(),
                    nickname: "민수".to_string(),
                    ..Member::default()
                },
            ],
            ..Room::default()
        };
        assert_eq!(
            resolve_kick_target(&room, None, Some("지민"))
                .unwrap()
                .user_id,
            "10"
        );
        assert_eq!(
            resolve_kick_target(&room, Some("20"), None)
                .unwrap()
                .nickname,
            "민수"
        );
        assert!(resolve_kick_target(&room, Some("20"), Some("지민")).is_err());
        assert!(resolve_kick_target(&room, Some("30"), None).is_err());
        assert!(ensure_accessibility_target_is_unique(&room, &room.members[0]).is_ok());

        let mut duplicated = room.clone();
        duplicated.members.push(Member {
            user_id: "30".to_string(),
            nickname: "지민".to_string(),
            ..Member::default()
        });
        assert!(
            ensure_accessibility_target_is_unique(&duplicated, &duplicated.members[0]).is_err()
        );
    }

    #[actix_web::test]
    async fn validates_kick_identifiers() {
        assert_eq!(
            parse_kick_user_id(Some(serde_json::json!(123))).unwrap(),
            Some("123".to_string())
        );
        assert_eq!(
            parse_kick_user_id(Some(serde_json::json!(" 456 "))).unwrap(),
            Some("456".to_string())
        );
        assert!(parse_kick_user_id(Some(serde_json::json!(0))).is_err());
        assert!(parse_kick_user_id(Some(serde_json::json!("abc"))).is_err());
        assert_eq!(
            parse_kick_nickname(Some(" 지민 ".to_string())).unwrap(),
            Some("지민".to_string())
        );
        assert!(parse_kick_nickname(Some(" ".to_string())).is_err());
    }

    #[actix_web::test]
    async fn custom_reply_reports_database_verification() {
        let response = IrisReplyResponse {
            success: true,
            message: "success",
            verified: Some(true),
            verification: Some("database"),
            chat_id: Some("123".to_string()),
            row_id: Some(456),
            client_message_id: Some(789),
        };
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["verified"], true);
        assert_eq!(json["verification"], "database");
        assert_eq!(json["clientMessageId"], 789);
    }

    #[actix_web::test]
    async fn selects_owned_profile_by_id_or_first() {
        let profiles = vec![
            OwnedProfile {
                profile_id: "first".to_string(),
                nickname: "첫 프로필".to_string(),
                profile_image_url: None,
                kind: crate::kakao::OwnedProfileKind::Kakao,
                is_main: true,
            },
            OwnedProfile {
                profile_id: "second".to_string(),
                nickname: "두 번째 프로필".to_string(),
                profile_image_url: None,
                kind: crate::kakao::OwnedProfileKind::OpenProfile,
                is_main: false,
            },
        ];
        assert_eq!(
            select_owned_profile(profiles.clone(), None)
                .unwrap()
                .profile_id,
            "first"
        );
        assert_eq!(
            select_owned_profile(profiles.clone(), Some("second"))
                .unwrap()
                .nickname,
            "두 번째 프로필"
        );
        assert!(select_owned_profile(profiles, Some("missing")).is_err());
        assert!(select_owned_profile(Vec::new(), None).is_err());

        assert!(
            serde_json::from_value::<OpenChatJoinRequest>(serde_json::json!({
                "url": "https://open.kakao.com/o/example",
                "profile": "이름은 더 이상 허용하지 않음"
            }))
            .is_err()
        );
        let request = serde_json::from_value::<OpenChatJoinRequest>(serde_json::json!({
            "url": "https://open.kakao.com/o/example",
            "profileId": null
        }))
        .unwrap();
        assert!(request.profile_id.is_none());
        assert!(is_canonical_open_link_url(
            "https://open.kakao.com/o/example"
        ));
        assert!(!is_canonical_open_link_url(
            "https://open.kakao.com/o/example?from=test"
        ));
        assert!(!is_canonical_open_link_url(
            "http://open.kakao.com/o/example"
        ));

        let share = serde_json::from_value::<OpenProfileShareRequest>(serde_json::json!({
            "linkId": "8382",
            "mode": "accessibility"
        }))
        .unwrap();
        assert!(matches!(
            share.mode,
            Some(OpenProfileShareMode::Accessibility)
        ));
        assert!(
            serde_json::from_value::<OpenProfileShareRequest>(serde_json::json!({
                "linkId": "8382",
                "mode": "invalid"
            }))
            .is_err()
        );
    }

    #[actix_web::test]
    async fn decodes_iris_file_data() {
        let plain = decode_iris_file("a G V s b G 8=", 32).unwrap();
        assert_eq!(plain.bytes, b"hello");
        assert!(plain.file_name.is_none());
        let file = decode_iris_file(
            "data:application/pdf;name=report%20final.pdf;base64,JVBERi0xLjQ=",
            32,
        )
        .unwrap();
        assert_eq!(file.bytes, b"%PDF-1.4");
        assert_eq!(file.file_name.as_deref(), Some("report final.pdf"));
        assert_eq!(file.mime_type.as_deref(), Some("application/pdf"));
        assert!(decode_iris_file("aGVsbG8=", 2).is_err());
        assert!(decode_iris_file("data:application/pdf,aGVsbG8=", 32).is_err());
    }

    #[actix_web::test]
    async fn iris_file_prefers_data_and_reads_path_as_fallback() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("fallback.txt");
        tokio::fs::write(&path, b"path").await.unwrap();
        let preferred = iris_file_payload(
            Some(serde_json::Value::String("ZGF0YQ==".to_string())),
            Some(path.to_string_lossy().into_owned()),
            32,
        )
        .await
        .unwrap();
        assert_eq!(preferred.bytes, b"data");
        let fallback = iris_file_payload(None, Some(path.to_string_lossy().into_owned()), 32)
            .await
            .unwrap();
        assert_eq!(fallback.bytes, b"path");
        assert_eq!(fallback.file_name.as_deref(), Some("fallback.txt"));
        assert!(iris_file_payload(None, None, 32).await.is_err());
    }

    #[actix_web::test]
    async fn validates_iris_markdown_data() {
        let text = iris_markdown_text(Some(serde_json::json!("**hello**"))).unwrap();
        assert_eq!(text, "**hello**");
        assert!(iris_markdown_text(None).is_err());
        assert!(iris_markdown_text(Some(serde_json::json!(42))).is_err());
        assert!(iris_markdown_text(Some(serde_json::json!("  "))).is_err());
    }

    #[actix_web::test]
    async fn serves_dashboard_and_enforces_api_token() {
        let directory = tempdir().unwrap();
        let config = Arc::new(Settings {
            bind: "127.0.0.1:0".to_string(),
            kakao_path: None,
            data_dir: directory.path().to_path_buf(),
            upload_dir: directory.path().join("uploads"),
            api_token: Some("test-token".to_string()),
            max_upload_bytes: 1024,
            poll_interval_ms: 100,
            snapshot_interval_ms: 500,
            send_interval_ms: 100,
            android_user_id: 0,
            calling_package: "com.android.shell".to_string(),
            file_provider_authority: None,
            image_max_dimension: 4096,
            jpeg_quality: 85,
            kakao_hook_enabled: true,
            chatonroom_interval_ms: 10_000,
            loco_history_limit: 1_000,
            iris_hook: crate::settings::IrisHookConfig {
                enabled: true,
                bridge_url: "http://127.0.0.1:4000/internal/iris/reply".to_string(),
                endpoint_bridge_url: "http://127.0.0.1:4000/internal/iris/endpoint".to_string(),
                endpoint_prefix: "/noa".to_string(),
                config_path: directory.path().join("iris-hook.json"),
                token: "hook-token".to_string(),
                types: vec![
                    "file".to_string(),
                    "markdown".to_string(),
                    "custom".to_string(),
                ],
            },
        });
        let (live_events, _) = broadcast::channel(8);
        let state = AppState {
            config,
            catalog: None,
            audit: AuditLog::open_archive(&directory.path().join("events.db")).unwrap(),
            relay: None,
            rooms: Arc::new(RwLock::new(Vec::new())),
            live_events,
        };
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure),
        )
        .await;

        let dashboard =
            test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;
        assert_eq!(dashboard.status(), StatusCode::OK);
        let body = test::read_body(dashboard).await;
        assert!(String::from_utf8_lossy(&body).contains("<title>noa · KakaoTalk"));

        let unauthorized = test::call_service(
            &app,
            test::TestRequest::get().uri("/api/status").to_request(),
        )
        .await;
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/api/status")
                .insert_header((header::AUTHORIZATION, "Bearer test-token"))
                .to_request(),
        )
        .await;
        assert_eq!(authorized.status(), StatusCode::OK);

        let loco_page = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/loco?token=test-token")
                .to_request(),
        )
        .await;
        assert_eq!(loco_page.status(), StatusCode::OK);
        let body = test::read_body(loco_page).await;
        assert!(String::from_utf8_lossy(&body).contains("<title>Noa LOCO</title>"));

        let loco_api = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/api/loco?limit=10")
                .insert_header((header::AUTHORIZATION, "Bearer test-token"))
                .to_request(),
        )
        .await;
        assert_eq!(loco_api.status(), StatusCode::OK);

        let hook_body = serde_json::json!({
            "type": "file",
            "room": "1234567890123456789",
            "data": "aGVsbG8="
        });
        let hook_unauthorized = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/internal/iris/reply")
                .set_json(&hook_body)
                .to_request(),
        )
        .await;
        assert_eq!(hook_unauthorized.status(), StatusCode::UNAUTHORIZED);

        let hook_without_android = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/internal/iris/reply")
                .insert_header(("X-Noa-Hook-Token", "hook-token"))
                .set_json(&hook_body)
                .to_request(),
        )
        .await;
        assert_eq!(
            hook_without_android.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );

        let endpoint_unauthorized = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/internal/iris/endpoint/health")
                .to_request(),
        )
        .await;
        assert_eq!(endpoint_unauthorized.status(), StatusCode::UNAUTHORIZED);

        let endpoint_health = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/internal/iris/endpoint/health")
                .insert_header(("X-Noa-Hook-Token", "hook-token"))
                .to_request(),
        )
        .await;
        assert_eq!(endpoint_health.status(), StatusCode::OK);
        let endpoint_health: serde_json::Value = test::read_body_json(endpoint_health).await;
        assert_eq!(endpoint_health["service"], "noa");

        let endpoint_index = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/internal/iris/endpoint/")
                .insert_header(("X-Noa-Hook-Token", "hook-token"))
                .to_request(),
        )
        .await;
        assert_eq!(endpoint_index.status(), StatusCode::OK);
        let endpoint_index: serde_json::Value = test::read_body_json(endpoint_index).await;
        assert!(
            endpoint_index["endpoints"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "POST /rooms/{chatId}/kick")
        );
        assert!(
            endpoint_index["endpoints"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "POST /open-chat/join")
        );
        assert!(
            endpoint_index["endpoints"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "GET /open-chat/profiles")
        );
        assert!(
            endpoint_index["endpoints"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "POST /open-chat/profiles/share")
        );
        assert!(
            endpoint_index["endpoints"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "POST /open-chat/profiles/share-member")
        );
        for endpoint in [
            "GET /vox/status",
            "POST /vox/voice-talk",
            "POST /vox/voice-rooms",
            "POST /vox/voice-rooms/join",
            "POST /vox/leave",
            "POST /vox/audio/start",
            "POST /vox/audio",
            "POST /vox/audio/stream",
            "POST /vox/audio/stop",
        ] {
            assert!(
                endpoint_index["endpoints"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|value| value == endpoint),
                "Iris endpoint index is missing {endpoint}"
            );
        }

        let endpoint_vox_status = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/internal/iris/endpoint/vox/status")
                .insert_header(("X-Noa-Hook-Token", "hook-token"))
                .to_request(),
        )
        .await;
        assert_eq!(
            endpoint_vox_status.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );

        let vox_post_cases: [(&str, &str, &[u8], StatusCode); 8] = [
            (
                "/internal/iris/endpoint/vox/voice-talk",
                "application/json",
                br#"{"chatId":"invalid"}"#,
                StatusCode::BAD_REQUEST,
            ),
            (
                "/internal/iris/endpoint/vox/voice-rooms",
                "application/json",
                br#"{"chatId":"invalid"}"#,
                StatusCode::BAD_REQUEST,
            ),
            (
                "/internal/iris/endpoint/vox/voice-rooms/join",
                "application/json",
                br#"{"chatId":"invalid"}"#,
                StatusCode::BAD_REQUEST,
            ),
            (
                "/internal/iris/endpoint/vox/leave",
                "application/json",
                br#"{"chatId":"invalid","kind":"voiceroom"}"#,
                StatusCode::BAD_REQUEST,
            ),
            (
                "/internal/iris/endpoint/vox/audio/start",
                "application/json",
                br#"{"mode":"replace"}"#,
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                "/internal/iris/endpoint/vox/audio",
                "application/octet-stream",
                &[0, 255],
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                "/internal/iris/endpoint/vox/audio/stream?mode=replace",
                "application/octet-stream",
                &[0, 255],
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                "/internal/iris/endpoint/vox/audio/stop",
                "application/octet-stream",
                &[],
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        ];
        for (uri, content_type, body, expected_status) in vox_post_cases {
            let response = test::call_service(
                &app,
                test::TestRequest::post()
                    .uri(uri)
                    .insert_header(("X-Noa-Hook-Token", "hook-token"))
                    .insert_header((header::CONTENT_TYPE, content_type))
                    .set_payload(web::Bytes::copy_from_slice(body))
                    .to_request(),
            )
            .await;
            assert_eq!(
                response.status(),
                expected_status,
                "unexpected status for {uri}"
            );
        }

        let endpoint_profiles_without_database = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/internal/iris/endpoint/open-chat/profiles")
                .insert_header(("X-Noa-Hook-Token", "hook-token"))
                .to_request(),
        )
        .await;
        assert_eq!(
            endpoint_profiles_without_database.status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );

        let endpoint_share_invalid_link = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/internal/iris/endpoint/open-chat/profiles/share")
                .insert_header(("X-Noa-Hook-Token", "hook-token"))
                .set_json(serde_json::json!({"linkId": "not-a-number"}))
                .to_request(),
        )
        .await;
        assert_eq!(
            endpoint_share_invalid_link.status(),
            StatusCode::BAD_REQUEST
        );

        let endpoint_share_member_invalid_chat = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/internal/iris/endpoint/open-chat/profiles/share-member")
                .insert_header(("X-Noa-Hook-Token", "hook-token"))
                .set_json(serde_json::json!({
                    "chatId": "not-a-number",
                    "userId": "10",
                    "mode": "accessibility"
                }))
                .to_request(),
        )
        .await;
        assert_eq!(
            endpoint_share_member_invalid_chat.status(),
            StatusCode::BAD_REQUEST
        );

        let endpoint_share_without_database = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/internal/iris/endpoint/open-chat/profiles/share")
                .insert_header(("X-Noa-Hook-Token", "hook-token"))
                .set_json(serde_json::json!({"linkId": "700"}))
                .to_request(),
        )
        .await;
        assert_eq!(
            endpoint_share_without_database.status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );

        let endpoint_join_unauthorized = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/internal/iris/endpoint/open-chat/join")
                .set_json(serde_json::json!({
                    "url": "https://open.kakao.com/o/example"
                }))
                .to_request(),
        )
        .await;
        assert_eq!(
            endpoint_join_unauthorized.status(),
            StatusCode::UNAUTHORIZED
        );

        let endpoint_join_without_android = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/internal/iris/endpoint/open-chat/join")
                .insert_header(("X-Noa-Hook-Token", "hook-token"))
                .set_json(serde_json::json!({
                    "url": "https://open.kakao.com/o/example"
                }))
                .to_request(),
        )
        .await;
        assert_eq!(
            endpoint_join_without_android.status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );

        let endpoint_kick_without_android = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/internal/iris/endpoint/rooms/123/kick")
                .insert_header(("X-Noa-Hook-Token", "hook-token"))
                .set_json(serde_json::json!({"userId": "456"}))
                .to_request(),
        )
        .await;
        assert_eq!(
            endpoint_kick_without_android.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );

        let endpoint_leave_without_android = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/internal/iris/endpoint/rooms/123/leave")
                .insert_header(("X-Noa-Hook-Token", "hook-token"))
                .to_request(),
        )
        .await;
        assert_eq!(
            endpoint_leave_without_android.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
