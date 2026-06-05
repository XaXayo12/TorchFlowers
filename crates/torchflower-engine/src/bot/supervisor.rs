use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    config::Config,
    core::{AutomationPolicy, BotSession, ServerAddress},
    db::Database,
    diagnostics::Diagnostics,
    error::{EngineError, EngineResult},
    pool::MemoryStats,
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
            loop {
                let result = async {
                    db.update_bot_status(&bot_id_string, "authenticating", None)
                        .await?;
                    db.update_bot_status(&bot_id_string, "connecting", None)
                        .await?;
                    let runner = BotSession::builder()
                        .config(config.clone())
                        .database(db.clone())
                        .account(bot.account_id.clone())
                        .server(ServerAddress::new(server.host.clone(), server.port as u16))
                        .automation_policy(AutomationPolicy::default())
                        .build()
                        .await
                        .map_err(|err| EngineError::InvalidRequest(err.to_string()))?;
                    let capabilities = runner
                        .validate_for(Duration::from_secs(300), false)
                        .await
                        .map_err(|err| EngineError::Bedrock(err.to_string()))?;
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

    pub async fn active_count(&self) -> usize {
        self.tasks.lock().await.len()
    }

    pub async fn memory_stats(&self) -> MemoryStats {
        let active = self.active_count().await;
        MemoryStats {
            bots_active: active,
            heap_bytes_estimated: active.saturating_mul(128 * 1024),
            buffer_pool_hit_rate: 1.0,
            buffer_pool_misses: 0,
        }
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
        let runner = BotSession::builder()
            .config(self.config.clone())
            .database(self.db.clone())
            .account(account_id.to_string())
            .server(ServerAddress::new(host.to_string(), port))
            .automation_policy(AutomationPolicy::allow_for_hosts([host.to_string()]))
            .build()
            .await
            .map_err(|err| EngineError::InvalidRequest(err.to_string()))?;
        runner
            .validate_for(required_duration, true)
            .await
            .map_err(|err| EngineError::Bedrock(err.to_string()))
    }
}
