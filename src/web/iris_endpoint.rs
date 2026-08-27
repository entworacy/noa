use actix_web::{HttpRequest, Responder, web};

use super::{
    AppState, KickMemberRequest, OpenChatJoinRequest, OpenMemberProfileShareRequest,
    OpenProfileShareRequest, authorize_iris_hook, join_open_chat_action, kick_member_action,
    leave_chat_action, open_chat_profiles_action, share_member_open_profile_action,
    share_open_profile_action,
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
            .route("/rooms/{chat_id}/leave", web::post().to(leave_chat)),
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
            "POST /rooms/{chatId}/leave"
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
