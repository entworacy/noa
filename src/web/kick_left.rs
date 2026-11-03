use actix_web::{HttpRequest, Responder, web};
use serde::Deserialize;

use super::{AppState, authorize, parse_chat_id, parse_kick_user_id};
use crate::failure::NoaError;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct KickLeftRequest {
    user_id: serde_json::Value,
}

pub(super) async fn endpoint(
    req: HttpRequest,
    chat_id: web::Path<String>,
    body: web::Json<KickLeftRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    Ok(web::Json(
        action(chat_id.into_inner(), body.into_inner(), &state).await?,
    ))
}

pub(super) async fn action(
    chat_id: String,
    body: KickLeftRequest,
    state: &AppState,
) -> Result<serde_json::Value, NoaError> {
    let chat_id = parse_chat_id(&chat_id)?;
    let user = parse_kick_user_id(Some(body.user_id))?
        .ok_or_else(|| NoaError::BadRequest("userId가 필요합니다".to_string()))?;
    let user_id = user
        .parse::<i64>()
        .map_err(|_| NoaError::BadRequest("올바르지 않은 userId".to_string()))?;
    if !state.config.kakao_hook_enabled {
        return Err(NoaError::AndroidUnavailable(
            "퇴장 사용자 강퇴에는 KakaoTalk 후킹이 필요합니다".to_string(),
        ));
    }
    let catalog = state.catalog.clone().ok_or_else(|| {
        NoaError::Database("강퇴 대상을 검증할 KakaoTalk 데이터베이스가 없습니다".to_string())
    })?;
    tokio::task::spawn_blocking(move || catalog.validate_left_kick_target(chat_id, user_id))
        .await
        .map_err(|error| NoaError::Internal(format!("강퇴 대상 조회 작업 실패: {error}")))??;

    crate::intercept::kick_left_member(chat_id, user_id).await?;
    Ok(serde_json::json!({
        "ok": true,
        "verified": true,
        "verification": "loco-response",
        "serverConfirmed": true,
        "chatId": chat_id.to_string(),
        "userId": user,
        "message": "퇴장 사용자 강퇴를 완료했습니다"
    }))
}
