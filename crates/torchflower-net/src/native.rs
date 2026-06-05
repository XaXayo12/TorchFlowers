use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use binary_util::{
    interfaces::{Reader, Writer},
    io::ByteReader,
};
use tokio::{
    net::{lookup_host, UdpSocket},
    time::timeout,
};

use crate::protocol::{
    mcpe::motd::Motd,
    packet::{
        offline::{OfflinePacket, UnconnectedPing, UnconnectedPong},
        RakPacket,
    },
    Magic,
};

#[derive(Debug, thiserror::Error)]
pub enum NativePingError {
    #[error("could not resolve Bedrock server address {host}:{port}")]
    Resolve { host: String, port: u16 },
    #[error("udp io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Bedrock server ping timed out after {0:?}")]
    Timeout(Duration),
    #[error("failed to encode RakNet ping packet: {0}")]
    Encode(std::io::Error),
    #[error("failed to decode RakNet pong packet: {0}")]
    Decode(std::io::Error),
    #[error("unexpected packet during Bedrock ping: id=0x{packet_id:02x}")]
    UnexpectedPacket { packet_id: u8 },
}

#[derive(Debug, Clone)]
pub struct PingResponse {
    pub remote_addr: SocketAddr,
    pub latency: Duration,
    pub timestamp: u64,
    pub server_id: u64,
    pub motd: Motd,
}

#[derive(Debug, Clone)]
pub struct NativePingClient {
    timeout: Duration,
    client_id: i64,
}

impl NativePingClient {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            client_id: default_client_id(),
        }
    }

    pub fn with_client_id(mut self, client_id: i64) -> Self {
        self.client_id = client_id;
        self
    }

    pub async fn ping(&self, host: &str, port: u16) -> Result<PingResponse, NativePingError> {
        let remote_addr = resolve_first(host, port).await?;
        self.ping_addr(remote_addr).await
    }

    pub async fn ping_addr(
        &self,
        remote_addr: SocketAddr,
    ) -> Result<PingResponse, NativePingError> {
        let bind_addr = match remote_addr.ip() {
            IpAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
            IpAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
        };
        let socket = UdpSocket::bind(bind_addr).await?;
        let timestamp = monotonic_millis();
        let packet: RakPacket = UnconnectedPing {
            timestamp,
            magic: Magic::new(),
            client_id: self.client_id,
        }
        .into();
        let bytes = packet.write_to_bytes().map_err(NativePingError::Encode)?;
        let start = Instant::now();
        socket.send_to(bytes.as_slice(), remote_addr).await?;

        let mut buffer = [0u8; 4096];
        let (len, from) = timeout(self.timeout, socket.recv_from(&mut buffer))
            .await
            .map_err(|_| NativePingError::Timeout(self.timeout))??;
        let pong = decode_unconnected_pong(&buffer[..len])?;
        Ok(PingResponse {
            remote_addr: from,
            latency: start.elapsed(),
            timestamp: pong.timestamp,
            server_id: pong.server_id,
            motd: pong.motd,
        })
    }
}

impl Default for NativePingClient {
    fn default() -> Self {
        Self::new(Duration::from_secs(5))
    }
}

pub struct NativePingServer {
    socket: UdpSocket,
    motd: Motd,
    server_id: u64,
}

impl NativePingServer {
    pub async fn bind(addr: SocketAddr, motd: Motd) -> Result<Self, NativePingError> {
        let socket = UdpSocket::bind(addr).await?;
        let server_id = motd.server_guid;
        Ok(Self {
            socket,
            motd,
            server_id,
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, NativePingError> {
        Ok(self.socket.local_addr()?)
    }

    pub async fn serve_once(&self) -> Result<PingResponse, NativePingError> {
        let mut buffer = [0u8; 4096];
        let (len, from) = self.socket.recv_from(&mut buffer).await?;
        let ping = decode_unconnected_ping(&buffer[..len])?;
        let response: RakPacket = UnconnectedPong {
            timestamp: ping.timestamp,
            server_id: self.server_id,
            magic: Magic::new(),
            motd: self.motd.clone(),
        }
        .into();
        let bytes = response.write_to_bytes().map_err(NativePingError::Encode)?;
        self.socket.send_to(bytes.as_slice(), from).await?;
        Ok(PingResponse {
            remote_addr: from,
            latency: Duration::ZERO,
            timestamp: ping.timestamp,
            server_id: self.server_id,
            motd: self.motd.clone(),
        })
    }
}

async fn resolve_first(host: &str, port: u16) -> Result<SocketAddr, NativePingError> {
    let mut addrs = lookup_host((host, port)).await?;
    addrs.next().ok_or_else(|| NativePingError::Resolve {
        host: host.to_string(),
        port,
    })
}

fn decode_unconnected_ping(bytes: &[u8]) -> Result<UnconnectedPing, NativePingError> {
    match RakPacket::read(&mut ByteReader::from(bytes)).map_err(NativePingError::Decode)? {
        RakPacket::Offline(OfflinePacket::UnconnectedPing(packet)) => Ok(packet),
        _ => Err(NativePingError::UnexpectedPacket {
            packet_id: bytes.first().copied().unwrap_or_default(),
        }),
    }
}

fn decode_unconnected_pong(bytes: &[u8]) -> Result<UnconnectedPong, NativePingError> {
    match RakPacket::read(&mut ByteReader::from(bytes)).map_err(NativePingError::Decode)? {
        RakPacket::Offline(OfflinePacket::UnconnectedPong(packet)) => Ok(packet),
        _ => Err(NativePingError::UnexpectedPacket {
            packet_id: bytes.first().copied().unwrap_or_default(),
        }),
    }
}

fn monotonic_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn default_client_id() -> i64 {
    let millis = monotonic_millis();
    (millis ^ 0x544f_5243_4846_4c52) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::mcpe::motd::Gamemode;

    #[tokio::test]
    async fn native_ping_client_and_server_round_trip() {
        let motd = Motd {
            edition: "MCPE".to_string(),
            name: "TorchFlower Native Test".to_string(),
            sub_name: "native".to_string(),
            protocol: 975,
            version: "1.21.130".to_string(),
            player_count: 1,
            player_max: 20,
            gamemode: Gamemode::Survival,
            server_guid: 42,
            port: Some("19132".to_string()),
            ipv6_port: Some("19133".to_string()),
            nintendo_limited: Some(false),
        };
        let server =
            NativePingServer::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), motd.clone())
                .await
                .unwrap();
        let addr = server.local_addr().unwrap();
        let server_task = tokio::spawn(async move { server.serve_once().await.unwrap() });

        let response = NativePingClient::new(Duration::from_secs(2))
            .with_client_id(7)
            .ping_addr(addr)
            .await
            .unwrap();
        let observed = server_task.await.unwrap();

        assert_eq!(response.server_id, 42);
        assert_eq!(response.motd.name, "TorchFlower Native Test");
        assert_eq!(response.motd.protocol, 975);
        assert_eq!(observed.remote_addr.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
}
