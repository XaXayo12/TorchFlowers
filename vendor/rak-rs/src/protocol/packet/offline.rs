//!  Offline packets are packets that are sent before a connection is established.
//! In rak-rs, these packets consist of:
//! - [`UnconnectedPing`]
//! - [`UnconnectedPong`]
//! - [`OpenConnectRequest`]
//! - [`OpenConnectReply`]
//! - [`SessionInfoRequest`]
//! - [`SessionInfoReply`]
//! - [`IncompatibleProtocolVersion`]
//!
//! During this stage, the client and server are exchanging information about each other, such as
//! the server id, the client id, the mtu size, etc, to prepare for the connection handshake.
use std::net::{SocketAddr, Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};

use super::RakPacket;
#[cfg(feature = "mcpe")]
pub use crate::protocol::mcpe::UnconnectedPong;
use crate::protocol::Magic;
use crate::protocol::RAKNET_HEADER_FRAME_OVERHEAD;
use crate::register_packets;

use binary_util::interfaces::{Reader, Writer};
use binary_util::io::{ByteReader, ByteWriter};
use binary_util::BinaryIo;

/// This is an enum of all offline packets.
///
/// You can use this to read and write offline packets,
/// with the `binary_util` traits `Reader` and `Writer`.
#[derive(Clone, Debug, BinaryIo)]
#[repr(u8)]
pub enum OfflinePacket {
    UnconnectedPing(UnconnectedPing) = 0x01,
    UnconnectedPong(UnconnectedPong) = 0x1c,
    OpenConnectRequest(OpenConnectRequest) = 0x05,
    OpenConnectReply(OpenConnectReply) = 0x06,
    SessionInfoRequest(SessionInfoRequest) = 0x07,
    SessionInfoReply(SessionInfoReply) = 0x08,
    IncompatibleProtocolVersion(IncompatibleProtocolVersion) = 0x19,
}

register_packets! {
    Offline is OfflinePacket,
    UnconnectedPing,
    UnconnectedPong,
    OpenConnectRequest,
    OpenConnectReply,
    SessionInfoRequest,
    SessionInfoReply,
    IncompatibleProtocolVersion
}

/// Send to the other peer expecting a [`UnconnectedPong`] packet,
/// this is used to determine the latency between the client and the server,
/// and to determine if the server is online.
///
/// If the peer does not respond with a [`UnconnectedPong`] packet, the iniatior should
/// expect that the server is offline.
#[derive(Debug, Clone, BinaryIo)]
pub struct UnconnectedPing {
    pub timestamp: u64,
    pub magic: Magic,
    pub client_id: i64,
}

/// Sent in response to a [`UnconnectedPing`] packet.
/// This is used to determine the latency between the client and the server, and to determine
/// that the peer is online.
///
/// <style>
/// .warning-2 {
///     background: rgba(255,240,76,0.34) !important;
///     padding: 0.75em;
///     border-left: 2px solid #fce811;
///     font-family: "Source Serif 4", NanumBarunGothic, serif;
///  }
///
/// .warning-2 code {
///     background: rgba(211,201,88,0.64) !important;
/// }
///
/// .notice-2 {
///     background: rgba(88, 211, 255, 0.34) !important;
///     padding: 0.75em;
///     border-left: 2px solid #4c96ff;
///     font-family: "Source Serif 4", NanumBarunGothic, serif;
/// }
///
/// .notice-2 code {
///     background: rgba(88, 211, 255, 0.64) !important;
/// }
/// </style>
/// <div class="notice-2">
///     <strong> Note: </strong>
///    <p>
///         If the client is a Minecraft: Bedrock Edition client, this packet is not sent
///         and the
///         <a
///             href="/rak-rs/latest/protocol/mcpe/struct.UnconnectedPong.html"
///             title="struct rak_rs::protocol::mcpe::UnconnectedPing">
///             UnconnectedPong
///         </a>
///         from the <code>mcpe</code> module is sent instead.
///   </p>
/// </div>
///
/// [`UnconnectedPong`]: crate::protocol::packet::offline::UnconnectedPong
#[cfg(not(feature = "mcpe"))]
#[derive(Debug, Clone, BinaryIo)]
pub struct UnconnectedPong {
    pub timestamp: u64,
    pub server_id: u64,
    pub magic: Magic,
}

/// This packet is the equivelant of the `OpenConnectRequest` packet in RakNet.
///
/// This packet is sent by the peer to a server to request a connection.
/// It contains information about the client, such as the protocol version, and the mtu size.
/// The peer should expect a [`OpenConnectReply`] packet in response to this packet, if the
/// server accepts the connection. Otherwise, the peer should expect a [`IncompatibleProtocolVersion`]
/// packet to be sent to indicate that the server does not support the protocol version.
///
/// <style>
/// .warning-2 {
///     background: rgba(255,240,76,0.34) !important;
///     padding: 0.75em;
///     border-left: 2px solid #fce811;
///     font-family: "Source Serif 4", NanumBarunGothic, serif;
///  }
///
/// .warning-2 code {
///     background: rgba(211,201,88,0.64) !important;
/// }
///
/// .notice-2 {
///     background: rgba(88, 211, 255, 0.34) !important;
///     padding: 0.75em;
///     border-left: 2px solid #4c96ff;
///     font-family: "Source Serif 4", NanumBarunGothic, serif;
/// }
///
/// .notice-2 code {
///     background: rgba(88, 211, 255, 0.64) !important;
/// }
/// </style>
/// <div class="notice-2">
///     <strong> Note: </strong>
///    <p>
///         Internally this packet is padded by the given
///         <code>mtu_size</code> in the packet. This is done by appending null bytes
///         to the current buffer of the packet which is calculated by adding the difference
///         between the <code>mtu_size</code> and the current length.
///   </p>
/// </div>
#[derive(Debug, Clone)]
pub struct OpenConnectRequest {
    pub protocol: u8,  // 9
    pub mtu_size: u16, // 500
}

impl Reader<OpenConnectRequest> for OpenConnectRequest {
    fn read(buf: &mut ByteReader) -> Result<OpenConnectRequest, std::io::Error> {
        let len = buf.as_slice().len();
        buf.read_type::<Magic>()?;
        Ok(OpenConnectRequest {
            protocol: buf.read_u8()?,
            mtu_size: (len + RAKNET_HEADER_FRAME_OVERHEAD as usize) as u16,
        })
    }
}

impl Writer for OpenConnectRequest {
    fn write(&self, buf: &mut ByteWriter) -> Result<(), std::io::Error> {
        buf.write_type::<Magic>(&Magic::new())?;
        buf.write_u8(self.protocol)?;
        // padding
        // remove 28 bytes from the mtu size
        let mtu_size = self.mtu_size - RAKNET_HEADER_FRAME_OVERHEAD as u16;
        for _ in 0..mtu_size {
            buf.write_u8(0)?;
        }
        Ok(())
    }
}

// Open Connection Reply
/// This packet is sent in response to a [`OpenConnectRequest`] packet, and confirms
/// the information sent by the peer in the [`OpenConnectRequest`] packet.
///
/// This packet is the equivalent of the `Open Connect Reply 1` within the original RakNet implementation.
///
/// If the server chooses to deny the connection, it should send a [`IncompatibleProtocolVersion`]
/// or ignore the packet.
#[derive(Debug, Clone)]
pub struct OpenConnectReply {
    pub magic: Magic,
    pub server_id: u64,
    pub security: bool,
    pub cookie: u32,
    pub mtu_size: u16,
}

impl Reader<OpenConnectReply> for OpenConnectReply {
    fn read(buf: &mut ByteReader) -> Result<OpenConnectReply, std::io::Error> {
        let magic = Magic::read(buf)?;
        let server_id = buf.read_u64()?;
        let security = buf.read_bool()?;
        let mut cookie = 0u32;
        if security {
            cookie = buf.read_u32()?;
        }
        let mtu_size = buf.read_u16()?;
        Ok(OpenConnectReply {
            magic,
            server_id,
            security,
            cookie,
            mtu_size,
        })
    }
}

impl Writer for OpenConnectReply {
    fn write(&self, buf: &mut ByteWriter) -> Result<(), std::io::Error> {
        self.magic.write(buf)?;
        buf.write_u64(self.server_id)?;
        buf.write_bool(self.security)?;
        if self.security {
            buf.write_u32(self.cookie)?;
        }
        buf.write_u16(self.mtu_size)?;
        Ok(())
    }
}

/// This packet is sent after receiving a [`OpenConnectReply`] packet, and confirms
/// that the peer wishes to proceed with the connection. The information within this packet
/// is primarily used to get the external address of the peer.
///
/// This packet is the equivalent of the `Open Connect Request 2` within the original RakNet implementation.
fn write_inverted_addr(addr: &SocketAddr, buf: &mut ByteWriter) -> Result<(), std::io::Error> {
    match addr {
        SocketAddr::V4(addr) => {
            buf.write_u8(4)?;
            let octets = addr.ip().octets();
            let inverted: [u8; 4] = [
                !octets[0],
                !octets[1],
                !octets[2],
                !octets[3],
            ];
            buf.write(&inverted)?;
            buf.write_u16(addr.port())?;
        }
        SocketAddr::V6(addr) => {
            buf.write_u8(6)?;
            buf.write_u16(0)?; // family
            buf.write_u16(addr.port())?;
            buf.write_u32(addr.flowinfo())?;
            let octets = addr.ip().octets();
            let mut inverted = [0u8; 16];
            for i in 0..16 {
                inverted[i] = !octets[i];
            }
            buf.write(&inverted)?;
            buf.write_u32(addr.scope_id())?;
        }
    }
    Ok(())
}

fn read_inverted_addr(buf: &mut ByteReader) -> Result<SocketAddr, std::io::Error> {
    match buf.read_u8()? {
        4 => {
            let mut octets = [0u8; 4];
            octets[0] = !buf.read_u8()?;
            octets[1] = !buf.read_u8()?;
            octets[2] = !buf.read_u8()?;
            octets[3] = !buf.read_u8()?;
            let port = buf.read_u16()?;
            Ok(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]),
                port,
            )))
        }
        6 => {
            let _family = buf.read_u16()?;
            let port = buf.read_u16()?;
            let flow = buf.read_u32()?;
            let mut octets = [0u8; 16];
            for i in 0..16 {
                octets[i] = !buf.read_u8()?;
            }
            let scope = buf.read_u32()?;
            Ok(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(octets),
                port,
                flow,
                scope,
            )))
        }
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Invalid IP version",
        )),
    }
}

#[derive(Debug, Clone)]
pub struct SessionInfoRequest {
    pub magic: Magic,
    pub cookie: u32,
    pub address: SocketAddr,
    pub mtu_size: u16,
    pub client_id: i64,
}

impl Reader<SessionInfoRequest> for SessionInfoRequest {
    fn read(buf: &mut ByteReader) -> Result<SessionInfoRequest, std::io::Error> {
        let magic = Magic::read(buf)?;
        let address = read_inverted_addr(buf)?;
        let mtu_size = buf.read_u16()?;
        let client_id = buf.read_i64()?;
        Ok(SessionInfoRequest {
            magic,
            cookie: 0,
            address,
            mtu_size,
            client_id,
        })
    }
}

impl Writer for SessionInfoRequest {
    fn write(&self, buf: &mut ByteWriter) -> Result<(), std::io::Error> {
        self.magic.write(buf)?;
        if self.cookie != 0 {
            buf.write_u32(self.cookie)?;
            buf.write_bool(false)?; // client supports security = false
        }
        write_inverted_addr(&self.address, buf)?;
        buf.write_u16(self.mtu_size)?;
        buf.write_i64(self.client_id)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SessionInfoReply {
    pub magic: Magic,
    pub server_id: u64,
    pub client_address: SocketAddr,
    pub mtu_size: u16,
    pub security: bool,
}

impl Reader<SessionInfoReply> for SessionInfoReply {
    fn read(buf: &mut ByteReader) -> Result<SessionInfoReply, std::io::Error> {
        let magic = Magic::read(buf)?;
        let server_id = buf.read_u64()?;
        let client_address = read_inverted_addr(buf)?;
        let mtu_size = buf.read_u16()?;
        let security = buf.read_bool()?;
        Ok(SessionInfoReply {
            magic,
            server_id,
            client_address,
            mtu_size,
            security,
        })
    }
}

impl Writer for SessionInfoReply {
    fn write(&self, buf: &mut ByteWriter) -> Result<(), std::io::Error> {
        self.magic.write(buf)?;
        buf.write_u64(self.server_id)?;
        write_inverted_addr(&self.client_address, buf)?;
        buf.write_u16(self.mtu_size)?;
        buf.write_bool(self.security)?;
        Ok(())
    }
}

/// This packet is sent by the server to indicate that the server does not support the
/// protocol version of the client.
#[derive(Debug, Clone, BinaryIo)]
pub struct IncompatibleProtocolVersion {
    pub protocol: u8,
    pub magic: Magic,
    pub server_id: u64,
}
