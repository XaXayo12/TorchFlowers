use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use torchflower_engine::auth::minecraft::MinecraftAuth;
use torchflower_engine::auth::ProvisionedBedrockSession;
use torchflower_engine::bedrock::protocol_adapter::BedrockProtocolAdapter;
use torchflower_engine::bedrock::session::{BedrockBotSession, InstantScript, InstantScriptEvent};
use torchflower_engine::db::Database;
use torchflower_engine::error::EngineResult;

#[derive(Debug, Deserialize)]
struct BotConfig {
    username: String,
}

#[derive(Debug, Deserialize)]
struct Config {
    server: ServerConfig,
    bots: Vec<BotConfig>,
}

#[derive(Debug, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
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

    // eyJhbGciOiJFUzM4NCJ9 is {"alg":"ES384"}
    // eyJleHRyYURhdGEiOnsiZGlzcGxheU5hbWUiOiJQbGF5ZXIiLCJYVUlEIjoiMTIzNDUifX0 is {"extraData":{"displayName":"Player","XUID":"12345"}}
    let dummy_jwt = "eyJhbGciOiJFUzM4NCJ9.eyJleHRyYURhdGEiOnsiZGlzcGxheU5hbWUiOiJCb3RQbGF5ZXIiLCJYVUlEIjoiMTIzNDU2Nzg5MDEyMzQ1In19.dHVtbXk".to_string();
    // Let's create a minimal valid legacy chain.
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

fn main() -> Result<(), anyhow::Error> {
    // Setup tracing/logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("run");

    if cmd == "bench" {
        run_benchmark(args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10));
    } else {
        run_bots()?;
    }

    Ok(())
}

fn run_bots() -> Result<(), anyhow::Error> {
    let config_content = std::fs::read_to_string("bots.toml").unwrap_or_else(|_| {
        println!("No bots.toml found. Creating a default one...");
        let default_toml = r#"
[server]
host = "127.0.0.1"
port = 19132

[[bots]]
username = "AFKBot_1"

[[bots]]
username = "AFKBot_2"
"#;
        std::fs::write("bots.toml", default_toml).unwrap();
        default_toml.to_string()
    });

    let config: Config = toml::from_str(&config_content)?;
    println!(
        "Loaded {} bots. Connecting to {}:{}...",
        config.bots.len(),
        config.server.host,
        config.server.port
    );

    // Run using current thread runtime for lowest memory consumption
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
            let username = bot.username.clone();

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
                        Duration::from_secs(3600), // Run for 1 hour or until disconnect
                    )
                    .await;

                if let Err(e) = res {
                    eprintln!("[{}] Session loop terminated with error: {:?}", username, e);
                }
            });
            handles.push(handle);
        }

        // Wait for all bots
        for handle in handles {
            let _ = handle.await;
        }
    });

    Ok(())
}

fn run_benchmark(num_bots: usize) {
    println!("=== TorchFlower Lite Bot Memory & CPU Benchmark ===");
    println!("Target: Spawning {} concurrent bot sessions...", num_bots);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let session_manager = Arc::new(BedrockBotSession::new(db));

        let mut handles = Vec::new();
        println!("Spawning bots in background (waiting for connections)...");

        for i in 0..num_bots {
            let username = format!("BenchBot_{}", i);
            let session_manager = session_manager.clone();
            let session = generate_mock_session(&username);
            let script = SimpleAfkScript {
                username: username.clone(),
            };

            let handle = tokio::spawn(async move {
                // Connect to a dummy/non-listening local port to measure connection setup allocation overhead
                let _ = session_manager
                    .run_instant_script(
                        &username,
                        None,
                        "127.0.0.1",
                        19199,
                        &session,
                        script,
                        Duration::from_secs(5),
                    )
                    .await;
            });
            handles.push(handle);
        }

        println!("Successfully spawned {} bot connection tasks.", num_bots);
        println!("Waiting 3 seconds to let connection attempts start and allocate state...");
        sleep(Duration::from_secs(3)).await;

        // Print OS memory details if we can, otherwise user will measure
        println!("\nBenchmark ready. Run the following command in another window or wait for the script to finish:");
        println!("  Get-Process -Id {} | Select-Object WorkingSet64, PrivateMemorySize64", std::process::id());
        println!("\nWaiting for benchmark to complete...");

        // Wait a bit more
        sleep(Duration::from_secs(5)).await;
    });

    println!("Benchmark finished.");
}
