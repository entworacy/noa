use actix_web::{HttpRequest, HttpResponse, Responder, web};

use super::{
    AppState, KickMemberRequest, OpenChatJoinRequest, OpenMemberProfileShareRequest,
    OpenProfileShareRequest, authorize_iris_hook, hide_message_action, join_open_chat_action,
    kick_member_action, leave_chat_action, open_chat_profiles_action,
    share_member_open_profile_action, share_open_profile_action, vox, vox_audio,
};
use crate::failure::NoaError;

/// Routes exposed through the injected Iris endpoint gateway.
///
/// A route registered at `/health` here is available from Iris at
/// `{NOA_IRIS_ENDPOINT_PREFIX}/health` (the default is `/noa/health`).
pub(super) fn configure(config: &mut web::ServiceConfig) {
    config.service(
        web::scope("/internal/iris/endpoint")
            .route("", web::get().to(index))
            .route("/", web::get().to(index))
            .route("/health", web::get().to(health))
            .route("/open-chat/profiles", web::get().to(open_chat_profiles))
            .route(
                "/open-chat/profiles/share",
                web::post().to(share_open_profile),
            )
            .route(
                "/open-chat/profiles/share-member",
                web::post().to(share_member_open_profile),
            )
            .route("/open-chat/join", web::post().to(join_open_chat))
            .route("/rooms/{chat_id}/kick", web::post().to(kick_member))
            .route(
                "/rooms/{chat_id}/messages/{log_id}/hide",
                web::post().to(hide_message),
            )
            .route("/rooms/{chat_id}/leave", web::post().to(leave_chat))
            .route("/vox/status", web::get().to(vox_status))
            .route("/vox/voice-talk", web::post().to(start_voice_talk))
            .route("/vox/voice-rooms", web::post().to(create_voice_room))
            .route("/vox/voice-rooms/join", web::post().to(join_voice_room))
            .route("/vox/leave", web::post().to(leave_vox))
            .route("/vox/audio/start", web::post().to(start_vox_audio))
            .route("/vox/audio", web::post().to(push_vox_audio))
            .route("/vox/audio/stream", web::post().to(stream_vox_audio))
            .route("/vox/audio/stop", web::post().to(stop_vox_audio)),
    );
}

async fn index(req: HttpRequest, state: web::Data<AppState>) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(serde_json::json!({
        "ok": true,
        "service": "noa-iris-endpoints",
        "prefix": state.config.iris_hook.endpoint_prefix,
        "endpoints": [
            "GET /health",
            "GET /open-chat/profiles",
            "POST /open-chat/profiles/share",
            "POST /open-chat/profiles/share-member",
            "POST /open-chat/join",
            "POST /rooms/{chatId}/kick",
            "POST /rooms/{chatId}/messages/{logId}/hide",
            "POST /rooms/{chatId}/leave",
            "GET /vox/status",
            "POST /vox/voice-talk",
            "POST /vox/voice-rooms",
            "POST /vox/voice-rooms/join",
            "POST /vox/leave",
            "POST /vox/audio/start",
            "POST /vox/audio",
            "POST /vox/audio/stream",
            "POST /vox/audio/stop"
        ]
    })))
}

async fn open_chat_profiles(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(open_chat_profiles_action(&state).await?))
}

async fn share_open_profile(
    req: HttpRequest,
    body: web::Json<OpenProfileShareRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(
        share_open_profile_action(body.into_inner(), &state).await?,
    ))
}

async fn share_member_open_profile(
    req: HttpRequest,
    body: web::Json<OpenMemberProfileShareRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(
        share_member_open_profile_action(body.into_inner(), &state).await?,
    ))
}

async fn health(req: HttpRequest, state: web::Data<AppState>) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(serde_json::json!({
        "ok": true,
        "service": "noa",
        "version": env!("CARGO_PKG_VERSION"),
        "irisHookActive": crate::intercept::active()
    })))
}

async fn join_open_chat(
    req: HttpRequest,
    body: web::Json<OpenChatJoinRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(
        join_open_chat_action(body.into_inner(), &state).await?,
    ))
}

async fn kick_member(
    req: HttpRequest,
    chat_id: web::Path<String>,
    body: web::Json<KickMemberRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(
        kick_member_action(chat_id.into_inner(), body.into_inner(), &state).await?,
    ))
}

async fn hide_message(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    let (chat_id, log_id) = path.into_inner();
    Ok(web::Json(
        hide_message_action(chat_id, log_id, &state).await?,
    ))
}

async fn leave_chat(
    req: HttpRequest,
    chat_id: web::Path<String>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(
        leave_chat_action(chat_id.into_inner(), &state).await?,
    ))
}

async fn vox_status(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(vox::status_action(&state).await?))
}

async fn start_voice_talk(
    req: HttpRequest,
    body: web::Json<vox::VoiceTalkRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(
        vox::start_voice_talk_action(body.into_inner(), &state).await?,
    ))
}

async fn create_voice_room(
    req: HttpRequest,
    body: web::Json<vox::VoiceRoomRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(
        vox::create_voice_room_action(body.into_inner(), &state).await?,
    ))
}

async fn join_voice_room(
    req: HttpRequest,
    body: web::Json<vox::JoinVoiceRoomRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(
        vox::join_voice_room_action(body.into_inner(), &state).await?,
    ))
}

async fn leave_vox(
    req: HttpRequest,
    body: web::Json<vox::LeaveRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(
        vox::leave_action(body.into_inner(), &state).await?,
    ))
}

async fn start_vox_audio(
    req: HttpRequest,
    body: web::Json<vox_audio::AudioStartRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(
        vox_audio::start_action(body.into_inner(), &state).await?,
    ))
}

async fn push_vox_audio(
    req: HttpRequest,
    body: web::Bytes,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(vox_audio::push_action(body, &state).await?))
}

async fn stream_vox_audio(
    req: HttpRequest,
    query: web::Query<vox_audio::AudioStreamQuery>,
    payload: web::Payload,
    state: web::Data<AppState>,
) -> Result<HttpResponse, NoaError> {
    authorize_iris_hook(&req, &state)?;
    vox_audio::stream_action(query.into_inner(), payload, &state).await
}

async fn stop_vox_audio(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize_iris_hook(&req, &state)?;
    Ok(web::Json(vox_audio::stop_action(&state).await?))
}
