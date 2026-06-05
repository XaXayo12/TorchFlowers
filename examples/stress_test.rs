#![allow(unknown_lints)]

use std::{net::SocketAddr, time::Duration};

use torchflower::{BotCommand, BotPool, PoolConfig, ServerAddr};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bot_count = read_env_usize("BOT_COUNT", 10);
    let host = std::env::var("MINECRAFT_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = read_env_u16("MINECRAFT_PORT", 19132);
    let server: ServerAddr = format!("{host}:{port}").parse::<SocketAddr>()?;

    if !is_local_target(&server) {
        eprintln!(
            "stress_test is intended for local/offline-mode servers unless you control the target"
        );
        std::process::exit(1);
    }

    let mut pool = BotPool::new(PoolConfig {
        max_concurrent: bot_count,
        max_auth_concurrent: read_env_usize("TORCHFLOWER_MAX_AUTH_CONCURRENT", 3),
        spawn_interval: Duration::from_millis(read_env_u64("TORCHFLOWER_SPAWN_INTERVAL_MS", 500)),
        command_buffer: 128,
        buffer_size: 16 * 1024,
    })
    .await;

    let started = std::time::Instant::now();
    let accounts = (0..bot_count)
        .map(|index| format!("OfflineBot{index}"))
        .collect::<Vec<_>>();
    let results = pool.spawn_batch(accounts, server).await;
    let connected = results.iter().filter(|result| result.is_ok()).count();
    let time_to_all = started.elapsed();

    for seconds in [5_u64, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60] {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let stats = pool.memory_stats();
        println!(
            "[T+{}s] bots={}/{} connected={} pkt/s={} mem={}MB buf_hits={:.1}%",
            seconds,
            stats.bots_active,
            bot_count,
            connected,
            0,
            stats.heap_bytes_estimated / (1024 * 1024),
            stats.buffer_pool_hit_rate * 100.0
        );
        pool.broadcast(BotCommand::SendChat("TorchFlower stress tick".to_string()))
            .await;
    }

    pool.shutdown_all().await;
    println!(
        "time_to_first_connected_ms={} time_to_all_connected_ms={} disconnects={}",
        if connected > 0 {
            time_to_all.as_millis()
        } else {
            0
        },
        time_to_all.as_millis(),
        bot_count.saturating_sub(connected)
    );

    if connected == bot_count {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

fn read_env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn read_env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn read_env_u16(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(default)
}

fn is_local_target(server: &SocketAddr) -> bool {
    server.ip().is_loopback()
}
