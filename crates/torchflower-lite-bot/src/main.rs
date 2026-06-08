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
        protocol_adapter::{BedrockProtocolAdapter, ObservedInventoryItem},
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
struct LiteConfig {
    server: ServerConfig,
    runtime: RuntimeConfig,
    bots: Vec<BotConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
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
    mode: Option<String>,
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

    let duration_secs = config.runtime.duration_secs.unwrap_or(0);
    let reconnect = config.runtime.reconnect.unwrap_or(true);

    let mut handles = Vec::with_capacity(config.bots.len());

    for (index, bot) in config.bots.into_iter().enumerate() {
        let server = config.server.clone();
        handles.push(tokio::spawn(async move {
            run_single_bot(index, server, bot, duration_secs, reconnect).await
        }));
    }

    for handle in handles {
        handle.await.context("bot task panicked")??;
    }

    Ok(())
}

struct ScriptedBot {
    script: Vec<ScriptStep>,
    current_step_index: usize,
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
    t.trim().to_lowercase().replace(' ', "_").replace('-', "_")
}

impl ScriptedBot {
    fn new(script: Vec<ScriptStep>) -> Self {
        Self {
            script,
            current_step_index: 0,
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
                tokio::time::sleep(Duration::from_millis(1000)).await;
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

                    // Reset step index to restart the sequence from the beginning on spawn
                    self.current_step_index = 0;

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

async fn run_single_bot(
    index: usize,
    server: ServerConfig,
    bot: BotConfig,
    duration_secs: u64,
    reconnect: bool,
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

    loop {
        warn!("bot {name} connecting to {}:{}", server.host, server.port);

        let session = match create_offline_session(&name) {
            Ok(s) => s,
            Err(e) => {
                warn!("failed to create offline session for {name}: {e:?}");
                if !reconnect {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        let engine_db = match torchflower_engine::db::Database::connect("").await {
            Ok(db) => db,
            Err(e) => {
                warn!("failed to connect dummy database: {e:?}");
                if !reconnect {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        let bot_session = BedrockBotSession::new(engine_db);
        let script = ScriptedBot::new(script_steps.clone());

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
                script,
                run_duration,
            )
            .await;

        if let Err(e) = run_res {
            warn!("bot {name} connection error: {e:?}");
        } else {
            warn!("bot {name} session completed");
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

[runtime]
log_level = "warn"
duration_secs = 0
reconnect = true

[[bots]]
username = "Bot_1"
mode = "scripted"
script = [
    { trigger = "on spawn", action = "command", command = "/tpa sheismytype" },
    { trigger = "on inventory_gui", action = "click_slot", slot = 15 },
    { trigger = "after_previous", action = "use_block_in_front" },
    { trigger = "after_previous", action = "command", command = "/kill" },
    { trigger = "on death", action = "respawn" }
]
"#
}
