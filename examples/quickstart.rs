#![allow(unknown_lints)]

use std::time::Duration;

use chrono::Utc;
use torchflower::{AuthConfig, BotBuilder, Event, ProtocolVersion};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let host = std::env::var("MINECRAFT_HOST").unwrap_or_else(|_| "play.example.com".to_string());
    let port = std::env::var("MINECRAFT_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(19132);

    println!("Open the Microsoft device-code page when prompted by your auth client.");
    println!("Target server: {host}:{port}");

    let bot = BotBuilder::new()
        .address(host, port)
        .protocol_version(ProtocolVersion::V1_21_100)
        .auth(AuthConfig::device_code())
        .build()
        .await?;

    bot.run(|ctx, event| {
        Box::pin(async move {
            match event {
                Event::Spawned => ctx.send_chat("Hello from TorchFlower!").await?,
                Event::Chat { sender, message } => {
                    println!("[{}] {}: {}", Utc::now().to_rfc3339(), sender, message);
                }
                _ => {}
            }
            Ok(())
        })
    })
    .await?;

    tokio::time::sleep(Duration::from_secs(60)).await;
    Ok(())
}
