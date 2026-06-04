use crate::protocol::packet::{offline::OfflinePacket, online::OnlinePacket, RakPacket};
use crate::{rakrs_debug, rakrs_debug_buffers};
#[cfg(feature = "async_std")]
use async_std::net::UdpSocket;
use binary_util::interfaces::Writer;
use std::sync::Arc;
#[cfg(feature = "async_tokio")]
use tokio::net::UdpSocket;

pub async fn send_packet(socket: &Arc<UdpSocket>, packet: RakPacket) -> bool {
    if let Ok(buf) = packet.write_to_bytes() {
        if raknet_trace_enabled() {
            eprintln!(
                "[RR_PACKET_LOG] direction=tx stage=offline packet={} id={} len={} details=\"{}\"",
                packet_name(&packet),
                packet_id(&packet),
                buf.as_slice().len(),
                packet_details(&packet)
            );
        }
        if let Err(e) = socket
            .send_to(buf.as_slice(), socket.peer_addr().unwrap())
            .await
        {
            rakrs_debug!("[CLIENT] Failed sending payload to server! {}", e);
            return false;
        } else {
            rakrs_debug_buffers!(false, "[annon]\n{:?}", buf.as_slice());
            return true;
        }
    } else {
        rakrs_debug!("[CLIENT] Failed writing payload to bytes!");
        return false;
    }
}

fn packet_name(packet: &RakPacket) -> &'static str {
    match packet {
        RakPacket::Offline(OfflinePacket::OpenConnectRequest(_)) => "OpenConnectionRequest1",
        RakPacket::Offline(OfflinePacket::OpenConnectReply(_)) => "OpenConnectionReply1",
        RakPacket::Offline(OfflinePacket::SessionInfoRequest(_)) => "OpenConnectionRequest2",
        RakPacket::Offline(OfflinePacket::SessionInfoReply(_)) => "OpenConnectionReply2",
        RakPacket::Offline(OfflinePacket::IncompatibleProtocolVersion(_)) => "IncompatibleProtocol",
        RakPacket::Offline(OfflinePacket::UnconnectedPing(_)) => "UnconnectedPing",
        RakPacket::Offline(OfflinePacket::UnconnectedPong(_)) => "UnconnectedPong",
        RakPacket::Online(OnlinePacket::ConnectionRequest(_)) => "ConnectionRequest",
        RakPacket::Online(OnlinePacket::ConnectionAccept(_)) => "ConnectionRequestAccepted",
        RakPacket::Online(OnlinePacket::NewConnection(_)) => "NewIncomingConnection",
        RakPacket::Online(OnlinePacket::ConnectedPing(_)) => "ConnectedPing",
        RakPacket::Online(OnlinePacket::ConnectedPong(_)) => "ConnectedPong",
        RakPacket::Online(OnlinePacket::Disconnect(_)) => "Disconnect",
        _ => "Other",
    }
}

fn packet_id(packet: &RakPacket) -> String {
    match packet {
        RakPacket::Offline(OfflinePacket::OpenConnectRequest(_)) => "0x05".to_string(),
        RakPacket::Offline(OfflinePacket::OpenConnectReply(_)) => "0x06".to_string(),
        RakPacket::Offline(OfflinePacket::SessionInfoRequest(_)) => "0x07".to_string(),
        RakPacket::Offline(OfflinePacket::SessionInfoReply(_)) => "0x08".to_string(),
        RakPacket::Offline(OfflinePacket::IncompatibleProtocolVersion(_)) => "0x19".to_string(),
        RakPacket::Offline(OfflinePacket::UnconnectedPing(_)) => "0x01".to_string(),
        RakPacket::Offline(OfflinePacket::UnconnectedPong(_)) => "0x1c".to_string(),
        RakPacket::Online(OnlinePacket::ConnectedPing(_)) => "0x00".to_string(),
        RakPacket::Online(OnlinePacket::ConnectedPong(_)) => "0x03".to_string(),
        RakPacket::Online(OnlinePacket::ConnectionRequest(_)) => "0x09".to_string(),
        RakPacket::Online(OnlinePacket::ConnectionAccept(_)) => "0x10".to_string(),
        RakPacket::Online(OnlinePacket::NewConnection(_)) => "0x13".to_string(),
        RakPacket::Online(OnlinePacket::Disconnect(_)) => "0x15".to_string(),
        _ => "unknown".to_string(),
    }
}

fn packet_details(packet: &RakPacket) -> String {
    match packet {
        RakPacket::Offline(OfflinePacket::OpenConnectRequest(packet)) => {
            format!("protocol={} mtu={}", packet.protocol, packet.mtu_size)
        }
        RakPacket::Offline(OfflinePacket::SessionInfoRequest(packet)) => {
            format!(
                "mtu={} cookie={} address={} client_id={}",
                packet.mtu_size, packet.cookie, packet.address, packet.client_id
            )
        }
        _ => String::new(),
    }
}

fn raknet_trace_enabled() -> bool {
    std::env::var("RAKNET_TRACE")
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
}
