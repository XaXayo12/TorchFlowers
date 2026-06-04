use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    auth::entitlement::EntitlementProvisioner,
    bedrock::session::BedrockBotSession,
    config::Config,
    db::Database,
    diagnostics::Diagnostics,
    error::{EngineError, EngineResult},
};

#[derive(Clone)]
pub struct BotSupervisor {
    db: Database,
    config: Config,
    tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

impl BotSupervisor {
    pub fn new(config: Config, db: Database) -> Self {
        Self {
            db,
            config,
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn start(&self, bot_id: &str) -> EngineResult<()> {
        if self.tasks.lock().await.contains_key(bot_id) {
            return Ok(());
        }
        let (bot, server) = self.db.get_bot_with_server(bot_id).await?;
        self.db.update_bot_status(bot_id, "starting", None).await?;
        let db = self.db.clone();
        let config = self.config.clone();
        let bot_id_string = bot_id.to_string();
        let handle = tokio::spawn(async move {
            let diagnostics = Diagnostics::new(db.clone());
            let provisioner = EntitlementProvisioner::new(&config, db.clone());
            let runner = BedrockBotSession::new(db.clone());
            loop {
                let result = async {
                    db.update_bot_status(&bot_id_string, "authenticating", None)
                        .await?;
                    let session = provisioner.provision(&bot.account_id).await?;
                    db.update_bot_status(&bot_id_string, "connecting", None)
                        .await?;
                    let capabilities = runner
                        .validate_real_server(
                            &bot.account_id,
                            Some(&bot_id_string),
                            &server.host,
                            server.port as u16,
                            &session,
                            false,
                        )
                        .await?;
                    db.update_bot_capabilities(
                        &bot_id_string,
                        &serde_json::to_value(&capabilities).map_err(EngineError::Json)?,
                    )
                    .await?;
                    db.mark_bot_joined(&bot_id_string).await?;
                    diagnostics
                        .log_event(
                            Some(&bot.account_id),
                            Some(&bot_id_string),
                            "info",
                            "bot",
                            Some("join"),
                            "bot joined and completed capability handshake",
                            serde_json::to_value(&capabilities).map_err(EngineError::Json)?,
                        )
                        .await?;
                    if bot.anti_afk_enabled {
                        diagnostics
                            .log_event(
                                Some(&bot.account_id),
                                Some(&bot_id_string),
                                "info",
                                "bot",
                                Some("anti_afk"),
                                "anti-AFK movement probe is enabled for this bot",
                                serde_json::json!({
                                    "strategy": "movement_probe",
                                    "interval_seconds": 30
                                }),
                            )
                            .await?;
                    }
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    Ok::<(), EngineError>(())
                }
                .await;

                if let Err(err) = result {
                    let _ = db
                        .mark_bot_left(&bot_id_string, "error", Some(&err.to_string()))
                        .await;
                    let _ = diagnostics
                        .log_event(
                            Some(&bot.account_id),
                            Some(&bot_id_string),
                            "error",
                            "bot",
                            Some("runtime"),
                            &err.to_string(),
                            serde_json::json!({}),
                        )
                        .await;
                    if !bot.reconnect_enabled {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
        });
        self.tasks.lock().await.insert(bot_id.to_string(), handle);
        Ok(())
    }

    pub async fn stop(&self, bot_id: &str) -> EngineResult<()> {
        if let Some(handle) = self.tasks.lock().await.remove(bot_id) {
            handle.abort();
        }
        self.db.mark_bot_left(bot_id, "stopped", None).await?;
        Ok(())
    }

    pub async fn validate_once(
        &self,
        account_id: &str,
        host: &str,
        port: u16,
    ) -> EngineResult<crate::models::CapabilityStatus> {
        self.validate_once_for(account_id, host, port, Duration::from_secs(300))
            .await
    }

    pub async fn validate_once_for(
        &self,
        account_id: &str,
        host: &str,
        port: u16,
        required_duration: Duration,
    ) -> EngineResult<crate::models::CapabilityStatus> {
        let provisioner = EntitlementProvisioner::new(&self.config, self.db.clone());
        let session = provisioner.provision(account_id).await?;
        BedrockBotSession::new(self.db.clone())
            .validate_real_server_for(
                account_id,
                None,
                host,
                port,
                &session,
                true,
                required_duration,
            )
            .await
    }
}
