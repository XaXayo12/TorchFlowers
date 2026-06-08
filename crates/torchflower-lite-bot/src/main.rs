use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use torchflower_engine::auth::minecraft::MinecraftAuth;
use torchflower_engine::auth::ProvisionedBedrockSession;
use torchflower_engine::bedrock::protocol_adapter::BedrockProtocolAdapter;
use torchflower_engine::bedrock::session::{BedrockBotSession, InstantScript, InstantScriptEvent};
use torchflower_engine::db::Database;
use torchflower_engine::error::EngineResult;

#[derive(Debug, Parser)]
#[command(name = "torchflower-lite-bot")]
#[command(author = "TorchFlower Contributors")]
#[command(version = "0.1.0")]
#[command(about = "Extremely lightweight Bedrock AFK bot runtime for TorchFlower.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start bots using a config file
    Run {
        /// Path to the configuration file
        #[arg(short, long, default_value = "bots.toml")]
        config: PathBuf,
    },
    /// Benchmark resource usage of concurrent bot sessions
    Bench {
        /// Number of concurrent bots to spawn
        #[arg(short, long, default_value_t = 10)]
        bots: usize,

        /// Duration of the benchmark (e.g. 60s, 10m, 1h)
        #[arg(short, long, default_value = "60s")]
        duration: String,
    },
    /// Initialize a default bots.toml configuration file
    Init {
        /// Output path for the default configuration
        #[arg(short, long, default_value = "bots.toml")]
        output: PathBuf,
    },
}

#[derive(Debug, Deserialize, Serialize)]
struct Config {
    server: ServerConfig,
    runtime: Option<RuntimeConfig>,
    bots: Vec<BotConfig>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ServerConfig {
    host: String,
    port: u16,
}

#[derive(Debug, Deserialize, Serialize)]
struct RuntimeConfig {
    #[serde(default = "default_log_level")]
    log_level: String,
    #[serde(default = "default_duration_secs")]
    duration_secs: u64,
    #[serde(default = "default_reconnect")]
    reconnect: bool,
}

fn default_log_level() -> String {
    "warn".to_string()
}
fn default_duration_secs() -> u64 {
    0
}
fn default_reconnect() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize)]
struct BotConfig {
    username: Option<String>,
    account_id: Option<String>,
    #[serde(default = "default_mode")]
    mode: String,
}

fn default_mode() -> String {
    "kill-loop".to_string()
}

struct SimpleAfkScript {
    username: String,
}

#[async_trait::async_trait]
impl InstantScript for SimpleAfkScript {
    async fn handle_event(
        &mut self,
        _conn: &mut BedrockProtocolAdapter,
        _session: &ProvisionedBedrockSession,
        event: InstantScriptEvent,
    ) -> EngineResult<()> {
        match event {
            InstantScriptEvent::Spawn {
                runtime_id,
                position,
            } => {
                println!(
                    "[{}] Spawned in world! Entity Runtime ID: {}, Position: {:?}",
                    self.username, runtime_id, position
                );
            }
            InstantScriptEvent::Death => {
                println!(
                    "[{}] Bot died! Instant respawn is scheduled.",
                    self.username
                );
            }
            InstantScriptEvent::Gui {
                form_id,
                form_content,
            } => {
                println!(
                    "[{}] GUI Opened! Form ID: {}, Content length: {} chars",
                    self.username,
                    form_id,
                    form_content.len()
                );
            }
        }
        Ok(())
    }
}

fn generate_mock_session(username: &str) -> ProvisionedBedrockSession {
    let (_signing_key, private_key_pem, public_key_der_base64) =
        MinecraftAuth::generate_device_keypair().unwrap();

    let dummy_jwt = "eyJhbGciOiJFUzM4NCJ9.eyJleHRyYURhdGEiOnsiZGlzcGxheU5hbWUiOiJCb3RQbGF5ZXIiLCJYVUlEIjoiMTIzNDU2Nzg5MDEyMzQ1In19.dHVtbXk".to_string();
    let legacy_chain = vec![dummy_jwt];

    let chain = MinecraftAuth::build_jwt_chain(
        legacy_chain,
        _signing_key,
        private_key_pem,
        public_key_der_base64,
        username,
        "1234567890",
        Some("playfab-offline-id"),
    )
    .unwrap();

    ProvisionedBedrockSession {
        account_id: username.to_string(),
        playfab_id: "playfab-offline-id".to_string(),
        playfab_session_ticket: "offline-ticket".to_string(),
        minecraft_access_token: "offline-access-token".to_string(),
        bedrock_login_token: "offline-login-token".to_string(),
        legacy_bedrock_token: "offline-legacy-token".to_string(),
        chain,
    }
}

#[cfg(feature = "logging")]
fn init_logging(log_level: &str) {
    let level = match log_level.to_lowercase().as_str() {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "warn" => tracing::Level::WARN,
        _ => tracing::Level::ERROR,
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive(level.into()),
        )
        .try_init();
}

#[cfg(not(feature = "logging"))]
fn init_logging(_log_level: &str) {}

fn parse_duration(s: &str) -> Result<Duration, anyhow::Error> {
    let s = s.trim();
    if s.is_empty() {
        return Err(anyhow::anyhow!("Empty duration"));
    }
    let (num_str, unit) = s.split_at(s.find(|c: char| !c.is_numeric()).unwrap_or(s.len()));
    let num: u64 = num_str.parse()?;
    match unit.trim().to_lowercase().as_str() {
        "s" | "sec" | "secs" | "" => Ok(Duration::from_secs(num)),
        "m" | "min" | "mins" => Ok(Duration::from_secs(num * 60)),
        "h" | "hour" | "hours" => Ok(Duration::from_secs(num * 3600)),
        _ => Err(anyhow::anyhow!("Unknown duration unit: {}", unit)),
    }
}

fn validate_config(config: &Config) -> Result<(), anyhow::Error> {
    if config.server.host.trim().is_empty() {
        return Err(anyhow::anyhow!("server.host cannot be empty"));
    }
    if config.server.port == 0 {
        return Err(anyhow::anyhow!("server.port cannot be 0"));
    }
    if config.bots.is_empty() {
        return Err(anyhow::anyhow!("No bots configured under [[bots]]"));
    }
    for (idx, bot) in config.bots.iter().enumerate() {
        if bot.username.is_none() && bot.account_id.is_none() {
            return Err(anyhow::anyhow!(
                "Bot at index {} is invalid: both 'username' and 'account_id' are missing",
                idx
            ));
        }
    }
    Ok(())
}

/// Entry point for the TorchFlower Lite Bot application.
fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init { output } => {
            if output.exists() {
                return Err(anyhow::anyhow!(
                    "Configuration file already exists at {:?}",
                    output
                ));
            }
            let default_toml = r#"[server]
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
"#;
            std::fs::write(&output, default_toml)?;
            println!("Initialized default configuration file at {:?}", output);
        }
        Command::Run {
            config: config_path,
        } => {
            if !config_path.exists() {
                return Err(anyhow::anyhow!(
                    "Configuration file does not exist at {:?}",
                    config_path
                ));
            }
            let content = std::fs::read_to_string(&config_path)?;
            let config: Config = toml::from_str(&content)
                .map_err(|e| anyhow::anyhow!("Failed to parse configuration: {}", e))?;

            validate_config(&config)?;

            let log_level = config
                .runtime
                .as_ref()
                .map(|r| r.log_level.as_str())
                .unwrap_or("warn");
            init_logging(log_level);

            let duration_secs = config
                .runtime
                .as_ref()
                .map(|r| r.duration_secs)
                .unwrap_or(0);

            println!(
                "Loaded {} bots. Connecting to {}:{}...",
                config.bots.len(),
                config.server.host,
                config.server.port
            );

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;

            rt.block_on(async {
                let db = Database::connect("sqlite::memory:").await.unwrap();
                let session_manager = Arc::new(BedrockBotSession::new(db));
                let host = Arc::new(config.server.host);

                let mut handles = Vec::new();
                for bot in config.bots {
                    let session_manager = session_manager.clone();
                    let host = host.clone();
                    let port = config.server.port;
                    let username = bot
                        .username
                        .clone()
                        .unwrap_or_else(|| bot.account_id.clone().unwrap_or_default());

                    let handle = tokio::spawn(async move {
                        let session = generate_mock_session(&username);
                        let script = SimpleAfkScript {
                            username: username.clone(),
                        };

                        println!("[{}] Starting AFK session loop...", username);
                        let res = session_manager
                            .run_instant_script(
                                &username,
                                None,
                                &host,
                                port,
                                &session,
                                script,
                                if duration_secs > 0 {
                                    Duration::from_secs(duration_secs)
                                } else {
                                    Duration::from_secs(3600 * 24 * 365) // 1 year
                                },
                            )
                            .await;

                        if let Err(e) = res {
                            eprintln!("[{}] Session loop terminated: {:?}", username, e);
                        }
                    });
                    handles.push(handle);
                }

                if duration_secs > 0 {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(duration_secs)) => {
                            println!("Specified runtime duration of {}s elapsed. Shutting down.", duration_secs);
                        }
                        _ = tokio::signal::ctrl_c() => {
                            println!("Interrupted by user. Shutting down.");
                        }
                    }
                } else {
                    println!("Running indefinitely. Press Ctrl+C to stop.");
                    let _ = tokio::signal::ctrl_c().await;
                    println!("Shutting down.");
                }
            });
        }
        Command::Bench { bots, duration } => {
            let duration_limit = parse_duration(&duration)?;
            println!("=== TorchFlower Lite Bot Memory & CPU Benchmark ===");
            println!("Target: Spawning {} concurrent bot sessions...", bots);

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            rt.block_on(async {
                let db = Database::connect("sqlite::memory:").await.unwrap();
                let session_manager = Arc::new(BedrockBotSession::new(db));

                let mut handles = Vec::new();
                println!("Spawning bots in background...");

                for i in 0..bots {
                    let username = format!("BenchBot_{}", i);
                    let session_manager = session_manager.clone();
                    let session = generate_mock_session(&username);
                    let script = SimpleAfkScript {
                        username: username.clone(),
                    };

                    let handle = tokio::spawn(async move {
                        let _ = session_manager
                            .run_instant_script(
                                &username,
                                None,
                                "127.0.0.1",
                                19199,
                                &session,
                                script,
                                duration_limit,
                            )
                            .await;
                    });
                    handles.push(handle);
                }

                println!("Successfully spawned {} bot connection tasks.", bots);
                println!(
                    "Waiting 3 seconds to let connection attempts start and allocate state..."
                );
                sleep(Duration::from_secs(3)).await;

                println!(
                    "\nBenchmark active. Run the following command in another window to monitor:"
                );
                println!("  Windows (PowerShell):");
                println!(
                    "    Get-Process -Id {} | Select-Object WorkingSet64, PrivateMemorySize64",
                    std::process::id()
                );
                println!("  Linux:");
                println!("    ps -o rss= -p {}", std::process::id());
                println!("\nWaiting for benchmark to complete...");

                sleep(duration_limit).await;
            });

            println!("Benchmark finished.");
        }
    }

    Ok(())
}
