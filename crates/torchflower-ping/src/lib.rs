use std::time::Duration;
use torchflower_network::native::NativePingClient;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerStatus {
    pub motd: String,
    pub level_name: String,
    pub protocol: u32,
    pub version: String,
    pub player_count: u32,
    pub player_max: u32,
    pub server_guid: u64,
    pub gamemode: String,
    pub ipv4_port: Option<String>,
    pub ipv6_port: Option<String>,
}

pub async fn ping(host: &str, port: u16) -> Result<ServerStatus, anyhow::Error> {
    let client = NativePingClient::new(Duration::from_secs(5));
    let response = client.ping(host, port).await?;
    
    Ok(ServerStatus {
        motd: response.motd.name,
        level_name: response.motd.sub_name,
        protocol: response.motd.protocol as u32,
        version: response.motd.version,
        player_count: response.motd.player_count,
        player_max: response.motd.player_max,
        server_guid: response.motd.server_guid,
        gamemode: response.motd.gamemode.as_str().to_string(),
        ipv4_port: response.motd.port,
        ipv6_port: response.motd.ipv6_port,
    })
}
