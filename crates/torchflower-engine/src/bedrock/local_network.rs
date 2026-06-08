//! Local Bedrock packet stream helpers used by the engine.
//!
//! TorchFlower keeps packet definitions on `bedrock-rs`, but avoids depending on
//! its `network` feature because that currently pulls in a broken crates.io
//! `rak-rs` transitive dependency. This module preserves the small network API
//! surface the engine needs: batch framing, negotiated compression, and Bedrock
//! AES-CTR packet encryption.

pub mod info {
    pub const RAKNET_GAMEPACKET_ID: u8 = 0xfe;
}

pub mod error {
    use thiserror::Error;

    #[derive(Debug, Error)]
    pub enum NetworkCodecError {
        #[error("proto codec error: {0}")]
        Proto(#[from] torchflower_protocol_core::CoreError),
        #[error("compression error: {0}")]
        Compression(#[from] super::compression::CompressionError),
        #[error("encryption error: {0}")]
        Encryption(#[from] super::encryption::EncryptionError),
        #[error("I/O error: {0}")]
        Io(#[from] std::io::Error),
    }
}

pub mod compression {
    use flate2::{read::DeflateDecoder, write::DeflateEncoder, Compression as FlateCompression};
    use snap::{read::FrameDecoder as SnapDecoder, write::FrameEncoder as SnapEncoder};
    use std::io::{Cursor, Read, Write};
    use thiserror::Error;

    #[derive(Debug, Clone)]
    pub enum Compression {
        Zlib {
            threshold: u16,
            compression_level: u8,
        },
        Snappy {
            threshold: u16,
        },
        None,
    }

    #[derive(Debug, Error)]
    pub enum CompressionError {
        #[error("I/O error: {0}")]
        Io(#[from] std::io::Error),
        #[error("zlib error: {0}")]
        Zlib(std::io::Error),
        #[error("snappy error: {0}")]
        Snappy(std::io::Error),
        #[error("unknown compression method {0}")]
        UnknownMethod(u8),
    }

    impl Compression {
        const ID_ZLIB: u8 = 0;
        const ID_SNAPPY: u8 = 1;
        const ID_NONE: u8 = u8::MAX;

        pub fn threshold(&self) -> u16 {
            match self {
                Self::Zlib { threshold, .. } | Self::Snappy { threshold } => *threshold,
                Self::None => 0,
            }
        }

        pub fn compress(&self, src: Vec<u8>) -> Result<Vec<u8>, CompressionError> {
            let mut dst = Vec::with_capacity(src.len() + 1);

            if src.len() < self.threshold() as usize {
                dst.push(Self::ID_NONE);
                dst.extend_from_slice(&src);
                return Ok(dst);
            }

            match self {
                Self::Zlib {
                    compression_level, ..
                } => {
                    dst.push(Self::ID_ZLIB);
                    let mut encoder = DeflateEncoder::new(
                        dst,
                        FlateCompression::new((*compression_level).into()),
                    );
                    encoder.write_all(&src).map_err(CompressionError::Zlib)?;
                    encoder.finish().map_err(CompressionError::Zlib)
                }
                Self::Snappy { .. } => {
                    dst.push(Self::ID_SNAPPY);
                    let mut encoder = SnapEncoder::new(dst);
                    encoder.write_all(&src).map_err(CompressionError::Snappy)?;
                    encoder
                        .into_inner()
                        .map_err(|err| CompressionError::Snappy(err.into_error()))
                }
                Self::None => {
                    dst.push(Self::ID_NONE);
                    dst.extend_from_slice(&src);
                    Ok(dst)
                }
            }
        }

        pub fn decompress(&self, src: Vec<u8>) -> Result<Vec<u8>, CompressionError> {
            let mut stream = Cursor::new(src.as_slice());
            let mut compression_method = [0u8; 1];
            stream.read_exact(&mut compression_method)?;
            let payload = &src[1..];

            match compression_method[0] {
                Self::ID_ZLIB => {
                    let mut dst = Vec::with_capacity(payload.len());
                    let mut decoder = DeflateDecoder::new(payload);
                    decoder.read_to_end(&mut dst)?;
                    Ok(dst)
                }
                Self::ID_SNAPPY => {
                    let mut dst = Vec::with_capacity(payload.len());
                    let mut decoder = SnapDecoder::new(payload);
                    decoder.read_to_end(&mut dst)?;
                    Ok(dst)
                }
                Self::ID_NONE => Ok(payload.to_vec()),
                other => Err(CompressionError::UnknownMethod(other)),
            }
        }
    }
}

pub mod encryption {
    use aes::Aes256;
    use ctr::{
        cipher::{KeyIvInit, StreamCipher},
        Ctr128BE,
    };
    use p384::{PublicKey, SecretKey};
    use sha2::{Digest, Sha256};
    use thiserror::Error;

    #[derive(Debug)]
    pub struct Encryption {
        encrypt_counter: u64,
        encrypt_cipher: Ctr128BE<Aes256>,
        decrypt_counter: u64,
        decrypt_cipher: Ctr128BE<Aes256>,
        key: [u8; 32],
    }

    #[derive(Debug, Error)]
    pub enum EncryptionError {
        #[error("encrypted packet is too short: {0} bytes")]
        InvalidLength(usize),
        #[error("encrypted packet trailer did not match expected digest")]
        InvalidTrailer,
    }

    impl Encryption {
        pub fn new(secret: &SecretKey, public: &PublicKey, token: &[u8; 16]) -> Self {
            let shared = secret.diffie_hellman(public);
            let shared_bytes = shared.raw_secret_bytes();

            let mut hasher = Sha256::new();
            hasher.update(token);
            hasher.update(shared_bytes);
            let key = hasher.finalize();

            let mut iv = [0u8; 16];
            iv[..12].copy_from_slice(&key[..12]);
            iv[15] = 2;

            let encrypt_cipher = Ctr128BE::<Aes256>::new(&key, (&iv).into());
            let decrypt_cipher = Ctr128BE::<Aes256>::new(&key, (&iv).into());

            Self {
                encrypt_counter: 0,
                encrypt_cipher,
                decrypt_counter: 0,
                decrypt_cipher,
                key: key.into(),
            }
        }

        pub fn encrypt(&mut self, buf: Vec<u8>) -> Result<Vec<u8>, EncryptionError> {
            let trailer = self.trailer(&buf, self.encrypt_counter);
            let mut out = Vec::with_capacity(buf.len() + trailer.len());
            out.extend_from_slice(&buf);
            out.extend_from_slice(&trailer);
            self.encrypt_cipher.apply_keystream(&mut out);
            self.encrypt_counter = self.encrypt_counter.wrapping_add(1);
            Ok(out)
        }

        pub fn decrypt(&mut self, buf: Vec<u8>) -> Result<Vec<u8>, EncryptionError> {
            if buf.len() <= 8 {
                return Err(EncryptionError::InvalidLength(buf.len()));
            }

            let mut out = buf;
            self.decrypt_cipher.apply_keystream(&mut out);

            let trailer_offset = out.len() - 8;
            let trailer = &out[trailer_offset..];
            let expected_trailer = self.trailer(&out[..trailer_offset], self.decrypt_counter);
            if trailer != expected_trailer {
                return Err(EncryptionError::InvalidTrailer);
            }

            self.decrypt_counter = self.decrypt_counter.wrapping_add(1);
            out.truncate(trailer_offset);
            Ok(out)
        }

        pub fn trailer(&self, buf: &[u8], counter: u64) -> [u8; 8] {
            let mut hasher = Sha256::new();
            hasher.update(counter.to_le_bytes());
            hasher.update(buf);
            hasher.update(self.key);
            let hash = hasher.finalize();

            let mut trailer = [0u8; 8];
            trailer.copy_from_slice(&hash[..8]);
            trailer
        }
    }
}

pub mod codec {
    use super::{compression::Compression, encryption::Encryption, error::NetworkCodecError};
    use bytes::{BufMut, Bytes, BytesMut};
    use std::io::Cursor;
    use torchflower_protocol::{Packet, ProtocolVersion};
    use torchflower_protocol_core::{get_var_u32, put_var_u32};

    pub fn encode_packets(
        packets: &[Packet],
        compression: Option<&Compression>,
        encryption: Option<&mut Encryption>,
        version: ProtocolVersion,
    ) -> Result<Vec<u8>, NetworkCodecError> {
        let mut packet_stream = Vec::new();
        for packet in packets {
            let payload = packet
                .encode(version)
                .map_err(|e| NetworkCodecError::Proto(e))?;

            let mut packet_buf = BytesMut::new();
            put_var_u32(&mut packet_buf, packet.id());
            packet_buf.put_slice(&payload);

            put_var_u32(&mut packet_stream, packet_buf.len() as u32);
            packet_stream.extend_from_slice(&packet_buf);
        }

        let packet_stream = compress_packets(packet_stream, compression)?;
        encrypt_packets(packet_stream, encryption)
    }

    pub fn decode_packets(
        packet_stream: Vec<u8>,
        compression: Option<&Compression>,
        encryption: Option<&mut Encryption>,
        version: ProtocolVersion,
    ) -> Result<Vec<Packet>, NetworkCodecError> {
        let packet_stream = decrypt_packets(packet_stream, encryption)?;
        let packet_stream = decompress_packets(packet_stream, compression)?;
        separate_packets(packet_stream, version)
    }

    fn separate_packets(
        packet_stream: Vec<u8>,
        version: ProtocolVersion,
    ) -> Result<Vec<Packet>, NetworkCodecError> {
        let mut cursor = Cursor::new(packet_stream.as_slice());
        let mut packets = Vec::new();

        while cursor.position() < cursor.get_ref().len() as u64 {
            let buf_len =
                get_var_u32(&mut cursor).map_err(|e| NetworkCodecError::Proto(e))? as usize;
            let start = cursor.position() as usize;
            let end = start.saturating_add(buf_len);
            if end > cursor.get_ref().len() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!(
                        "packet length {} exceeds remaining {} bytes",
                        buf_len,
                        cursor.get_ref().len().saturating_sub(start)
                    ),
                )
                .into());
            }

            let mut packet_data = &cursor.get_ref()[start..end];
            let id = get_var_u32(&mut packet_data).map_err(|e| NetworkCodecError::Proto(e))?;
            let mut payload = Bytes::copy_from_slice(packet_data);

            let decoded = Packet::decode(id, &mut payload, version)
                .map_err(|e| NetworkCodecError::Proto(e))?;
            packets.push(decoded);
            cursor.set_position(end as u64);
        }

        Ok(packets)
    }

    pub fn compress_packets(
        packet_stream: Vec<u8>,
        compression: Option<&Compression>,
    ) -> Result<Vec<u8>, NetworkCodecError> {
        match compression {
            Some(compression) => compression.compress(packet_stream).map_err(Into::into),
            None => Ok(packet_stream),
        }
    }

    pub fn decompress_packets(
        packet_stream: Vec<u8>,
        compression: Option<&Compression>,
    ) -> Result<Vec<u8>, NetworkCodecError> {
        match compression {
            Some(compression) => compression.decompress(packet_stream).map_err(Into::into),
            None => Ok(packet_stream),
        }
    }

    pub fn encrypt_packets(
        packet_stream: Vec<u8>,
        encryption: Option<&mut Encryption>,
    ) -> Result<Vec<u8>, NetworkCodecError> {
        match encryption {
            Some(encryption) => encryption.encrypt(packet_stream).map_err(Into::into),
            None => Ok(packet_stream),
        }
    }

    pub fn decrypt_packets(
        packet_stream: Vec<u8>,
        encryption: Option<&mut Encryption>,
    ) -> Result<Vec<u8>, NetworkCodecError> {
        match encryption {
            Some(encryption) => encryption.decrypt(packet_stream).map_err(Into::into),
            None => Ok(packet_stream),
        }
    }
}
