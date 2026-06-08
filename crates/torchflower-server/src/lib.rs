use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use torchflower_network::native::NativePingServer;
use torchflower_network::protocol::mcpe::motd::{Motd, Gamemode};
use torchflower_protocol::{Packet, ProtocolVersion};

#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub name: String,
    pub motd: String,
    pub max_players: u32,
    pub protocol_version: ProtocolVersion,
}

#[derive(Debug, Clone)]
pub struct ServerPlayer {
    pub username: String,
    pub runtime_entity_id: u64,
    // Channel or socket to send packets to the player
    packet_tx: mpsc::Sender<Packet>,
}

impl ServerPlayer {
    pub async fn send_packet(&self, packet: Packet) -> Result<(), anyhow::Error> {
        self.packet_tx.send(packet).await?;
        Ok(())
    }

    pub async fn disconnect(&self, reason: &str) -> Result<(), anyhow::Error> {
        let _ = self.send_packet(Packet::Disconnect(torchflower_protocol::DisconnectPacket {
            reason: 0,
            hide_reason: false,
            message: Some(reason.to_string()),
        })).await;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum ServerEvent {
    PlayerJoined(ServerPlayer),
    PlayerLeft(ServerPlayer),
    Packet { player: ServerPlayer, packet: Packet },
}

pub struct Server {
    addr: SocketAddr,
    options: ServerOptions,
    event_tx: Option<mpsc::Sender<ServerEvent>>,
    players: Arc<Mutex<Vec<ServerPlayer>>>,
}

pub fn create_server(addr: SocketAddr, options: ServerOptions) -> Server {
    Server {
        addr,
        options,
        event_tx: None,
        players: Arc::new(Mutex::new(Vec::new())),
    }
}

impl Server {
    pub async fn start(&mut self) -> Result<mpsc::Receiver<ServerEvent>, anyhow::Error> {
        let (tx, rx) = mpsc::channel(1024);
        self.event_tx = Some(tx.clone());

        // Spawn NativePingServer to handle server advertising/pings
        let motd = Motd {
            edition: "MCPE".to_string(),
            name: self.options.name.clone(),
            sub_name: self.options.motd.clone(),
            protocol: self.options.protocol_version.to_u32() as u16,
            version: "1.21.130".to_string(),
            player_count: 0,
            player_max: self.options.max_players,
            gamemode: Gamemode::Survival,
            server_guid: 12345678,
            port: Some(self.addr.port().to_string()),
            ipv6_port: None,
            nintendo_limited: Some(false),
        };

        let ping_server = NativePingServer::bind(self.addr, motd).await?;
        
        // Spawn background task for offline ping requests
        tokio::spawn(async move {
            loop {
                if let Err(_) = ping_server.serve_once().await {
                    // Ignore errors, continue serving
                }
            }
        });

        Ok(rx)
    }

    pub async fn broadcast_packet(&self, packet: Packet) -> Result<(), anyhow::Error> {
        let players = self.players.lock().await;
        for player in players.iter() {
            let _ = player.send_packet(packet.clone()).await;
        }
        Ok(())
    }
}
