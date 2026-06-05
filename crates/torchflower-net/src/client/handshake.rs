use crate::client::discovery;
use crate::client::discovery::DiscoveryStatus;
use crate::client::discovery::MtuDiscovery;
use crate::client::util::send_packet;
use crate::connection::queue::send::SendQueue;
use crate::connection::queue::RecvQueue;
use crate::protocol::frame::{Frame, FramePacket};
use crate::protocol::packet::offline::{SessionInfoReply, SessionInfoRequest};
use crate::protocol::packet::online::ConnectedPong;
use crate::protocol::packet::online::{ConnectionRequest, NewConnection, OnlinePacket};
use crate::protocol::reliability::Reliability;
use crate::protocol::Magic;
use crate::rakrs_debug;
use crate::server::current_epoch;
#[cfg(feature = "async_std")]
use async_std::{
    future::timeout,
    future::Future,
    net::UdpSocket,
    sync::RwLock,
    task::{self, Context, Poll, Waker},
};
use binary_util::interfaces::{Reader, Writer};
use binary_util::io::ByteReader;
#[cfg(feature = "async_tokio")]
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
#[cfg(feature = "async_tokio")]
use std::task::{Context, Poll, Waker};
use std::time::Duration;
#[cfg(feature = "async_tokio")]
use tokio::{
    net::UdpSocket,
    sync::RwLock,
    task::{self},
    time::timeout,
};

macro_rules! raknet_trace {
    ($($arg:tt)*) => {
        if raknet_trace_enabled() {
            println!($($arg)*);
        }
    };
}

#[macro_export]
macro_rules! match_ids {
    ($socket: expr, $timeout: expr, $($ids: expr),*) => {
        {
            let mut recv_buf: [u8; 2048] = [0; 2048];
            let mut tries: u8 = 0;
            let ids = vec![$($ids),*];
            let mut pk: Option<Vec<u8>> = None;

            'try_conn: loop {
                if (tries >= 5) {
                    break;
                }

                let len: usize;
                let send_result = timeout(
                    Duration::from_secs($timeout),
                    $socket.recv(&mut recv_buf)
                ).await;

                if (send_result.is_err()) {
                    rakrs_debug!(true, "[CLIENT] Failed to receive packet from server! Is it offline?");
                    break 'try_conn;
                }

                match send_result.unwrap() {
                    Err(e) => {
                        tries += 1;
                        rakrs_debug!(true, "[CLIENT] Failed to receive packet from server! {}", e);
                        continue;
                    },
                    Ok(l) => len = l
                };

                crate::rakrs_debug_buffers!(true, "[annon]\n {:?}", &recv_buf[..len]);

                // rakrs_debug!(true, "[CLIENT] Received packet from server: {:x?}", &recv_buf[..len]);

                if ids.contains(&recv_buf[0]) {
                    pk = Some(recv_buf[..len].to_vec());
                    break 'try_conn;
                }
            }

            pk
        }
    };
}

macro_rules! expect_reply {
    ($socket: expr, $reply: ty, $timeout: expr) => {{
        let mut recv_buf: [u8; 2048] = [0; 2048];
        let mut tries: u8 = 0;
        let mut pk: Option<$reply> = None;

        loop {
            if (tries >= 5) {
                break;
            }

            let len: usize;
            let send_result =
                timeout(Duration::from_secs($timeout), $socket.recv(&mut recv_buf)).await;

            if (send_result.is_err()) {
                rakrs_debug!(
                    true,
                    "[CLIENT] Failed to receive packet from server! Is it offline?"
                );
                break;
            }

            match send_result.unwrap() {
                Err(_) => {
                    tries += 1;
                    continue;
                }
                Ok(l) => len = l,
            };

            let packet_id = recv_buf[0];

            // Some servers wrap offline packets in a RakNet FrameSet envelope
            // (outer ids 0x80â€“0x8d). Unwrap the first frame body so that the
            // actual offline-packet id (0x08 = SessionInfoReply, 0x19 =
            // IncompatibleProtocol) can be checked rather than the FrameSet
            // outer FrameSet byte.
            let effective: Vec<u8>;
            let (inner_buf, inner_id) = if (0x80..=0x8d).contains(&packet_id) {
                use binary_util::interfaces::Reader as _;
                let mut r = ByteReader::from(&recv_buf[..len]);
                match FramePacket::read(&mut r) {
                    Ok(fp) => match fp.frames.into_iter().next() {
                        Some(f) => {
                            effective = f.body;
                            let id = effective[0];
                            (effective.as_slice(), id)
                        }
                        None => continue,
                    },
                    Err(_) => continue,
                }
            } else {
                effective = recv_buf[..len].to_vec();
                (effective.as_slice(), packet_id)
            };

            raknet_trace!(
                "[RR_PACKET_LOG] direction=rx stage=offline packet={} outer_id={:#04x} inner_id={:#04x} len={}",
                offline_packet_name(inner_id),
                packet_id,
                inner_id,
                len
            );

            if inner_id != 0x08 && inner_id != 0x19 {
                raknet_trace!(
                    "[RR_PACKET_LOG] direction=rx stage=offline packet=Ignored inner_id={:#04x} len={}",
                    inner_id,
                    len
                );
                continue;
            }

            let mut reader = ByteReader::from(&inner_buf[1..]);

            match <$reply>::read(&mut reader) {
                Ok(packet) => {
                    pk = Some(packet);
                    break;
                }
                Err(err) => {
                    raknet_trace!(
                        "[RR_PACKET_LOG] direction=rx stage=offline packet={} id={:#04x} parse_error={:?}",
                        offline_packet_name(packet_id),
                        packet_id,
                        err
                    );
                }
            }
        }

        pk
    }};
}

macro_rules! update_state {
    ($done: expr, $shared_state: expr, $state: expr) => {{
        let mut state = $shared_state.lock().unwrap();
        state.status = $state;
        state.done = true;
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
        return;
    }};
    ($shared_state: expr, $state: expr) => {{
        let mut state = $shared_state.lock().unwrap();
        state.status = $state;
        state.done = false;
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }};
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HandshakeStatus {
    Created,
    Opening,
    SessionOpen,
    Failed,
    FailedMtuDiscovery,
    FailedNoSessionReply,
    IncompatibleVersion,
    Completed,
}

impl std::fmt::Display for HandshakeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                HandshakeStatus::Created => "Handshake created",
                HandshakeStatus::Opening => "Opening handshake",
                HandshakeStatus::SessionOpen => "Session open",
                HandshakeStatus::Failed => "Handshake failed",
                HandshakeStatus::FailedMtuDiscovery => "MTU discovery failed",
                HandshakeStatus::FailedNoSessionReply => "No session reply",
                HandshakeStatus::IncompatibleVersion => "Incompatible version",
                HandshakeStatus::Completed => "Handshake completed",
            }
        )
    }
}

pub(crate) struct HandshakeState {
    status: HandshakeStatus,
    done: bool,
    waker: Option<Waker>,
}

pub struct ClientHandshake {
    status: Arc<Mutex<HandshakeState>>,
}

fn offline_packet_name(packet_id: u8) -> &'static str {
    match packet_id {
        0x05 => "OpenConnectionRequest1",
        0x06 => "OpenConnectionReply1",
        0x07 => "OpenConnectionRequest2",
        0x08 => "OpenConnectionReply2",
        0x19 => "IncompatibleProtocol",
        _ => "UnknownOffline",
    }
}

fn online_packet_name(packet_id: u8) -> &'static str {
    match packet_id {
        0x00 => "ConnectedPing",
        0x03 => "ConnectedPong",
        0x09 => "ConnectionRequest",
        0x10 => "ConnectionRequestAccepted",
        0x13 => "NewIncomingConnection",
        0x15 => "Disconnect",
        0xfe => "GamePacket",
        _ => "UnknownOnline",
    }
}

fn frame_body_id(frame: &Frame) -> Option<u8> {
    frame.body.first().copied()
}

fn raknet_trace_enabled() -> bool {
    std::env::var("RAKNET_TRACE")
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
}

fn log_frame_packet(direction: &str, packet: &FramePacket) {
    raknet_trace!(
        "[RR_PACKET_LOG] direction={} stage=frame packet=FrameSet sequence={} frame_count={}",
        direction,
        packet.sequence,
        packet.frames.len()
    );

    for (index, frame) in packet.frames.iter().enumerate() {
        let body_id = frame_body_id(frame).unwrap_or(0);
        raknet_trace!(
            "[RR_PACKET_LOG] direction={} stage=frame frame_index={} reliability={:?} body_id={:#04x} packet={} body_len={} reliable_index={:?} sequence_index={:?} order_index={:?} order_channel={:?}",
            direction,
            index,
            frame.reliability,
            body_id,
            online_packet_name(body_id),
            frame.body.len(),
            frame.reliable_index,
            frame.sequence_index,
            frame.order_index,
            frame.order_channel
        );
    }
}

impl ClientHandshake {
    pub fn new(
        socket: Arc<UdpSocket>,
        id: i64,
        version: u8,
        mut mtu: u16,
        _attempts: u8,
        timeout: u16,
        send_queue: Arc<RwLock<SendQueue>>,
    ) -> Self {
        let state = Arc::new(Mutex::new(HandshakeState {
            done: false,
            status: HandshakeStatus::Created,
            waker: None,
        }));

        let shared_state = state.clone();

        task::spawn(async move {
            update_state!(shared_state, HandshakeStatus::Opening);

            rakrs_debug!(true, "[CLIENT] Sending OpenConnectRequest to server...");

            let cookie;
            match MtuDiscovery::new(
                socket.clone(),
                discovery::MtuDiscoveryMeta {
                    id,
                    version,
                    mtu,
                    timeout,
                },
            )
            .await
            {
                DiscoveryStatus::Discovered(m, c) => {
                    rakrs_debug!(
                        true,
                        "[CLIENT] Discovered MTU size: {}, cookie: {:#x}",
                        m,
                        c
                    );
                    mtu = m;
                    cookie = c;
                }
                DiscoveryStatus::IncompatibleVersion => {
                    rakrs_debug!(
                        true,
                        "[CLIENT] Client is using incompatible protocol version."
                    );
                    update_state!(true, shared_state, HandshakeStatus::IncompatibleVersion);
                }
                _ => {
                    update_state!(true, shared_state, HandshakeStatus::FailedMtuDiscovery);
                }
            }

            let session_info = SessionInfoRequest {
                magic: Magic::new(),
                cookie,
                address: socket.peer_addr().unwrap(),
                mtu_size: mtu,
                client_id: id,
            };

            let mut session_reply = None;
            for _attempt in 0..3 {
                if !send_packet(&socket, session_info.clone().into()).await {
                    continue;
                }

                let reply = expect_reply!(socket, SessionInfoReply, 3);
                if reply.is_some() {
                    session_reply = reply;
                    break;
                }
            }

            if session_reply.is_none() {
                rakrs_debug!(true, "[CLIENT] Server did not reply with SessionInfoReply!");
                update_state!(true, shared_state, HandshakeStatus::FailedNoSessionReply);
            }

            let session_reply = session_reply.unwrap();
            raknet_trace!(
                "[RR_PACKET_LOG] direction=rx stage=offline packet=OpenConnectionReply2 id=0x08 mtu={} security={} client_address={} server_id={}",
                session_reply.mtu_size,
                session_reply.security,
                session_reply.client_address,
                session_reply.server_id
            );

            if session_reply.mtu_size != mtu {
                rakrs_debug!(
                    true,
                    "[CLIENT] Server replied with different MTU size! Adapting: ({} -> {})",
                    mtu,
                    session_reply.mtu_size
                );
                mtu = session_reply.mtu_size;
            }

            rakrs_debug!(true, "[CLIENT] Received SessionInfoReply from server!");

            let mut recv_q = RecvQueue::new();

            {
                let mut send_q = send_queue.write().await;
                send_q.mtu_size = mtu;
                if let Err(_) = Self::send_connection_request(&mut send_q, id).await {
                    update_state!(true, shared_state, HandshakeStatus::Failed);
                }
            }

            rakrs_debug!(true, "[CLIENT] Sent ConnectionRequest to server!");

            let mut send_time = current_epoch() as i64;
            let mut tries = 0_u8;

            let mut buf: [u8; 2048] = [0; 2048];

            loop {
                let len: usize;
                let rec = socket.recv_from(&mut buf).await;

                if (send_time + 2) <= current_epoch() as i64 {
                    send_time = current_epoch() as i64;

                    rakrs_debug!(
                        true,
                        "[CLIENT] Server did not reply with ConnectAccept, sending another..."
                    );

                    {
                        let mut send_q = send_queue.write().await;
                        if let Err(_) = Self::send_connection_request(&mut send_q, id).await {
                            update_state!(true, shared_state, HandshakeStatus::Failed);
                        }
                    }

                    tries += 1;
                    if tries >= 5 {
                        update_state!(true, shared_state, HandshakeStatus::Failed);
                    }
                }

                match rec {
                    Err(_) => {
                        continue;
                    }
                    Ok((l, _)) => len = l,
                };

                let mut reader = ByteReader::from(&buf[..len]);

                // proccess frame packet
                match buf[0] {
                    0x80..=0x8d => {
                        if let Ok(pk) = FramePacket::read(&mut reader) {
                            log_frame_packet("rx", &pk);
                            if let Err(_) = recv_q.insert(pk) {
                                continue;
                            }

                            let ack =
                                crate::protocol::ack::Ack::from_records(recv_q.ack_flush(), false);
                            if let Ok(buf) = ack.write_to_bytes() {
                                if let Err(e) = socket
                                    .send_to(buf.as_slice(), socket.peer_addr().unwrap())
                                    .await
                                {
                                    rakrs_debug!(
                                        true,
                                        "[CLIENT] Failed to send ACK during handshake: {:?}",
                                        e
                                    );
                                }
                            }

                            let raw_packets = recv_q.flush();

                            for raw_pk in raw_packets {
                                let mut pk = ByteReader::from(&raw_pk[..]);

                                if let Ok(pk) = OnlinePacket::read(&mut pk) {
                                    match pk {
                                        OnlinePacket::ConnectedPing(pk) => {
                                            raknet_trace!(
                                                "[RR_PACKET_LOG] direction=rx stage=online packet=ConnectedPing id=0x00 timestamp={}",
                                                pk.time
                                            );
                                            rakrs_debug!(
                                                true,
                                                "[CLIENT] Received ConnectedPing from server!"
                                            );
                                            let response = ConnectedPong {
                                                ping_time: pk.time,
                                                pong_time: current_epoch() as i64,
                                            };

                                            {
                                                let mut send_q = send_queue.write().await;
                                                if let Err(_) = send_q
                                                    .send_packet(
                                                        response.into(),
                                                        Reliability::Unreliable,
                                                        true,
                                                    )
                                                    .await
                                                {
                                                    rakrs_debug!(
                                                        true,
                                                        "[CLIENT] Failed to send pong packet!"
                                                    );
                                                }
                                            }

                                            continue;
                                        }
                                        OnlinePacket::ConnectionAccept(pk) => {
                                            raknet_trace!(
                                                "[RR_PACKET_LOG] direction=rx stage=online packet=ConnectionRequestAccepted id=0x10 request_time={} timestamp={} system_index={} client_address={} internal_id_count={}",
                                                pk.request_time,
                                                pk.timestamp,
                                                pk.system_index,
                                                pk.client_address,
                                                pk.internal_ids.len()
                                            );
                                            // send new incoming connection
                                            let new_incoming = NewConnection {
                                                server_address: socket.peer_addr().unwrap(),
                                                system_address: vec![
                                                    socket.peer_addr().unwrap(),
                                                    socket.peer_addr().unwrap(),
                                                    socket.peer_addr().unwrap(),
                                                    socket.peer_addr().unwrap(),
                                                    socket.peer_addr().unwrap(),
                                                    socket.peer_addr().unwrap(),
                                                    socket.peer_addr().unwrap(),
                                                    socket.peer_addr().unwrap(),
                                                    socket.peer_addr().unwrap(),
                                                    socket.peer_addr().unwrap(),
                                                ],
                                                request_time: pk.request_time,
                                                timestamp: pk.timestamp,
                                            };
                                            {
                                                let mut send_q = send_queue.write().await;
                                                if let Err(_) = send_q
                                                    .send_packet(
                                                        new_incoming.into(),
                                                        Reliability::Unreliable,
                                                        true,
                                                    )
                                                    .await
                                                {
                                                    update_state!(
                                                        true,
                                                        shared_state,
                                                        HandshakeStatus::Failed
                                                    );
                                                } else {
                                                    update_state!(
                                                        true,
                                                        shared_state,
                                                        HandshakeStatus::Completed
                                                    );
                                                }
                                            }
                                        }
                                        _ => {
                                            rakrs_debug!(
                                                true,
                                                "[CLIENT] Received unknown packet from server!"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        });

        Self { status: state }
    }

    pub(crate) async fn send_connection_request(
        send_q: &mut SendQueue,
        id: i64,
    ) -> std::io::Result<()> {
        let connect_request = ConnectionRequest {
            time: current_epoch() as i64,
            client_id: id,
            security: false,
        };

        if let Err(_) = send_q
            .send_packet(connect_request.into(), Reliability::Unreliable, true)
            .await
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to send ConnectionRequest!",
            ));
        }
        return Ok(());
    }
}

impl Future for ClientHandshake {
    type Output = HandshakeStatus;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // see if we can finish
        let mut state = self.status.lock().unwrap();

        if state.done {
            return Poll::Ready(state.status);
        } else {
            state.waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
    }
}
