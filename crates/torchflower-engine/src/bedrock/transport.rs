use binary_util::interfaces::Writer;
use torchflower_network::{
    client::{Client, DEFAULT_MTU},
    protocol::{
        packet::online::{ConnectedPing, OnlinePacket},
        reliability::Reliability,
    },
};

use crate::{
    bedrock::local_network::info::RAKNET_GAMEPACKET_ID,
    error::{EngineError, EngineResult},
};

pub struct RaknetClientAdapter {
    inner: Client,
}

impl RaknetClientAdapter {
    pub async fn connect(host: &str, port: u16, raknet_version: u8) -> EngineResult<Self> {
        let address = format!("{host}:{port}");
        let mut inner = Client::new(raknet_version, DEFAULT_MTU)
            .with_timeout(raknet_timeout_millis())
            .with_handshake_timeout(7)
            .with_handshake_attempts(5);
        inner
            .connect(address.as_str())
            .await
            .map_err(|err| EngineError::Bedrock(format!("RakNet connect failed: {err}")))?;
        let ping = OnlinePacket::ConnectedPing(ConnectedPing {
            time: current_epoch_millis(),
        })
        .write_to_bytes()
        .map_err(|err| EngineError::Bedrock(format!("encode ConnectedPing: {err}")))?;
        inner
            .send_immediate(ping.as_slice(), Reliability::Unreliable, 0)
            .await
            .map_err(|err| EngineError::Bedrock(format!("RakNet connected ping failed: {err}")))?;
        Ok(Self { inner })
    }

    pub async fn send_game_packet(&self, payload: &[u8]) -> EngineResult<()> {
        let mut frame = Vec::with_capacity(payload.len() + 1);
        frame.push(RAKNET_GAMEPACKET_ID);
        frame.extend_from_slice(payload);
        self.inner
            .send_immediate(&frame, Reliability::ReliableOrd, 0)
            .await
            .map_err(|err| EngineError::Bedrock(format!("RakNet send failed: {err}")))
    }

    pub async fn send_game_packet_queued(&self, payload: &[u8]) -> EngineResult<()> {
        // Bedrock GamePacket traffic uses a single ReliableOrdered channel. torchflower-net assigns
        // ordered indices when a packet enters its queue, not when it is physically sent.
        // Mixing queued movement packets with immediate latency responses can therefore put
        // higher order indices on the wire before lower ones. Use the immediate path here so
        // the wire order always matches the RakNet ordered index.
        self.send_game_packet(payload).await
    }

    pub async fn recv_game_packet(&mut self) -> EngineResult<Vec<u8>> {
        let payload = self
            .inner
            .recv()
            .await
            .map_err(|err| EngineError::Bedrock(format!("RakNet recv failed: {err}")))?;
        match payload.split_first() {
            Some((id, rest)) if *id == RAKNET_GAMEPACKET_ID => Ok(rest.to_vec()),
            Some((id, _)) => {
                tracing::warn!("[TRANSPORT_RECV] unexpected packet id={:#04x}", id);
                Err(EngineError::Bedrock(format!(
                    "unexpected RakNet payload id {id:#04x}"
                )))
            }
            None => Err(EngineError::Bedrock("empty RakNet payload".to_string())),
        }
    }

    pub async fn close(&self) {
        self.inner.close().await;
    }

    pub async fn is_connected(&self) -> bool {
        self.inner.is_connected().await
    }
}

fn raknet_timeout_millis() -> u64 {
    parse_raknet_timeout_millis(
        std::env::var("BEDROCK_RAKNET_TIMEOUT_MILLIS")
            .ok()
            .as_deref(),
    )
}

fn parse_raknet_timeout_millis(value: Option<&str>) -> u64 {
    value
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|millis| *millis >= 5_000)
        .unwrap_or(60_000)
}

fn current_epoch_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raknet_timeout_default_is_milliseconds_not_seconds() {
        assert_eq!(parse_raknet_timeout_millis(None), 60_000);
        assert_eq!(parse_raknet_timeout_millis(Some("20")), 60_000);
        assert_eq!(parse_raknet_timeout_millis(Some("20000")), 20_000);
    }
}
