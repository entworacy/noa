use super::{AppState, authorize, parse_chat_id, parse_positive_i64};
use crate::failure::NoaError;
use actix_web::{HttpRequest, Responder, web};
use noa_agent_protocol::reaction::ReactionRequest;

pub(super) async fn endpoint(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    body: web::Json<ReactionRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    let (room, log) = path.into_inner();
    Ok(web::Json(
        action(room, log, body.into_inner(), &state).await?,
    ))
}

pub(super) async fn action(
    room: String,
    log: String,
    reaction: ReactionRequest,
    state: &AppState,
) -> Result<serde_json::Value, NoaError> {
    let room = parse_chat_id(&room)?;
    let log = parse_positive_i64("logId", &log)?;
    reaction.validate().map_err(NoaError::BadRequest)?;
    if !state.config.kakao_hook_enabled {
        return Err(NoaError::AndroidUnavailable(
            "메시지 리액션에는 KakaoTalk 후킹이 필요합니다".into(),
        ));
    }
    let catalog = state.catalog.clone().ok_or_else(|| {
        NoaError::Database("리액션 대상을 검증할 KakaoTalk 데이터베이스가 없습니다".into())
    })?;
    let link = tokio::task::spawn_blocking(move || catalog.reaction_target(room, log))
        .await
        .map_err(|e| NoaError::Internal(e.to_string()))??;
    crate::intercept::react_message(room, log, link, reaction).await?;
    Ok(
        serde_json::json!({"ok":true,"accepted":true,"verified":false,"verification":"native-callback","chatId":room.to_string(),"logId":log.to_string()}),
    )
}
