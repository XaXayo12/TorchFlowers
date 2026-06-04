use crate::{
    auth::entitlement::EntitlementProvisioner,
    bedrock::session::BedrockBotSession,
    config::Config,
    db::Database,
    error::{EngineError, EngineResult},
};

pub struct RealServerValidation {
    config: Config,
    db: Database,
}

impl RealServerValidation {
    pub fn new(config: Config, db: Database) -> Self {
        Self { config, db }
    }

    pub async fn run_from_env(&self) -> EngineResult<()> {
        let account_id = std::env::var("BEDROCK_VALIDATE_ACCOUNT_ID")
            .map_err(|_| EngineError::MissingConfig("BEDROCK_VALIDATE_ACCOUNT_ID"))?;
        let host = std::env::var("BEDROCK_VALIDATE_SERVER_HOST")
            .map_err(|_| EngineError::MissingConfig("BEDROCK_VALIDATE_SERVER_HOST"))?;
        let port = std::env::var("BEDROCK_VALIDATE_SERVER_PORT")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(19132);
        let duration = std::env::var("BEDROCK_VALIDATE_DURATION_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(300)
            .clamp(5, 900);
        let provisioned = EntitlementProvisioner::new(&self.config, self.db.clone())
            .provision(&account_id)
            .await?;
        let status = BedrockBotSession::new(self.db.clone())
            .validate_real_server_for(
                &account_id,
                None,
                &host,
                port,
                &provisioned,
                true,
                std::time::Duration::from_secs(duration),
            )
            .await?;
        println!("{}", serde_json::to_string_pretty(&status)?);
        if !status.success {
            return Err(EngineError::Bedrock(format!(
                "missing validation capabilities: {:?}",
                status.missing_capabilities
            )));
        }
        Ok(())
    }
}
