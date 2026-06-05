#![allow(unknown_lints)]

pub mod protocol_version;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression as ZlibLevel};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

pub use protocol_version::ProtocolVersion;

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("buffer ended while decoding {0}")]
    UnexpectedEof(&'static str),
    #[error("invalid UTF-8 string: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported packet for this codec")]
    UnsupportedPacket,
    #[error("compression error: {0}")]
    Compression(String),
}

pub trait PacketCodec: Sized {
    fn encode(&self, version: ProtocolVersion) -> Result<Bytes, ProtoError>;
    fn decode(buf: &mut Bytes, version: ProtocolVersion) -> Result<Self, ProtoError>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Packet {
    RequestNetworkSettings(RequestNetworkSettingsPacket),
    NetworkSettings(NetworkSettingsPacket),
    Login(LoginPacket),
    Text(TextPacket),
    MovePlayer(MovePlayerPacket),
    ModalFormRequest(ModalFormRequest),
    ModalFormResponse(ModalFormResponse),
}

impl Packet {
    pub fn encode(&self, version: ProtocolVersion) -> Result<Bytes, ProtoError> {
        match self {
            Self::RequestNetworkSettings(packet) => packet.encode(version),
            Self::NetworkSettings(packet) => packet.encode(version),
            Self::Login(packet) => packet.encode(version),
            Self::Text(packet) => packet.encode(version),
            Self::MovePlayer(packet) => packet.encode(version),
            Self::ModalFormRequest(packet) => packet.encode(version),
            Self::ModalFormResponse(packet) => packet.encode(version),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestNetworkSettingsPacket {
    pub protocol_version: u32,
}

impl PacketCodec for RequestNetworkSettingsPacket {
    fn encode(&self, _version: ProtocolVersion) -> Result<Bytes, ProtoError> {
        let mut out = BytesMut::new();
        out.put_u32_le(self.protocol_version);
        Ok(out.freeze())
    }

    fn decode(buf: &mut Bytes, _version: ProtocolVersion) -> Result<Self, ProtoError> {
        ensure_remaining(buf, 4, "request network settings protocol version")?;
        Ok(Self {
            protocol_version: buf.get_u32_le(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompressionAlgorithm {
    Zlib,
    Zstd,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkSettingsPacket {
    pub compression_threshold: u16,
    pub compression_algorithm: CompressionAlgorithm,
}

impl PacketCodec for NetworkSettingsPacket {
    fn encode(&self, _version: ProtocolVersion) -> Result<Bytes, ProtoError> {
        let mut out = BytesMut::new();
        out.put_u16_le(self.compression_threshold);
        out.put_u8(match self.compression_algorithm {
            CompressionAlgorithm::Zlib => 0,
            CompressionAlgorithm::Zstd => 2,
            CompressionAlgorithm::None => u8::MAX,
        });
        Ok(out.freeze())
    }

    fn decode(buf: &mut Bytes, _version: ProtocolVersion) -> Result<Self, ProtoError> {
        ensure_remaining(buf, 3, "network settings")?;
        let compression_threshold = buf.get_u16_le();
        let compression_algorithm = match buf.get_u8() {
            0 => CompressionAlgorithm::Zlib,
            2 => CompressionAlgorithm::Zstd,
            u8::MAX => CompressionAlgorithm::None,
            _ => return Err(ProtoError::UnsupportedPacket),
        };
        Ok(Self {
            compression_threshold,
            compression_algorithm,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compression {
    Zlib { level: u32 },
    Zstd { level: i32 },
    None,
}

pub fn should_compress(data: &[u8], threshold: usize) -> bool {
    data.len() >= threshold
}

pub fn compress(
    data: &[u8],
    compression: Compression,
    threshold: usize,
) -> Result<Vec<u8>, ProtoError> {
    if !should_compress(data, threshold) || matches!(compression, Compression::None) {
        return Ok(data.to_vec());
    }

    match compression {
        Compression::Zlib { level } => {
            let mut encoder = ZlibEncoder::new(Vec::new(), ZlibLevel::new(level));
            encoder
                .write_all(data)
                .map_err(|err| ProtoError::Compression(err.to_string()))?;
            encoder
                .finish()
                .map_err(|err| ProtoError::Compression(err.to_string()))
        }
        Compression::Zstd { level } => zstd::bulk::compress(data, level)
            .map_err(|err| ProtoError::Compression(err.to_string())),
        Compression::None => Ok(data.to_vec()),
    }
}

pub fn decompress(data: &[u8], compression: Compression) -> Result<Vec<u8>, ProtoError> {
    match compression {
        Compression::Zlib { .. } => {
            let mut decoder = ZlibDecoder::new(data);
            let mut out = Vec::new();
            decoder
                .read_to_end(&mut out)
                .map_err(|err| ProtoError::Compression(err.to_string()))?;
            Ok(out)
        }
        Compression::Zstd { .. } => {
            zstd::bulk::decompress(data, data.len().saturating_mul(32).max(64))
                .map_err(|err| ProtoError::Compression(err.to_string()))
        }
        Compression::None => Ok(data.to_vec()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginPacket {
    pub protocol_version: u32,
    pub chain_json: String,
    pub client_data_jwt: String,
}

impl PacketCodec for LoginPacket {
    fn encode(&self, _version: ProtocolVersion) -> Result<Bytes, ProtoError> {
        let mut out = BytesMut::new();
        out.put_u32_le(self.protocol_version);
        put_string(&mut out, &self.chain_json);
        put_string(&mut out, &self.client_data_jwt);
        Ok(out.freeze())
    }

    fn decode(buf: &mut Bytes, _version: ProtocolVersion) -> Result<Self, ProtoError> {
        ensure_remaining(buf, 4, "login protocol version")?;
        Ok(Self {
            protocol_version: buf.get_u32_le(),
            chain_json: get_string(buf, "login chain")?,
            client_data_jwt: get_string(buf, "login client data")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPacket {
    pub source: String,
    pub message: String,
}

impl PacketCodec for TextPacket {
    fn encode(&self, _version: ProtocolVersion) -> Result<Bytes, ProtoError> {
        let mut out = BytesMut::new();
        put_string(&mut out, &self.source);
        put_string(&mut out, &self.message);
        Ok(out.freeze())
    }

    fn decode(buf: &mut Bytes, _version: ProtocolVersion) -> Result<Self, ProtoError> {
        Ok(Self {
            source: get_string(buf, "text source")?,
            message: get_string(buf, "text message")?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MovePlayerPacket {
    pub runtime_id: u64,
    pub position: [f32; 3],
    pub pitch: f32,
    pub yaw: f32,
    pub head_yaw: f32,
}

impl PacketCodec for MovePlayerPacket {
    fn encode(&self, _version: ProtocolVersion) -> Result<Bytes, ProtoError> {
        let mut out = BytesMut::new();
        out.put_u64_le(self.runtime_id);
        for component in self.position {
            out.put_f32_le(component);
        }
        out.put_f32_le(self.pitch);
        out.put_f32_le(self.yaw);
        out.put_f32_le(self.head_yaw);
        Ok(out.freeze())
    }

    fn decode(buf: &mut Bytes, _version: ProtocolVersion) -> Result<Self, ProtoError> {
        ensure_remaining(buf, 32, "move player")?;
        Ok(Self {
            runtime_id: buf.get_u64_le(),
            position: [buf.get_f32_le(), buf.get_f32_le(), buf.get_f32_le()],
            pitch: buf.get_f32_le(),
            yaw: buf.get_f32_le(),
            head_yaw: buf.get_f32_le(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModalFormRequest {
    pub form_id: u32,
    pub form_data: FormData,
}

impl PacketCodec for ModalFormRequest {
    fn encode(&self, _version: ProtocolVersion) -> Result<Bytes, ProtoError> {
        let mut out = BytesMut::new();
        out.put_u32_le(self.form_id);
        put_string(&mut out, &serde_json::to_string(&self.form_data)?);
        Ok(out.freeze())
    }

    fn decode(buf: &mut Bytes, _version: ProtocolVersion) -> Result<Self, ProtoError> {
        ensure_remaining(buf, 4, "modal form id")?;
        let form_id = buf.get_u32_le();
        let form_json = get_string(buf, "modal form json")?;
        Ok(Self {
            form_id,
            form_data: serde_json::from_str(&form_json)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModalFormResponse {
    pub form_id: u32,
    pub response_data: Option<serde_json::Value>,
}

impl PacketCodec for ModalFormResponse {
    fn encode(&self, _version: ProtocolVersion) -> Result<Bytes, ProtoError> {
        let mut out = BytesMut::new();
        out.put_u32_le(self.form_id);
        put_string(
            &mut out,
            &self
                .response_data
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?
                .unwrap_or_default(),
        );
        Ok(out.freeze())
    }

    fn decode(buf: &mut Bytes, _version: ProtocolVersion) -> Result<Self, ProtoError> {
        ensure_remaining(buf, 4, "modal response id")?;
        let form_id = buf.get_u32_le();
        let response = get_string(buf, "modal response json")?;
        Ok(Self {
            form_id,
            response_data: if response.is_empty() {
                None
            } else {
                Some(serde_json::from_str(&response)?)
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FormData {
    Simple(SimpleForm),
    Custom(CustomForm),
    Modal(ModalForm),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimpleForm {
    pub title: String,
    pub content: String,
    pub buttons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomForm {
    pub title: String,
    pub inputs: Vec<CustomFormInput>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CustomFormInput {
    Dropdown {
        text: String,
        options: Vec<String>,
    },
    Slider {
        text: String,
        min: f64,
        max: f64,
        step: f64,
    },
    TextField {
        text: String,
        placeholder: String,
    },
    Toggle {
        text: String,
        default: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModalForm {
    pub title: String,
    pub content: String,
    pub button1: String,
    pub button2: String,
}

fn put_string(out: &mut BytesMut, value: &str) {
    out.put_u32_le(value.len() as u32);
    out.extend_from_slice(value.as_bytes());
}

fn get_string(buf: &mut Bytes, field: &'static str) -> Result<String, ProtoError> {
    ensure_remaining(buf, 4, field)?;
    let len = buf.get_u32_le() as usize;
    ensure_remaining(buf, len, field)?;
    let bytes = buf.copy_to_bytes(len).to_vec();
    Ok(String::from_utf8(bytes)?)
}

fn ensure_remaining(buf: &Bytes, len: usize, field: &'static str) -> Result<(), ProtoError> {
    if buf.remaining() < len {
        return Err(ProtoError::UnexpectedEof(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T>(packet: T) -> T
    where
        T: PacketCodec + PartialEq + std::fmt::Debug,
    {
        let mut bytes = packet.encode(ProtocolVersion::V1_21_100).unwrap();
        let decoded = T::decode(&mut bytes, ProtocolVersion::V1_21_100).unwrap();
        assert!(bytes.is_empty());
        assert_eq!(decoded, packet);
        decoded
    }

    #[test]
    fn login_round_trip() {
        round_trip(LoginPacket {
            protocol_version: 766,
            chain_json: "{\"chain\":[]}".to_string(),
            client_data_jwt: "client.jwt".to_string(),
        });
    }

    #[test]
    fn network_settings_round_trip() {
        round_trip(RequestNetworkSettingsPacket {
            protocol_version: 766,
        });
        round_trip(NetworkSettingsPacket {
            compression_threshold: 256,
            compression_algorithm: CompressionAlgorithm::Zstd,
        });
    }

    #[test]
    fn text_round_trip() {
        round_trip(TextPacket {
            source: "bot".to_string(),
            message: "hello".to_string(),
        });
    }

    #[test]
    fn move_player_round_trip() {
        round_trip(MovePlayerPacket {
            runtime_id: 42,
            position: [1.0, 2.0, 3.0],
            pitch: 4.0,
            yaw: 5.0,
            head_yaw: 6.0,
        });
    }

    #[test]
    fn modal_form_request_round_trip() {
        round_trip(ModalFormRequest {
            form_id: 7,
            form_data: FormData::Simple(SimpleForm {
                title: "Title".to_string(),
                content: "Content".to_string(),
                buttons: vec!["OK".to_string()],
            }),
        });
    }

    #[test]
    fn modal_form_response_round_trip() {
        round_trip(ModalFormResponse {
            form_id: 7,
            response_data: Some(serde_json::json!({ "accepted": true })),
        });
    }

    #[test]
    fn compression_threshold_is_honored() {
        assert!(!should_compress(&[1, 2, 3], 4));
        assert!(should_compress(&[1, 2, 3, 4], 4));
    }

    #[test]
    fn zlib_and_zstd_round_trip() {
        let data = b"TorchFlower compression payload".repeat(32);
        for compression in [
            Compression::Zlib { level: 1 },
            Compression::Zstd { level: 1 },
        ] {
            let compressed = compress(&data, compression, 1).unwrap();
            let decompressed = decompress(&compressed, compression).unwrap();
            assert_eq!(decompressed, data);
        }
    }
}
