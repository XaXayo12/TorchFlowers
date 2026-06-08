use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::runtime::Builder;
use tracing::{info, warn};

use torchflower_engine::{
    auth::ProvisionedBedrockSession,
    bedrock::{
        protocol_adapter::{BedrockProtocolAdapter, BedrockProtocolOptions, ObservedInventoryItem},
        session::{
            create_offline_session, BedrockBotSession, InstantScript, InstantScriptEvent,
            MenuClickMethod, MenuClickTarget,
        },
    },
};

#[derive(Debug, Parser)]
#[command(name = "torchflower-lite-bot")]
#[command(about = "Low-resource Minecraft Bedrock bot runner")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start bots using a config file.
    Run {
        #[arg(short, long, default_value = "bots.toml")]
        config: PathBuf,
    },

    /// Run a lightweight local benchmark.
    Bench {
        #[arg(short, long, default_value_t = 10)]
        bots: usize,

        #[arg(short, long, default_value = "60s")]
        duration: String,
    },

    /// Create a default bots.toml config file.
    Init {
        #[arg(short, long, default_value = "bots.toml")]
        output: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuthConfig {
    cache_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AccountConfig {
    id: String,
    #[serde(rename = "type")]
    account_type: Option<String>,
    token_cache: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LiteConfig {
    server: ServerConfig,
    runtime: RuntimeConfig,
    auth: Option<AuthConfig>,
    accounts: Option<Vec<AccountConfig>>,
    bots: Vec<BotConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
    protocol_version: Option<i32>,
    protocol_versions: Option<Vec<i32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeConfig {
    log_level: Option<String>,
    duration_secs: Option<u64>,
    reconnect: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BotConfig {
    username: Option<String>,
    account_id: Option<String>,
    auth_mode: Option<String>,
    mode: Option<String>,
    reset_on_spawn: Option<bool>,
    script: Option<Vec<ScriptStep>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScriptStep {
    trigger: String,
    action: String,
    command: Option<String>,
    slot: Option<u32>,
}

impl LiteConfig {
    fn validate(&self) -> Result<()> {
        if self.server.host.trim().is_empty() {
            bail!("server.host cannot be empty");
        }

        if self.server.port == 0 {
            bail!("server.port cannot be 0");
        }

        if let Some(version) = self.server.protocol_version {
            if version <= 0 {
                bail!("server.protocol_version must be a positive integer");
            }
        }

        if let Some(versions) = &self.server.protocol_versions {
            if versions.is_empty() {
                bail!("server.protocol_versions cannot be empty");
            }
            for version in versions {
                if *version <= 0 {
                    bail!("server.protocol_versions entries must be positive integers");
                }
            }
        }

        if self.bots.is_empty() {
            bail!("at least one [[bots]] entry is required");
        }

        for (index, bot) in self.bots.iter().enumerate() {
            let has_username = bot
                .username
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());

            let has_account_id = bot
                .account_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());

            if !has_username && !has_account_id {
                bail!(
                    "bot entry #{index} must include either username or account_id",
                    index = index + 1
                );
            }
        }

        Ok(())
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init { output } => init_config(&output),
        Command::Run { config } => {
            init_logging("warn");

            let rt = Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
                .context("failed to build current-thread Tokio runtime")?;

            rt.block_on(run_config(config))
        }
        Command::Bench { bots, duration } => {
            init_logging("warn");

            let rt = Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
                .context("failed to build current-thread Tokio runtime")?;

            rt.block_on(run_bench(bots, &duration))
        }
    }
}

fn init_logging(default_level: &str) {
    #[cfg(feature = "logging")]
    {
        use tracing_subscriber::{fmt, EnvFilter};

        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

        let _ = fmt().with_env_filter(filter).try_init();
    }

    #[cfg(not(feature = "logging"))]
    {
        let _ = default_level;
    }
}

fn init_config(output: &Path) -> Result<()> {
    if output.exists() {
        bail!("{} already exists", output.display());
    }

    let config = default_config_text();
    fs::write(output, config).with_context(|| format!("failed to write {}", output.display()))?;

    println!("Created {}", output.display());
    println!("Edit it, then run:");
    println!("  torchflower-lite-bot run --config {}", output.display());

    Ok(())
}

async fn run_config(path: PathBuf) -> Result<()> {
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;

    let config: LiteConfig =
        toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))?;

    config.validate()?;

    let log_level = config
        .runtime
        .log_level
        .clone()
        .unwrap_or_else(|| "warn".to_string());

    info!("lite bot config loaded from {}", path.display());
    info!("requested log level: {}", log_level);
    info!(
        "server target: {}:{}",
        config.server.host, config.server.port
    );

    let protocol_attempts = resolve_protocol_attempts(&config.server)?;
    for (attempt_index, protocol_options) in protocol_attempts.iter().enumerate() {
        warn!(
            "selected Bedrock protocol attempt #{attempt}: protocol_version={requested} codec_protocol_version={codec} source={source} codec_exact_match={exact}",
            attempt = attempt_index + 1,
            requested = protocol_options.requested_protocol_version,
            codec = protocol_options.codec_protocol_version_number(),
            source = protocol_options.source,
            exact = protocol_options.codec_exact_match()
        );
    }

    let duration_secs = config.runtime.duration_secs.unwrap_or(0);
    let reconnect = config.runtime.reconnect.unwrap_or(true);
    let config_dir = path.parent().map(|p| p.to_path_buf());

    let mut handles = Vec::with_capacity(config.bots.len());

    for (index, bot) in config.bots.clone().into_iter().enumerate() {
        let server = config.server.clone();
        let protocol_attempts = protocol_attempts.clone();
        let config_dir_clone = config_dir.clone();
        let lite_config_clone = config.clone();
        handles.push(tokio::spawn(async move {
            run_single_bot(
                index,
                server,
                protocol_attempts,
                bot,
                duration_secs,
                reconnect,
                config_dir_clone,
                lite_config_clone,
            )
            .await
        }));
    }

    for handle in handles {
        handle.await.context("bot task panicked")??;
    }

    Ok(())
}

fn resolve_protocol_attempts(server: &ServerConfig) -> Result<Vec<BedrockProtocolOptions>> {
    if let Some(version) = server.protocol_version {
        return Ok(vec![
            BedrockProtocolOptions::from_config(version).map_err(|err| anyhow::anyhow!(err))?
        ]);
    }

    if let Some(versions) = &server.protocol_versions {
        return versions
            .iter()
            .copied()
            .map(|version| BedrockProtocolOptions::from_config(version).map_err(Into::into))
            .collect();
    }

    Ok(vec![
        BedrockProtocolOptions::from_env_or_default().map_err(|err| anyhow::anyhow!(err))?
    ])
}

fn is_network_settings_failure(error: &dyn std::fmt::Display) -> bool {
    let message = error.to_string();
    message.contains("NetworkSettingsPacket") || message.contains("NetworkSettings")
}

struct ScriptedBot {
    script: Vec<ScriptStep>,
    current_step_index: usize,
    reset_on_spawn: bool,
    player_entity_id: i64,
    player_runtime_id: u64,
    player_position: (f32, f32, f32),
    player_yaw: f32,
    player_pitch: f32,
    current_tick: u64,
    current_window_id: Option<u8>,
    current_window_type: Option<u8>,
    container_items: HashMap<(u32, u32), ObservedInventoryItem>,
}

fn normalize_trigger(t: &str) -> String {
    t.trim().to_lowercase().replace([' ', '-'], "_")
}

impl ScriptedBot {
    fn new(script: Vec<ScriptStep>, reset_on_spawn: bool) -> Self {
        Self {
            script,
            current_step_index: 0,
            reset_on_spawn,
            player_entity_id: 0,
            player_runtime_id: 0,
            player_position: (0.0, 0.0, 0.0),
            player_yaw: 0.0,
            player_pitch: 0.0,
            current_tick: 0,
            current_window_id: None,
            current_window_type: None,
            container_items: HashMap::new(),
        }
    }

    async fn execute_step(
        &self,
        conn: &mut BedrockProtocolAdapter,
        step: &ScriptStep,
    ) -> Result<()> {
        let action = step.action.trim().to_lowercase();
        warn!(
            "[SCRIPT_BOT] executing action={action} trigger={}",
            step.trigger
        );
        match action.as_str() {
            "command" => {
                if let Some(ref cmd) = step.command {
                    conn.send_command(cmd, self.player_entity_id)
                        .await
                        .context("failed to send command")?;
                }
            }
            "respawn" => {
                conn.respawn(self.player_runtime_id)
                    .await
                    .context("failed to send respawn")?;
            }
            "use_block_in_front" => {
                conn.use_block_in_front(
                    self.player_position,
                    self.player_yaw,
                    self.player_pitch,
                    self.current_tick,
                )
                .await
                .context("failed to send use_block_in_front")?;
            }
            "click_slot" => {
                if let Some(slot) = step.slot {
                    let window_id = self.current_window_id.unwrap_or(0);
                    let container_type = self.current_window_type.unwrap_or(0);

                    let (item_id, stack_id) =
                        if let Some(item) = self.container_items.get(&(window_id as u32, slot)) {
                            (item.item_id, item.stack_id.unwrap_or(0))
                        } else {
                            (0, 0)
                        };

                    let target = MenuClickTarget {
                        window_id: window_id as u32,
                        slot,
                        item_id,
                        stack_id,
                        container_type,
                        dynamic_container_id: None,
                        priority: 0,
                    };
                    conn.click_slot(&target, MenuClickMethod::StandaloneObservedTake)
                        .await
                        .context("failed to click slot")?;
                }
            }
            other => {
                warn!("[SCRIPT_BOT] unknown action: {}", other);
            }
        }
        Ok(())
    }

    async fn run_chain(&mut self, conn: &mut BedrockProtocolAdapter) -> Result<()> {
        while self.current_step_index < self.script.len() {
            let next_step = &self.script[self.current_step_index];
            if normalize_trigger(&next_step.trigger) == "after_previous" {
                warn!(
                    "[SCRIPT_BOT] executing after_previous step index={}",
                    self.current_step_index
                );
                self.execute_step(conn, next_step).await?;
                self.current_step_index += 1;
            } else {
                break;
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl InstantScript for ScriptedBot {
    async fn handle_event(
        &mut self,
        conn: &mut BedrockProtocolAdapter,
        _session: &ProvisionedBedrockSession,
        event: InstantScriptEvent,
    ) -> torchflower_engine::error::EngineResult<()> {
        self.current_tick = self.current_tick.saturating_add(1);

        let res: Result<()> = async {
            match event {
                InstantScriptEvent::Spawn { runtime_id, entity_id, position } => {
                    warn!("[SCRIPT_BOT] Spawn event: runtime_id={runtime_id}, entity_id={entity_id}, position={position:?}");
                    self.player_runtime_id = runtime_id;
                    self.player_entity_id = entity_id;
                    self.player_position = position;

                    if self.reset_on_spawn {
                        self.current_step_index = 0;
                    }

                    if self.current_step_index < self.script.len() {
                        let step = &self.script[self.current_step_index];
                        if normalize_trigger(&step.trigger) == "on_spawn" {
                            self.execute_step(conn, step).await?;
                            self.current_step_index += 1;
                            self.run_chain(conn).await?;
                        }
                    }
                }
                InstantScriptEvent::InventoryGui { window_id, window_type } => {
                    warn!("[SCRIPT_BOT] InventoryGui event: window_id={window_id}, window_type={window_type}");
                    self.current_window_id = Some(window_id);
                    self.current_window_type = Some(window_type);

                    if self.current_step_index < self.script.len() {
                        let step = &self.script[self.current_step_index];
                        if normalize_trigger(&step.trigger) == "on_inventory_gui" {
                            self.execute_step(conn, step).await?;
                            self.current_step_index += 1;
                            self.run_chain(conn).await?;
                        }
                    }
                }
                InstantScriptEvent::InventoryContent { container_id, items } => {
                    for item in items {
                        self.container_items.insert((container_id, item.slot), item);
                    }
                }
                InstantScriptEvent::InventorySlot { container_id, slot, item } => {
                    if let Some(it) = item {
                        self.container_items.insert((container_id, slot), it);
                    } else {
                        self.container_items.remove(&(container_id, slot));
                    }
                }
                InstantScriptEvent::Death => {
                    warn!("[SCRIPT_BOT] Death event");
                    for step in &self.script {
                        if normalize_trigger(&step.trigger) == "on_death" {
                            self.execute_step(conn, step).await?;
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }.await;

        res.map_err(|e| torchflower_engine::error::EngineError::Bedrock(e.to_string()))
    }
}

fn is_fatal_auth_failure(error: &dyn std::fmt::Display) -> bool {
    let msg = error.to_string();
    msg.contains("Real Xbox Live authentication is required")
        || msg.contains("rejected offline/mock login")
        || msg.contains("Server disconnected during login handshake")
        || msg.contains("Server returned PlayStatus failure")
        || msg.contains("Session file not found or invalid")
        || msg.contains("real auth mode requires a valid account/session source")
}

fn load_bot_session(
    bot: &BotConfig,
    config_dir: Option<&Path>,
    lite_config: &LiteConfig,
) -> Result<ProvisionedBedrockSession> {
    let is_real_auth = if let Some(ref mode) = bot.auth_mode {
        mode != "offline" && mode != "mock"
    } else {
        bot.account_id.is_some()
    };

    if is_real_auth {
        let account_id = bot.account_id.as_deref().ok_or_else(|| {
            anyhow::anyhow!("real auth mode requires a valid account/session source (account_id must be specified)")
        })?;

        // 1. Search in accounts list for custom token_cache
        if let Some(accounts) = &lite_config.accounts {
            if let Some(acc) = accounts.iter().find(|a| a.id == account_id) {
                if let Some(ref token_cache) = acc.token_cache {
                    let path = PathBuf::from(token_cache);
                    let resolved_path = if path.is_absolute() {
                        path
                    } else if let Some(dir) = config_dir {
                        dir.join(path)
                    } else {
                        path
                    };
                    if resolved_path.exists() {
                        let content = fs::read_to_string(&resolved_path)?;
                        let session: ProvisionedBedrockSession = serde_json::from_str(&content)
                            .map_err(|e| {
                                anyhow::anyhow!(
                                    "failed to parse session cache file {:?}: {}",
                                    resolved_path,
                                    e
                                )
                            })?;
                        return Ok(session);
                    } else {
                        bail!(
                            "Session file not found or invalid at {:?} for account_id='{}'",
                            resolved_path,
                            account_id
                        );
                    }
                }
            }
        }

        // 2. Resolve cache dir
        let cache_dir_str = lite_config
            .auth
            .as_ref()
            .and_then(|a| a.cache_dir.clone())
            .or_else(|| std::env::var("TORCHFLOWER_AUTH_CACHE_DIR").ok())
            .unwrap_or_else(|| ".torchflower/accounts".to_string());

        let cache_dir = PathBuf::from(cache_dir_str);
        let resolved_cache_dir = if cache_dir.is_absolute() {
            cache_dir
        } else if let Some(dir) = config_dir {
            dir.join(cache_dir)
        } else {
            cache_dir
        };

        let session_file = resolved_cache_dir.join(format!("{}.json", account_id));
        if session_file.exists() {
            let content = fs::read_to_string(&session_file)?;
            let session: ProvisionedBedrockSession =
                serde_json::from_str(&content).map_err(|e| {
                    anyhow::anyhow!(
                        "failed to parse session cache file {:?}: {}",
                        session_file,
                        e
                    )
                })?;
            Ok(session)
        } else {
            bail!(
                "Session file not found or invalid at {:?} for account_id='{}'",
                session_file,
                account_id
            );
        }
    } else {
        // Offline/mock mode
        let name = bot.username.as_deref().unwrap_or("Bot_1");
        create_offline_session(name)
            .map_err(|e| anyhow::anyhow!("failed to create offline session: {}", e))
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_single_bot(
    index: usize,
    server: ServerConfig,
    protocol_attempts: Vec<BedrockProtocolOptions>,
    bot: BotConfig,
    duration_secs: u64,
    reconnect: bool,
    config_dir: Option<PathBuf>,
    lite_config: LiteConfig,
) -> Result<()> {
    let name = bot
        .username
        .clone()
        .or(bot.account_id.clone())
        .unwrap_or_else(|| format!("bot_{}", index + 1));

    let mode = bot.mode.clone().unwrap_or_else(|| "afk".to_string());

    warn!(
        "starting bot #{index}: name={name}, mode={mode}, server={host}:{port}, reconnect={reconnect}",
        index = index + 1,
        host = server.host,
        port = server.port
    );

    let script_steps = bot.script.clone().unwrap_or_default();
    let reset_on_spawn = bot.reset_on_spawn.unwrap_or(true);

    'reconnect_loop: loop {
        let mut exhausted_protocol_probe = false;

        for (attempt_index, protocol_options) in protocol_attempts.iter().copied().enumerate() {
            warn!(
                "bot {name} connecting to {host}:{port} with protocol_version={protocol} codec_protocol_version={codec} attempt={attempt}/{total}",
                host = server.host,
                port = server.port,
                protocol = protocol_options.requested_protocol_version,
                codec = protocol_options.codec_protocol_version_number(),
                attempt = attempt_index + 1,
                total = protocol_attempts.len()
            );

            let session = match load_bot_session(&bot, config_dir.as_deref(), &lite_config) {
                Ok(s) => s,
                Err(e) => {
                    warn!("bot {name} failed to load session: {e:?}");
                    break 'reconnect_loop;
                }
            };

            let engine_db = match torchflower_engine::db::Database::connect("").await {
                Ok(db) => db,
                Err(e) => {
                    warn!("failed to connect dummy database: {e:?}");
                    if !reconnect {
                        break 'reconnect_loop;
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue 'reconnect_loop;
                }
            };

            let bot_session = BedrockBotSession::new(engine_db);
            let script = ScriptedBot::new(script_steps.clone(), reset_on_spawn);

            let run_duration = if duration_secs == 0 {
                Duration::from_secs(3600 * 24 * 365) // 1 year
            } else {
                Duration::from_secs(duration_secs)
            };

            let run_res = bot_session
                .run_instant_script(
                    &name,
                    None,
                    &server.host,
                    server.port,
                    &session,
                    protocol_options,
                    script,
                    run_duration,
                )
                .await;

            if let Err(e) = run_res {
                warn!(
                    "bot {name} connection error with protocol_version={protocol} codec_protocol_version={codec}: {e:?}",
                    protocol = protocol_options.requested_protocol_version,
                    codec = protocol_options.codec_protocol_version_number()
                );

                if is_fatal_auth_failure(&e) {
                    warn!("bot {name} fatal authentication or login rejection error: {e}. Stopping bot task.");
                    break 'reconnect_loop;
                }

                if protocol_attempts.len() > 1 && is_network_settings_failure(&e) {
                    if attempt_index + 1 < protocol_attempts.len() {
                        warn!("bot {name} trying next configured protocol version after NetworkSettings failure");
                        continue;
                    }
                    exhausted_protocol_probe = true;
                }
            } else {
                warn!(
                    "bot {name} session completed with protocol_version={protocol} codec_protocol_version={codec}",
                    protocol = protocol_options.requested_protocol_version,
                    codec = protocol_options.codec_protocol_version_number()
                );
            }

            break;
        }

        if exhausted_protocol_probe {
            warn!(
                "bot {name} exhausted configured protocol_versions after NetworkSettings failures"
            );
            break;
        }

        if !reconnect || duration_secs > 0 {
            break;
        }

        warn!("bot {name} reconnecting in 5 seconds...");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    warn!("bot {name} stopped");
    Ok(())
}

async fn run_bench(bots: usize, duration: &str) -> Result<()> {
    if bots == 0 {
        bail!("--bots must be greater than 0");
    }

    let duration = parse_duration(duration)?;
    let started = Instant::now();

    warn!("starting lightweight benchmark: bots={bots}, duration={duration:?}");

    let mut handles = Vec::with_capacity(bots);

    for index in 0..bots {
        handles.push(tokio::spawn(async move {
            let mut ticks = 0_u64;
            let deadline = Instant::now() + duration;

            while Instant::now() < deadline {
                ticks = ticks.saturating_add(1);
                tokio::time::sleep(Duration::from_millis(250)).await;
            }

            (index, ticks)
        }));
    }

    let mut total_ticks = 0_u64;

    for handle in handles {
        let (_index, ticks) = handle.await.context("benchmark task panicked")?;
        total_ticks = total_ticks.saturating_add(ticks);
    }

    println!("Benchmark complete");
    println!("bots: {bots}");
    println!("duration: {:?}", started.elapsed());
    println!("total simulated ticks: {total_ticks}");
    println!(
        "note: this is a lightweight runtime benchmark, not a real Bedrock protocol benchmark yet"
    );

    Ok(())
}

fn parse_duration(input: &str) -> Result<Duration> {
    let input = input.trim();

    if input.is_empty() {
        bail!("duration cannot be empty");
    }

    let split_at = input
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(input.len());

    let number = &input[..split_at];
    let unit = &input[split_at..];

    let value: u64 = number
        .parse()
        .with_context(|| format!("invalid duration number: {number}"))?;

    match unit {
        "" | "s" | "sec" | "secs" => Ok(Duration::from_secs(value)),
        "m" | "min" | "mins" => Ok(Duration::from_secs(value * 60)),
        "h" | "hr" | "hrs" => Ok(Duration::from_secs(value * 60 * 60)),
        _ => bail!("unsupported duration unit: {unit}. Use s, m, or h."),
    }
}

fn default_config_text() -> &'static str {
    r#"[server]
host = "127.0.0.1"
port = 19132
# Optional: pin a Bedrock protocol version for this server.
# This takes priority over TORCHFLOWER_BEDROCK_PROTOCOL_VERSION and BEDROCK_PROTOCOL_VERSION.
# protocol_version = 898
# Optional: probe a bounded list once, in order, when NetworkSettings fails.
# protocol_versions = [893, 898, 899]

[runtime]
log_level = "warn"
duration_secs = 0
reconnect = true

# [auth]
# cache_dir = ".torchflower/accounts"

# [[accounts]]
# id = "my-account"
# token_cache = "accounts/my-account.json"

[[bots]]
username = "Bot_1"
mode = "afk"
# account_id = "my-account" # Set to enable real authenticated Xbox Live login
# auth_mode = "microsoft" # Optional: force specific auth mode ("offline" or "microsoft")
# Scripted bots reset to the first script step on every Spawn by default.
# Set reset_on_spawn = false when a generic script should continue after respawn.

# For private scripted bots, create your own bots.toml outside the repo.
# Do not commit server-specific scripts, private commands, usernames, targets,
# or server-specific behavior into TorchFlower.
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_lite_config_parsing_and_validation() {
        let toml_text = r#"
            [server]
            host = "127.0.0.1"
            port = 19132
            protocol_version = 766

            [runtime]
            log_level = "info"

            [[bots]]
            username = "Bot_Test"
            mode = "afk"
            reset_on_spawn = false
        "#;

        let config: LiteConfig = toml::from_str(toml_text).unwrap();
        assert_eq!(config.server.protocol_version, Some(766));
        assert_eq!(config.bots[0].reset_on_spawn, Some(false));
        assert!(config.validate().is_ok());

        let invalid_toml_text = r#"
            [server]
            host = "127.0.0.1"
            port = 19132
            protocol_version = -10

            [runtime]
            log_level = "info"

            [[bots]]
            username = "Bot_Test"
            mode = "afk"
        "#;

        let config_invalid: LiteConfig = toml::from_str(invalid_toml_text).unwrap();
        assert_eq!(config_invalid.server.protocol_version, Some(-10));
        assert!(config_invalid.validate().is_err());

        let invalid_list_toml_text = r#"
            [server]
            host = "127.0.0.1"
            port = 19132
            protocol_versions = [893, 0]

            [runtime]
            log_level = "info"

            [[bots]]
            username = "Bot_Test"
            mode = "afk"
        "#;

        let config_invalid: LiteConfig = toml::from_str(invalid_list_toml_text).unwrap();
        assert!(config_invalid.validate().is_err());
    }

    #[test]
    fn test_protocol_override_resolution() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("TORCHFLOWER_BEDROCK_PROTOCOL_VERSION");
        std::env::remove_var("BEDROCK_PROTOCOL_VERSION");

        // 1. Fallback default
        let config = LiteConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 19132,
                protocol_version: None,
                protocol_versions: None,
            },
            runtime: RuntimeConfig {
                log_level: None,
                duration_secs: None,
                reconnect: None,
            },
            auth: None,
            accounts: None,
            bots: vec![],
        };
        let resolved = resolve_protocol_attempts(&config.server).unwrap();
        assert_eq!(resolved[0].requested_protocol_version, 898);

        // 2. Env variable takes precedence over default
        std::env::set_var("BEDROCK_PROTOCOL_VERSION", "662");
        std::env::set_var("TORCHFLOWER_BEDROCK_PROTOCOL_VERSION", "975");
        let resolved = resolve_protocol_attempts(&config.server).unwrap();
        assert_eq!(resolved[0].requested_protocol_version, 975);

        // 3. Config takes precedence over env variable
        let config_with_ver = LiteConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 19132,
                protocol_version: Some(766),
                protocol_versions: Some(vec![893, 898]),
            },
            runtime: RuntimeConfig {
                log_level: None,
                duration_secs: None,
                reconnect: None,
            },
            auth: None,
            accounts: None,
            bots: vec![],
        };
        let resolved = resolve_protocol_attempts(&config_with_ver.server).unwrap();
        assert_eq!(resolved[0].requested_protocol_version, 766);
        assert_eq!(resolved.len(), 1);

        let config_with_versions = LiteConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 19132,
                protocol_version: None,
                protocol_versions: Some(vec![893, 898]),
            },
            runtime: RuntimeConfig {
                log_level: None,
                duration_secs: None,
                reconnect: None,
            },
            auth: None,
            accounts: None,
            bots: vec![],
        };
        let resolved = resolve_protocol_attempts(&config_with_versions.server).unwrap();
        assert_eq!(
            resolved
                .iter()
                .map(|option| option.requested_protocol_version)
                .collect::<Vec<_>>(),
            vec![893, 898]
        );

        std::env::remove_var("TORCHFLOWER_BEDROCK_PROTOCOL_VERSION");
        std::env::remove_var("BEDROCK_PROTOCOL_VERSION");
    }

    #[test]
    fn test_fatal_auth_failure_classification() {
        let err1 = anyhow::anyhow!("Server rejected offline/mock login (xuid=0). Real Xbox Live authentication is required. Configure an authenticated account using account_id/auth settings.");
        let err2 = anyhow::anyhow!("some unrelated network issue");
        let err3 = anyhow::anyhow!("Server disconnected during login handshake. Reason: 0");

        assert!(is_fatal_auth_failure(&err1));
        assert!(!is_fatal_auth_failure(&err2));
        assert!(is_fatal_auth_failure(&err3));
    }

    #[test]
    fn test_config_parsing_account_id_and_auth() {
        let toml_text = r#"
            [server]
            host = "127.0.0.1"
            port = 19132

            [runtime]

            [auth]
            cache_dir = "custom/cache/dir"

            [[accounts]]
            id = "my-account"
            token_cache = "my-account.json"

            [[bots]]
            account_id = "my-account"
            auth_mode = "microsoft"
            mode = "afk"
        "#;

        let config: LiteConfig = toml::from_str(toml_text).unwrap();
        assert_eq!(
            config.auth.as_ref().unwrap().cache_dir.as_deref(),
            Some("custom/cache/dir")
        );
        assert_eq!(config.accounts.as_ref().unwrap()[0].id, "my-account");
        assert_eq!(
            config.accounts.as_ref().unwrap()[0].token_cache.as_deref(),
            Some("my-account.json")
        );
        assert_eq!(config.bots[0].account_id.as_deref(), Some("my-account"));
        assert_eq!(config.bots[0].auth_mode.as_deref(), Some("microsoft"));
    }

    #[test]
    fn test_real_auth_requires_valid_session_source() {
        let config = LiteConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 19132,
                protocol_version: None,
                protocol_versions: None,
            },
            runtime: RuntimeConfig {
                log_level: None,
                duration_secs: None,
                reconnect: None,
            },
            auth: None,
            accounts: None,
            bots: vec![BotConfig {
                username: None,
                account_id: Some("missing-account".to_string()),
                auth_mode: None,
                mode: None,
                reset_on_spawn: None,
                script: None,
            }],
        };

        // Attempting to load a bot session with account_id but no cache file should fail
        let res = load_bot_session(&config.bots[0], None, &config);
        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("Session file not found or invalid"));
        // Make sure it doesn't print any raw tokens
        assert!(!err_msg.contains("Token"));
    }
}
