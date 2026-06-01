use bedrock::{
    network::{codec, compression::Compression, encryption::Encryption},
    protocol::{
        ProtoVersion, Unknown, V975,
        unknown::packets::RequestNetworkSettingsPacket as UnknownRequestNetworkSettingsPacket,
        v662::{enums::PacketCompressionAlgorithm, packets::LoginPacket},
    },
};

use crate::{
    auth::{ProvisionedBedrockSession, minecraft::MinecraftAuth},
    bedrock::transport::RaknetClientAdapter,
    error::{EngineError, EngineResult},
};

pub struct BedrockProtocolAdapter {
    transport: RaknetClientAdapter,
    compression: Option<Compression>,
    encryption: Option<Encryption>,
}

impl BedrockProtocolAdapter {
    pub async fn connect(host: &str, port: u16) -> EngineResult<Self> {
        Ok(Self {
            transport: RaknetClientAdapter::connect(host, port, V975::RAKNET_VERSION).await?,
            compression: None,
            encryption: None,
        })
    }

    pub async fn request_network_settings(&mut self) -> EngineResult<()> {
        let packet =
            Unknown::RequestNetworkSettingsPacket(Box::new(UnknownRequestNetworkSettingsPacket {
                client_network_version: V975::PROTOCOL_VERSION as i32,
            }));
        let payload = codec::encode_packets::<Unknown>(&[packet], None, None)
            .map_err(|err| EngineError::Bedrock(format!("encode RequestNetworkSettings: {err}")))?;
        self.transport.send_game_packet(&payload).await?;
        let response = self.transport.recv_game_packet().await?;
        let packets = codec::decode_packets::<V975>(response, None, None)
            .map_err(|err| EngineError::Bedrock(format!("decode NetworkSettings: {err}")))?;
        for packet in packets {
            if let V975::NetworkSettingsPacket(settings) = packet {
                self.compression = Some(match settings.compression_algorithm {
                    PacketCompressionAlgorithm::ZLib => Compression::Zlib {
                        threshold: settings.compression_threshold,
                        compression_level: 6,
                    },
                    PacketCompressionAlgorithm::Snappy => Compression::Snappy {
                        threshold: settings.compression_threshold,
                    },
                    PacketCompressionAlgorithm::None => Compression::None,
                });
                return Ok(());
            }
        }
        Err(EngineError::Bedrock(
            "server did not return NetworkSettingsPacket".to_string(),
        ))
    }

    pub async fn send_login(&mut self, session: &ProvisionedBedrockSession) -> EngineResult<()> {
        let connection_request = MinecraftAuth::connection_request(&session.chain)?;
        let packet = V975::LoginPacket(Box::new(LoginPacket {
            client_network_version: V975::PROTOCOL_VERSION as i32,
            connection_request,
        }));
        self.send(&[packet]).await
    }

    pub async fn send(&mut self, packets: &[V975]) -> EngineResult<()> {
        let payload = codec::encode_packets::<V975>(
            packets,
            self.compression.as_ref(),
            self.encryption.as_mut(),
        )
        .map_err(|err| EngineError::Bedrock(format!("encode packet batch: {err}")))?;
        self.transport.send_game_packet(&payload).await
    }

    pub async fn recv(&mut self) -> EngineResult<Vec<V975>> {
        let payload = self.transport.recv_game_packet().await?;
        codec::decode_packets::<V975>(payload, self.compression.as_ref(), self.encryption.as_mut())
            .map_err(|err| EngineError::Bedrock(format!("decode packet batch: {err}")))
    }

    pub async fn close(&self) {
        self.transport.close().await;
    }
}
