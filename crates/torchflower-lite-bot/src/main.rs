use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::runtime::Builder;
use tracing::{info, warn};

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

    let mode = bot.mode.unwrap_or_else(|| "afk".to_string());

    warn!(
        "starting bot #{index}: name={name}, mode={mode}, server={host}:{port}, reconnect={reconnect}",
        index = index + 1,
        host = server.host,
        port = server.port
    );

    if duration_secs == 0 {
        warn!(
            "bot {name} would run until stopped. Real Bedrock session wiring must be connected here."
        );
        tokio::signal::ctrl_c()
            .await
            .context("failed to listen for ctrl-c")?;
    } else {
        tokio::time::sleep(Duration::from_secs(duration_secs)).await;
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
mode = "kill-loop"

[[bots]]
username = "Bot_2"
mode = "kill-loop"

# Future real-auth format:
#
# [[bots]]
# account_id = "account1"
# mode = "kill-loop"
"#
}
