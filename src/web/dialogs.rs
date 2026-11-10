use super::{AppState, authorize, authorize_iris_hook};
use crate::failure::NoaError;
use actix_web::{HttpRequest, Responder, web};
use noa_agent_protocol::dialog::{DialogCommand, DialogContent};
use serde_json::Value;

pub(super) fn configure(config: &mut web::ServiceConfig) {
    config
        .route("/api/dialogs/status", web::get().to(status))
        .route("/api/dialogs", web::post().to(show))
        .route("/api/dialogs/{dialog_id}", web::get().to(get))
        .route("/api/dialogs/{dialog_id}", web::delete().to(dismiss));
}

async fn action(command: DialogCommand, state: &AppState) -> Result<Value, NoaError> {
    command.validate().map_err(NoaError::BadRequest)?;
    if !state.config.kakao_hook_enabled {
        return Err(NoaError::AndroidUnavailable(
            "오버레이 다이얼로그에는 KakaoTalk 후킹이 필요합니다".into(),
        ));
    }
    interpret_result(crate::intercept::dialog_command(command).await?)
}

fn interpret_result(value: Value) -> Result<Value, NoaError> {
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(value);
    }
    let message = value
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("다이얼로그 처리 실패")
        .to_string();
    Err(match value.get("errorCode").and_then(Value::as_str) {
        Some("permission-denied") => NoaError::Forbidden(message),
        Some("not-found") => NoaError::NotFound(message),
        Some("dialog-active") => NoaError::Conflict(message),
        Some("bad-request") => NoaError::BadRequest(message),
        _ => NoaError::AndroidUnavailable(message),
    })
}

macro_rules! endpoints {
    ($show:ident, $get:ident, $dismiss:ident, $status:ident, $auth:ident) => {
        pub(super) async fn $show(
            req: HttpRequest,
            body: web::Json<DialogContent>,
            state: web::Data<AppState>,
        ) -> Result<impl Responder, NoaError> {
            $auth(&req, &state)?;
            let command = DialogCommand::Show {
                dialog_id: uuid::Uuid::new_v4().to_string(),
                content: body.into_inner(),
            };
            Ok(web::Json(action(command, &state).await?))
        }
        pub(super) async fn $get(
            req: HttpRequest,
            id: web::Path<String>,
            state: web::Data<AppState>,
        ) -> Result<impl Responder, NoaError> {
            $auth(&req, &state)?;
            Ok(web::Json(
                action(
                    DialogCommand::Get {
                        dialog_id: id.into_inner(),
                    },
                    &state,
                )
                .await?,
            ))
        }
        pub(super) async fn $dismiss(
            req: HttpRequest,
            id: web::Path<String>,
            state: web::Data<AppState>,
        ) -> Result<impl Responder, NoaError> {
            $auth(&req, &state)?;
            Ok(web::Json(
                action(
                    DialogCommand::Dismiss {
                        dialog_id: id.into_inner(),
                    },
                    &state,
                )
                .await?,
            ))
        }
        pub(super) async fn $status(
            req: HttpRequest,
            state: web::Data<AppState>,
        ) -> Result<impl Responder, NoaError> {
            $auth(&req, &state)?;
            Ok(web::Json(action(DialogCommand::Status, &state).await?))
        }
    };
}
endpoints!(show, get, dismiss, status, authorize);
endpoints!(
    iris_show,
    iris_get,
    iris_dismiss,
    iris_status,
    authorize_iris_hook
);

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::ResponseError;
    #[test]
    fn preserves_dialog_error_status_codes() {
        for (code, status) in [
            ("permission-denied", 403),
            ("not-found", 404),
            ("dialog-active", 409),
            ("bad-request", 400),
            ("display-unavailable", 503),
        ] {
            let error =
                interpret_result(serde_json::json!({"ok":false,"errorCode":code,"error":"reason"}))
                    .unwrap_err();
            assert_eq!(error.status_code().as_u16(), status);
            assert!(error.to_string().contains("reason"));
        }
        assert!(interpret_result(serde_json::json!({"ok":true,"state":"shown"})).is_ok());
        assert!(interpret_result(serde_json::json!({})).is_err());
    }
}
