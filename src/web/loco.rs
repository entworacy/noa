use actix_web::{HttpRequest, Responder, web};
use noa_agent_protocol::loco::SendLocoRequest;

use super::{AppState, authorize};
use crate::failure::NoaError;

pub(super) fn configure(config: &mut web::ServiceConfig) {
    config.route("/api/loco/send", web::post().to(send));
}

async fn send(
    req: HttpRequest,
    body: web::Json<SendLocoRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    let request = body.into_inner();
    request.validate().map_err(NoaError::BadRequest)?;
    if !state.config.kakao_hook_enabled {
        return Err(NoaError::AndroidUnavailable(
            "LOCO 송신에는 KAKAO_HOOK_ENABLED=true가 필요합니다".into(),
        ));
    }
    Ok(web::Json(crate::intercept::send_loco(request).await?))
}
