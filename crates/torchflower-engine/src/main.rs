#![allow(unknown_lints)]

use std::net::SocketAddr;

use torchflower_engine::{
    api,
    config::Config,
    db::Database,
    validation::{ChestRoomBotValidation, RealServerValidation},
};
#[cfg(not(feature = "console"))]
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let workers = read_env_usize("TORCHFLOWER_WORKERS", num_cpus::get()).max(1);
    let thread_stack_size = read_env_usize("TORCHFLOWER_THREAD_STACK_BYTES", 2 * 1024 * 1024);
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .thread_stack_size(thread_stack_size)
        .enable_all()
        .build()?
        .block_on(async_main())
}

fn init_tracing() {
    #[cfg(feature = "console")]
    {
        console_subscriber::init();
    }

    #[cfg(not(feature = "console"))]
    {
        tracing_subscriber::registry()
            .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
            .with(tracing_subscriber::fmt::layer())
            .init();
    }
}

async fn async_main() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let db = Database::connect(&config.database_url).await?;
    db.migrate().await?;

    match std::env::args().nth(1).as_deref() {
        Some("validate-real-server") => {
            RealServerValidation::new(config, db).run_from_env().await?;
        }
        Some("run-room-bot") => {
            ChestRoomBotValidation::new(config, db)
                .run_from_env()
                .await?;
        }
        _ => {
            let bind: SocketAddr = config.rust_engine_bind.parse()?;
            api::serve(config, db, bind).await?;
        }
    }

    Ok(())
}

fn read_env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}
