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
        let status = self.status_code();
        let body = Json(json!({
            "error": {
                "code": self.code(),
                "message": self.safe_message(),
                "request_id": null
            }
        }));
        (status, body).into_response()
    }
}

impl EngineError {
    pub fn status_code(&self) -> StatusCode {
        match self {
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
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            EngineError::MissingConfig(_) => "missing_config",
            EngineError::Database(_) => "database_error",
            EngineError::Http(_) => "http_error",
            EngineError::Json(_) => "json_error",
            EngineError::Io(_) => "io_error",
            EngineError::Crypto(_) => "crypto_error",
            EngineError::Auth { .. } => "auth_error",
            EngineError::Bedrock(_) => "bedrock_error",
            EngineError::NotFound(_) => "not_found",
            EngineError::InvalidRequest(_) => "invalid_request",
        }
    }

    pub fn safe_message(&self) -> String {
        match self {
            EngineError::MissingConfig(name) => format!("missing required configuration: {name}"),
            EngineError::NotFound(_) => "resource not found".to_string(),
            EngineError::InvalidRequest(message) => message.clone(),
            EngineError::Auth { step, .. } => format!("authentication failed during {step}"),
            EngineError::Database(_)
            | EngineError::Http(_)
            | EngineError::Json(_)
            | EngineError::Io(_)
            | EngineError::Crypto(_)
            | EngineError::Bedrock(_) => "internal server error".to_string(),
        }
    }
}

pub type EngineResult<T> = Result<T, EngineError>;
