use bedrock::network::info::RAKNET_GAMEPACKET_ID;
use rak_rs::{
    client::{Client, DEFAULT_MTU},
    protocol::reliability::Reliability,
};

use crate::error::{EngineError, EngineResult};

pub struct RaknetClientAdapter {
    inner: Client,
}

impl RaknetClientAdapter {
    pub async fn connect(host: &str, port: u16, raknet_version: u8) -> EngineResult<Self> {
        let address = format!("{host}:{port}");
        let mut inner = Client::new(raknet_version, DEFAULT_MTU)
            .with_timeout(20)
            .with_handshake_timeout(7)
            .with_handshake_attempts(5);
        inner
            .connect(address.as_str())
            .await
            .map_err(|err| EngineError::Bedrock(format!("RakNet connect failed: {err}")))?;
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

    pub async fn recv_game_packet(&mut self) -> EngineResult<Vec<u8>> {
        let payload = self
            .inner
            .recv()
            .await
            .map_err(|err| EngineError::Bedrock(format!("RakNet recv failed: {err}")))?;
        match payload.split_first() {
            Some((id, rest)) if *id == RAKNET_GAMEPACKET_ID => Ok(rest.to_vec()),
            Some((id, _)) => Err(EngineError::Bedrock(format!(
                "unexpected RakNet payload id {id:#04x}"
            ))),
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
