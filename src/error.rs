use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("http client error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("telegram error: {0}")]
    Telegram(String),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("not found: {0}")]
    NotFound(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::Parse(_) => StatusCode::BAD_REQUEST,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Config(_)
            | AppError::Redis(_)
            | AppError::Reqwest(_)
            | AppError::Telegram(_)
            | AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let body = match status {
            StatusCode::UNAUTHORIZED => "unauthorized",
            StatusCode::BAD_REQUEST => "bad request",
            StatusCode::NOT_FOUND => "not found",
            _ => "internal server error",
        };

        tracing::error!(error = %self, "request error");

        (status, body).into_response()
    }
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn into_response_uses_generic_body_for_internal_errors() {
        let response =
            AppError::Internal("database password leaked here".to_string()).into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(&body[..], b"internal server error");
    }

    #[tokio::test]
    async fn into_response_uses_generic_body_for_not_found() {
        let response = AppError::NotFound("secret incident id".to_string()).into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(&body[..], b"not found");
    }
}
