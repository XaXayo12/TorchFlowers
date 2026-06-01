use axum::{Json, http::StatusCode, response::IntoResponse};
use serde_json::json;

#[derive(thiserror::Error, Debug)]
pub enum EngineError {
    #[error("missing required configuration: {0}")]
    MissingConfig(&'static str),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("authentication failed at {step}: {message}")]
    Auth { step: &'static str, message: String },
    #[error("bedrock networking error: {0}")]
    Bedrock(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

impl IntoResponse for EngineError {
    fn into_response(self) -> axum::response::Response {
        let status = match self {
            EngineError::MissingConfig(_) => StatusCode::INTERNAL_SERVER_ERROR,
            EngineError::NotFound(_) => StatusCode::NOT_FOUND,
            EngineError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            EngineError::Auth { .. } => StatusCode::BAD_GATEWAY,
            EngineError::Database(_)
            | EngineError::Http(_)
            | EngineError::Json(_)
            | EngineError::Io(_)
            | EngineError::Crypto(_)
            | EngineError::Bedrock(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(json!({ "error": self.to_string() }));
        (status, body).into_response()
    }
}

pub type EngineResult<T> = Result<T, EngineError>;
