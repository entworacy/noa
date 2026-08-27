use actix_web::{HttpResponse, ResponseError, http::StatusCode};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum NoaError {
    #[error("{0}")]
    BadRequest(String),
    #[error("인증이 필요합니다")]
    Unauthorized,
    #[error("{0}")]
    NotFound(String),
    #[error("Android 전송 기능을 사용할 수 없습니다: {0}")]
    AndroidUnavailable(String),
    #[error("{0}")]
    Database(String),
    #[error("{0}")]
    Internal(String),
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
}

impl ResponseError for NoaError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::AndroidUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Database(_) | Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).json(ErrorBody {
            error: &self.to_string(),
        })
    }
}

impl From<std::io::Error> for NoaError {
    fn from(value: std::io::Error) -> Self {
        Self::Internal(value.to_string())
    }
}
