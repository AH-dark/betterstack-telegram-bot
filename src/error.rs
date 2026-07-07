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

        tracing::error!(error = %self, "request error");

        (status, self.to_string()).into_response()
    }
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;
