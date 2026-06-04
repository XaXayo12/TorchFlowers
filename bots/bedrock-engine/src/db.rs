use std::{path::Path, str::FromStr};

use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tokio::fs;
use uuid::Uuid;

use crate::{
    error::{EngineError, EngineResult},
    models::{Account, Bot, DeviceAuthSession, Entitlement, LogEntry, Server},
};

#[derive(Debug, Clone)]
pub struct EntitlementRetryState {
    pub retry_count: i64,
    pub next_retry_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn connect(database_url: &str) -> EngineResult<Self> {
        if let Some(path) = sqlite_file_path(database_url) {
            if let Some(parent) = Path::new(path).parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent).await?;
                }
            }
        }
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn migrate(&self) -> EngineResult<()> {
        let sql = include_str!("../../../database/migrations/0001_initial.sql");
        for statement in sql.split(';') {
            let statement = statement.trim();
            if !statement.is_empty() {
                sqlx::query(statement).execute(&self.pool).await?;
            }
        }
        Ok(())
    }

    pub async fn upsert_account_email(&self, email: &str) -> EngineResult<String> {
        if let Some(row) = sqlx::query(
            "SELECT id FROM accounts
             WHERE lower(email) = lower(?1)
             ORDER BY
               CASE microsoft_status
                 WHEN 'authenticated' THEN 0
                 WHEN 'device_code_pending' THEN 1
                 ELSE 2
               END,
               created_at DESC
             LIMIT 1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?
        {
            return Ok(row.try_get("id")?);
        }

        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO accounts (id, email, microsoft_status) VALUES (?1, ?2, 'device_code_pending')",
        )
        .bind(&id)
        .bind(email)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn list_accounts(&self) -> EngineResult<Vec<Account>> {
        let rows = sqlx::query(
            "SELECT id,email,gamertag,xuid,microsoft_status,xbox_status,xsts_status,playfab_status,entitlement_status,bedrock_auth_status,bot_status,last_error,created_at,updated_at FROM accounts ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(account_from_row).collect()
    }

    pub async fn get_account(&self, account_id: &str) -> EngineResult<Account> {
        let row = sqlx::query(
            "SELECT id,email,gamertag,xuid,microsoft_status,xbox_status,xsts_status,playfab_status,entitlement_status,bedrock_auth_status,bot_status,last_error,created_at,updated_at FROM accounts WHERE id = ?1",
        )
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| EngineError::NotFound(format!("account {account_id}")))?;
        account_from_row(row)
    }

    pub async fn update_account_status(
        &self,
        account_id: &str,
        column: &str,
        status: &str,
        last_error: Option<&str>,
    ) -> EngineResult<()> {
        let allowed = [
            "microsoft_status",
            "xbox_status",
            "xsts_status",
            "playfab_status",
            "entitlement_status",
            "bedrock_auth_status",
            "bot_status",
        ];
        if !allowed.contains(&column) {
            return Err(EngineError::InvalidRequest(format!(
                "invalid account status column {column}"
            )));
        }
        let query = format!(
            "UPDATE accounts SET {column} = ?1, last_error = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?3"
        );
        sqlx::query(&query)
            .bind(status)
            .bind(last_error)
            .bind(account_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_account_profile(
        &self,
        account_id: &str,
        gamertag: Option<&str>,
        xuid: Option<&str>,
    ) -> EngineResult<()> {
        sqlx::query(
            "UPDATE accounts SET gamertag = ?1, xuid = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?3",
        )
        .bind(gamertag)
        .bind(xuid)
        .bind(account_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn save_tokens(
        &self,
        account_id: &str,
        access_token_ciphertext: &str,
        refresh_token_ciphertext: &str,
        expires_at: &str,
    ) -> EngineResult<()> {
        sqlx::query(
            "UPDATE accounts SET access_token_ciphertext=?1, refresh_token_ciphertext=?2, access_token_expires_at=?3, microsoft_status='authenticated', updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id=?4",
        )
        .bind(access_token_ciphertext)
        .bind(refresh_token_ciphertext)
        .bind(expires_at)
        .bind(account_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_auth_session(
        &self,
        account_id: &str,
        device_code: &str,
        user_code: &str,
        verification_uri: &str,
        expires_at: &str,
        interval_seconds: i64,
    ) -> EngineResult<String> {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO auth_sessions (id, account_id, device_code, user_code, verification_uri, expires_at, interval_seconds)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(&id)
        .bind(account_id)
        .bind(device_code)
        .bind(user_code)
        .bind(verification_uri)
        .bind(expires_at)
        .bind(interval_seconds)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn get_auth_session_secret(
        &self,
        session_id: &str,
    ) -> EngineResult<(DeviceAuthSession, String)> {
        let row = sqlx::query(
            "SELECT id,account_id,device_code,user_code,verification_uri,expires_at,interval_seconds,status,last_error FROM auth_sessions WHERE id=?1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| EngineError::NotFound(format!("auth session {session_id}")))?;
        let expires_raw: String = row.try_get("expires_at")?;
        let expires_at = chrono::DateTime::parse_from_rfc3339(&expires_raw)
            .map_err(|err| EngineError::InvalidRequest(err.to_string()))?
            .with_timezone(&chrono::Utc);
        Ok((
            DeviceAuthSession {
                id: row.try_get("id")?,
                account_id: row.try_get("account_id")?,
                user_code: row.try_get("user_code")?,
                verification_uri: row.try_get("verification_uri")?,
                expires_at,
                interval_seconds: row.try_get::<i64, _>("interval_seconds")? as u64,
                status: row.try_get("status")?,
            },
            row.try_get("device_code")?,
        ))
    }

    pub async fn update_auth_session_status(
        &self,
        session_id: &str,
        status: &str,
        last_error: Option<&str>,
    ) -> EngineResult<()> {
        sqlx::query(
            "UPDATE auth_sessions SET status=?1,last_error=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id=?3",
        )
        .bind(status)
        .bind(last_error)
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn token_ciphertexts(
        &self,
        account_id: &str,
    ) -> EngineResult<(String, String, Option<String>)> {
        let row = sqlx::query("SELECT access_token_ciphertext, refresh_token_ciphertext, access_token_expires_at FROM accounts WHERE id=?1")
            .bind(account_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("account {account_id}")))?;
        let access: Option<String> = row.try_get("access_token_ciphertext")?;
        let refresh: Option<String> = row.try_get("refresh_token_ciphertext")?;
        Ok((
            access.ok_or_else(|| {
                EngineError::InvalidRequest("account has no access token".to_string())
            })?,
            refresh.ok_or_else(|| {
                EngineError::InvalidRequest("account has no refresh token".to_string())
            })?,
            row.try_get("access_token_expires_at")?,
        ))
    }

    pub async fn upsert_entitlement(
        &self,
        account_id: &str,
        has_entitlement: bool,
        playfab_id: Option<&str>,
        session_ticket_ciphertext: Option<&str>,
        minecraft_token_ciphertext: Option<&str>,
        status: &str,
        request_id: Option<&str>,
        last_error: Option<&str>,
    ) -> EngineResult<()> {
        sqlx::query(
            "INSERT INTO entitlements (account_id, has_entitlement, playfab_id, session_ticket_ciphertext, minecraft_token_ciphertext, provisioning_status, retry_count, next_retry_at, last_request_id, last_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, NULL, ?7, ?8)
             ON CONFLICT(account_id) DO UPDATE SET has_entitlement=excluded.has_entitlement, playfab_id=excluded.playfab_id, session_ticket_ciphertext=COALESCE(excluded.session_ticket_ciphertext, entitlements.session_ticket_ciphertext), minecraft_token_ciphertext=COALESCE(excluded.minecraft_token_ciphertext, entitlements.minecraft_token_ciphertext), provisioning_status=excluded.provisioning_status, retry_count=0, next_retry_at=NULL, last_request_id=excluded.last_request_id, last_error=excluded.last_error, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(account_id)
        .bind(if has_entitlement { 1 } else { 0 })
        .bind(playfab_id)
        .bind(session_ticket_ciphertext)
        .bind(minecraft_token_ciphertext)
        .bind(status)
        .bind(request_id)
        .bind(last_error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn begin_entitlement_provisioning(&self, account_id: &str) -> EngineResult<()> {
        sqlx::query(
            "INSERT INTO entitlements (account_id, provisioning_status, next_retry_at, last_error)
             VALUES (?1, 'in_progress', NULL, NULL)
             ON CONFLICT(account_id) DO UPDATE SET provisioning_status='in_progress', next_retry_at=NULL, last_error=NULL, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(account_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_entitlement_provisioning_failure(
        &self,
        account_id: &str,
        last_error: &str,
    ) -> EngineResult<EntitlementRetryState> {
        let current = sqlx::query("SELECT retry_count FROM entitlements WHERE account_id=?1")
            .bind(account_id)
            .fetch_optional(&self.pool)
            .await?;
        let current_retry_count = match current {
            Some(row) => row.try_get::<i64, _>("retry_count")?,
            None => 0,
        };
        let retry_count = current_retry_count + 1;
        let backoff_seconds = provisioning_backoff_seconds(retry_count);
        let next_retry_at =
            sqlx::query_scalar::<_, String>("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)")
                .bind(format!("+{backoff_seconds} seconds"))
                .fetch_one(&self.pool)
                .await?;
        sqlx::query(
            "INSERT INTO entitlements (account_id, provisioning_status, retry_count, next_retry_at, last_error)
             VALUES (?1, 'failed', ?2, ?3, ?4)
             ON CONFLICT(account_id) DO UPDATE SET provisioning_status='failed', retry_count=?2, next_retry_at=?3, last_error=?4, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(account_id)
        .bind(retry_count)
        .bind(&next_retry_at)
        .bind(last_error)
        .execute(&self.pool)
        .await?;
        Ok(EntitlementRetryState {
            retry_count,
            next_retry_at: Some(next_retry_at),
            last_error: Some(last_error.to_string()),
        })
    }

    pub async fn entitlement_retry_state(
        &self,
        account_id: &str,
    ) -> EngineResult<EntitlementRetryState> {
        let row = sqlx::query(
            "SELECT retry_count,next_retry_at,last_error FROM entitlements WHERE account_id=?1",
        )
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| EngineError::NotFound(format!("entitlement {account_id}")))?;
        Ok(EntitlementRetryState {
            retry_count: row.try_get("retry_count")?,
            next_retry_at: row.try_get("next_retry_at")?,
            last_error: row.try_get("last_error")?,
        })
    }

    pub async fn list_entitlements(&self) -> EngineResult<Vec<Entitlement>> {
        let rows = sqlx::query(
            "SELECT e.account_id,a.email AS account_email,e.has_entitlement,e.playfab_id,e.provisioning_status,e.retry_count,e.next_retry_at,e.last_request_id,e.last_error,e.created_at,e.updated_at
             FROM entitlements e
             JOIN accounts a ON a.id=e.account_id
             ORDER BY e.updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(Entitlement {
                    account_id: row.try_get("account_id")?,
                    account_email: row.try_get("account_email")?,
                    has_entitlement: row.try_get::<i64, _>("has_entitlement")? == 1,
                    playfab_id: row.try_get("playfab_id")?,
                    provisioning_status: row.try_get("provisioning_status")?,
                    retry_count: row.try_get("retry_count")?,
                    next_retry_at: row.try_get("next_retry_at")?,
                    last_request_id: row.try_get("last_request_id")?,
                    last_error: row.try_get("last_error")?,
                    created_at: row.try_get("created_at")?,
                    updated_at: row.try_get("updated_at")?,
                })
            })
            .collect()
    }

    pub async fn create_server(&self, name: &str, host: &str, port: i64) -> EngineResult<String> {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO servers (id, name, host, port) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(host, port) DO UPDATE SET name=excluded.name, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(&id)
        .bind(name)
        .bind(host)
        .bind(port)
        .execute(&self.pool)
        .await?;
        let existing = sqlx::query("SELECT id FROM servers WHERE host=?1 AND port=?2")
            .bind(host)
            .bind(port)
            .fetch_one(&self.pool)
            .await?;
        Ok(existing.try_get("id")?)
    }

    pub async fn list_servers(&self) -> EngineResult<Vec<Server>> {
        let rows = sqlx::query(
            "SELECT id,name,host,port,protocol_version,enabled FROM servers ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(Server {
                    id: row.try_get("id")?,
                    name: row.try_get("name")?,
                    host: row.try_get("host")?,
                    port: row.try_get("port")?,
                    protocol_version: row.try_get("protocol_version")?,
                    enabled: row.try_get::<i64, _>("enabled")? == 1,
                })
            })
            .collect()
    }

    pub async fn get_server(&self, server_id: &str) -> EngineResult<Server> {
        let row = sqlx::query(
            "SELECT id,name,host,port,protocol_version,enabled FROM servers WHERE id=?1",
        )
        .bind(server_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| EngineError::NotFound(format!("server {server_id}")))?;
        Ok(Server {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            host: row.try_get("host")?,
            port: row.try_get("port")?,
            protocol_version: row.try_get("protocol_version")?,
            enabled: row.try_get::<i64, _>("enabled")? == 1,
        })
    }

    pub async fn create_bot(&self, account_id: &str, server_id: &str) -> EngineResult<String> {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO bots (id, account_id, server_id) VALUES (?1, ?2, ?3)
             ON CONFLICT(account_id, server_id) DO UPDATE SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(&id)
        .bind(account_id)
        .bind(server_id)
        .execute(&self.pool)
        .await?;
        let row = sqlx::query("SELECT id FROM bots WHERE account_id=?1 AND server_id=?2")
            .bind(account_id)
            .bind(server_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.try_get("id")?)
    }

    pub async fn get_bot_with_server(&self, bot_id: &str) -> EngineResult<(Bot, Server)> {
        let row = sqlx::query(
            "SELECT b.id,b.account_id,b.server_id,b.status,b.reconnect_enabled,b.anti_afk_enabled,b.current_position,b.inventory_json,b.capabilities_json,b.last_error,
                    s.id AS s_id,s.name,s.host,s.port,s.protocol_version,s.enabled
             FROM bots b JOIN servers s ON s.id=b.server_id WHERE b.id=?1",
        )
        .bind(bot_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| EngineError::NotFound(format!("bot {bot_id}")))?;
        let bot = Bot {
            id: row.try_get("id")?,
            account_id: row.try_get("account_id")?,
            server_id: row.try_get("server_id")?,
            status: row.try_get("status")?,
            reconnect_enabled: row.try_get::<i64, _>("reconnect_enabled")? == 1,
            anti_afk_enabled: row.try_get::<i64, _>("anti_afk_enabled")? == 1,
            current_position: row.try_get("current_position")?,
            inventory_json: parse_json(row.try_get::<String, _>("inventory_json")?),
            capabilities_json: parse_json(row.try_get::<String, _>("capabilities_json")?),
            last_error: row.try_get("last_error")?,
        };
        let server = Server {
            id: row.try_get("s_id")?,
            name: row.try_get("name")?,
            host: row.try_get("host")?,
            port: row.try_get("port")?,
            protocol_version: row.try_get("protocol_version")?,
            enabled: row.try_get::<i64, _>("enabled")? == 1,
        };
        Ok((bot, server))
    }

    pub async fn list_bots(&self) -> EngineResult<Vec<Bot>> {
        let rows = sqlx::query("SELECT id,account_id,server_id,status,reconnect_enabled,anti_afk_enabled,current_position,inventory_json,capabilities_json,last_error FROM bots ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                Ok(Bot {
                    id: row.try_get("id")?,
                    account_id: row.try_get("account_id")?,
                    server_id: row.try_get("server_id")?,
                    status: row.try_get("status")?,
                    reconnect_enabled: row.try_get::<i64, _>("reconnect_enabled")? == 1,
                    anti_afk_enabled: row.try_get::<i64, _>("anti_afk_enabled")? == 1,
                    current_position: row.try_get("current_position")?,
                    inventory_json: parse_json(row.try_get::<String, _>("inventory_json")?),
                    capabilities_json: parse_json(row.try_get::<String, _>("capabilities_json")?),
                    last_error: row.try_get("last_error")?,
                })
            })
            .collect()
    }

    pub async fn update_bot_status(
        &self,
        bot_id: &str,
        status: &str,
        last_error: Option<&str>,
    ) -> EngineResult<()> {
        sqlx::query("UPDATE bots SET status=?1,last_error=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id=?3")
            .bind(status)
            .bind(last_error)
            .bind(bot_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn mark_bot_joined(&self, bot_id: &str) -> EngineResult<()> {
        sqlx::query(
            "UPDATE bots SET status='connected', last_error=NULL, last_join_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id=?1",
        )
        .bind(bot_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_bot_left(
        &self,
        bot_id: &str,
        status: &str,
        last_error: Option<&str>,
    ) -> EngineResult<()> {
        sqlx::query(
            "UPDATE bots SET status=?1, last_error=?2, last_leave_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id=?3",
        )
        .bind(status)
        .bind(last_error)
        .bind(bot_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_bot_runtime_state(
        &self,
        bot_id: &str,
        current_position: Option<&str>,
        inventory_json: Option<&serde_json::Value>,
    ) -> EngineResult<()> {
        sqlx::query(
            "UPDATE bots SET current_position=COALESCE(?1, current_position), inventory_json=COALESCE(?2, inventory_json), updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id=?3",
        )
        .bind(current_position)
        .bind(inventory_json.map(serde_json::Value::to_string))
        .bind(bot_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_bot_capabilities(
        &self,
        bot_id: &str,
        capabilities: &serde_json::Value,
    ) -> EngineResult<()> {
        sqlx::query("UPDATE bots SET capabilities_json=?1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id=?2")
            .bind(capabilities.to_string())
            .bind(bot_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn log(&self, entry: NewLogEntry<'_>) -> EngineResult<()> {
        sqlx::query(
            "INSERT INTO logs (id,account_id,bot_id,level,category,step,request_id,method,url,status_code,request_body,response_body,message,metadata_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(entry.account_id)
        .bind(entry.bot_id)
        .bind(entry.level)
        .bind(entry.category)
        .bind(entry.step)
        .bind(entry.request_id)
        .bind(entry.method)
        .bind(entry.url)
        .bind(entry.status_code)
        .bind(entry.request_body)
        .bind(entry.response_body)
        .bind(entry.message)
        .bind(entry.metadata_json.unwrap_or("{}"))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_logs(&self, limit: i64) -> EngineResult<Vec<LogEntry>> {
        let rows = sqlx::query(
            "SELECT id,account_id,bot_id,level,category,step,request_id,method,url,status_code,request_body,response_body,message,metadata_json,created_at FROM logs ORDER BY created_at DESC LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(LogEntry {
                    id: row.try_get("id")?,
                    account_id: row.try_get("account_id")?,
                    bot_id: row.try_get("bot_id")?,
                    level: row.try_get("level")?,
                    category: row.try_get("category")?,
                    step: row.try_get("step")?,
                    request_id: row.try_get("request_id")?,
                    method: row.try_get("method")?,
                    url: row.try_get("url")?,
                    status_code: row.try_get("status_code")?,
                    request_body: row.try_get("request_body")?,
                    response_body: row.try_get("response_body")?,
                    message: row.try_get("message")?,
                    metadata_json: parse_json(row.try_get::<String, _>("metadata_json")?),
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }
}

fn sqlite_file_path(database_url: &str) -> Option<&str> {
    let path = database_url
        .strip_prefix("sqlite://")
        .or_else(|| database_url.strip_prefix("sqlite:"))?;
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    if path.is_empty() || path == ":memory:" {
        None
    } else {
        Some(path)
    }
}

fn provisioning_backoff_seconds(retry_count: i64) -> i64 {
    let exponent = retry_count.saturating_sub(1).min(7) as u32;
    (30_i64.saturating_mul(2_i64.saturating_pow(exponent))).min(3600)
}

pub struct NewLogEntry<'a> {
    pub account_id: Option<&'a str>,
    pub bot_id: Option<&'a str>,
    pub level: &'a str,
    pub category: &'a str,
    pub step: Option<&'a str>,
    pub request_id: Option<&'a str>,
    pub method: Option<&'a str>,
    pub url: Option<&'a str>,
    pub status_code: Option<i64>,
    pub request_body: Option<&'a str>,
    pub response_body: Option<&'a str>,
    pub message: &'a str,
    pub metadata_json: Option<&'a str>,
}

fn account_from_row(row: sqlx::sqlite::SqliteRow) -> EngineResult<Account> {
    Ok(Account {
        id: row.try_get("id")?,
        email: row.try_get("email")?,
        gamertag: row.try_get("gamertag")?,
        xuid: row.try_get("xuid")?,
        microsoft_status: row.try_get("microsoft_status")?,
        xbox_status: row.try_get("xbox_status")?,
        xsts_status: row.try_get("xsts_status")?,
        playfab_status: row.try_get("playfab_status")?,
        entitlement_status: row.try_get("entitlement_status")?,
        bedrock_auth_status: row.try_get("bedrock_auth_status")?,
        bot_status: row.try_get("bot_status")?,
        last_error: row.try_get("last_error")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn parse_json(value: String) -> serde_json::Value {
    serde_json::from_str(&value).unwrap_or_else(|_| serde_json::json!({}))
}
