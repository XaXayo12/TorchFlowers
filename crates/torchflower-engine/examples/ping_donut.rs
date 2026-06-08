#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Pinging donutsmp.net...");
    match torchflower_ping::ping("donutsmp.net", 19132).await {
        Ok(status) => {
            println!("Ping succeeded!");
            println!("MOTD: {}", status.motd);
            println!("Protocol: {}", status.protocol);
            println!("Version: {}", status.version);
            println!("Players: {}/{}", status.player_count, status.player_max);
        }
        Err(err) => {
            println!("Ping failed: {:?}", err);
        }
    }
    Ok(())
}
