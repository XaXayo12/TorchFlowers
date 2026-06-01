use crate::error::EngineError;

#[derive(Clone, Debug)]
pub struct Config {
    pub microsoft_client_id: String,
    pub token_encryption_secret: String,
    pub database_url: String,
    pub rust_engine_bind: String,
}

impl Config {
    pub fn from_env() -> Result<Self, EngineError> {
        Ok(Self {
            microsoft_client_id: required_env("MICROSOFT_CLIENT_ID")?,
            token_encryption_secret: required_env("TOKEN_ENCRYPTION_SECRET")?,
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://database/rustrock.sqlite".to_string()),
            rust_engine_bind: std::env::var("RUST_ENGINE_BIND")
                .unwrap_or_else(|_| "127.0.0.1:9080".to_string()),
        })
    }
}

fn required_env(name: &'static str) -> Result<String, EngineError> {
    let value = std::env::var(name).map_err(|_| EngineError::MissingConfig(name))?;
    if value.trim().is_empty() {
        return Err(EngineError::MissingConfig(name));
    }
    Ok(value)
}
