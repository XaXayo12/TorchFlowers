use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "async_std")]
use async_std::net::UdpSocket;

use binary_util::interfaces::Writer;
#[cfg(feature = "async_tokio")]
use tokio::net::UdpSocket;

use crate::protocol::ack::{record_range_values, Ack, Ackable, Record, SingleRecord};
use crate::protocol::frame::{Frame, FramePacket};
use crate::protocol::packet::RakPacket;
use crate::protocol::reliability::Reliability;
use crate::util::{to_address_token, SafeGenerator};
use crate::{rakrs_debug, rakrs_debug_buffers};

use super::{fragment_body_limit, FragmentQueue, FragmentQueueError, NetQueue, RecoveryQueue};

macro_rules! raknet_trace {
    ($($arg:tt)*) => {
        if raknet_trace_enabled() {
            println!($($arg)*);
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SendQueueError {
    /// The packet is too large to be sent.
    PacketTooLarge,
    /// Parsing Error
    ParseError,
    /// Fragmentation error
    FragmentError(FragmentQueueError),
    /// Send queue error
    SendError,
}

impl std::fmt::Display for SendQueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                SendQueueError::PacketTooLarge => "Packet too large".to_string(),
                SendQueueError::ParseError => "Parse error".to_string(),
                SendQueueError::FragmentError(e) => format!("Fragment error: {}", e),
                SendQueueError::SendError => "Send error".to_string(),
            }
        )
    }
}

impl std::error::Error for SendQueueError {}

/// This queue is used to prioritize packets being sent out
/// Packets that are old, are either dropped or requested again.
/// You can define this behavior with the `timeout` property.
#[derive(Debug, Clone)]
pub struct SendQueue {
    pub(crate) mtu_size: u16,

    /// The amount of time that needs to pass for a packet to be
    /// dropped or requested again.
    _timeout: u16,

    /// The amount of times we should retry sending a packet before
    /// dropping it from the queue. This is currently set to `5`.
    _max_tries: u16,

    /// The current sequence number. This is incremented every time
    /// a packet is sent reliably. We can resend these if they are
    /// NAcked.
    send_seq: SafeGenerator<u32>,

    /// The current reliable index number.
    /// a packet is sent reliably an sequenced.
    reliable_seq: SafeGenerator<u32>,

    /// The current recovery queue.
    ack: RecoveryQueue<FramePacket>,

    /// Recently sent reliable frames are retained briefly so a late NACK can
    /// still be serviced after the server has also sent an ACK for the same
    /// sequence. Some Bedrock proxies emit that conflicting ACK/NACK pattern
    /// during fragmented Login.
    recent_sent: HashMap<u32, FramePacket>,
    recent_sent_order: VecDeque<u32>,

    /// The fragment queue.
    fragment_queue: FragmentQueue,

    /// The ordered channels.
    /// (send_seq, reliable_seq)
    order_channels: HashMap<u8, (u32, u32)>,

    ready: Vec<Frame>,

    socket: Arc<UdpSocket>,

    address: SocketAddr,
}

impl SendQueue {
    pub fn new(
        mtu_size: u16,
        _timeout: u16,
        _max_tries: u16,
        socket: Arc<UdpSocket>,
        address: SocketAddr,
    ) -> Self {
        Self {
            mtu_size,
            _timeout,
            _max_tries,
            send_seq: SafeGenerator::new(),
            reliable_seq: SafeGenerator::new(),
            ack: RecoveryQueue::new(),
            recent_sent: HashMap::new(),
            recent_sent_order: VecDeque::new(),
            fragment_queue: FragmentQueue::new(),
            order_channels: HashMap::new(),
            ready: Vec::new(),
            socket,
            address,
        }
    }

    /// Send a packet based on its reliability.
    /// Note, reliability will be set to `Reliability::ReliableOrd` if
    /// the buffer is larger than max MTU.
    pub async fn insert(
        &mut self,
        packet: &[u8],
        reliability: Reliability,
        immediate: bool,
        channel: Option<u8>,
    ) -> Result<(), SendQueueError> {
        rakrs_debug!(
            true,
            "Inserting packet into send queue: {} bytes",
            packet.len()
        );
        rakrs_debug!("Write is now processing packet");
        let fragment_limit = fragment_body_limit(self.mtu_size) as usize;
        let reliable = if packet.len() > fragment_limit {
            Reliability::ReliableOrd
        } else {
            reliability
        };
        raknet_trace!(
            "[RR_PACKET_LOG] direction=tx stage=queue packet={} body_id={:#04x} body_len={} requested_reliability={:?} effective_reliability={:?} immediate={} channel={:?}",
            packet_body_name(packet),
            packet.first().copied().unwrap_or(0),
            packet.len(),
            reliability,
            reliable,
            immediate,
            channel
        );

        rakrs_debug!("Write is now processing packet: {:?}", reliable);

        match reliability {
            Reliability::Unreliable => {
                // we can just send this packet out immediately.
                let frame = Frame::new(Reliability::Unreliable, Some(packet));
                self.send_frame(frame).await;
                return Ok(());
            }
            Reliability::Reliable => {
                // we need to send this packet out reliably.
                let frame = Frame::new(Reliability::Reliable, Some(packet));
                self.send_frame(frame).await;
                return Ok(());
            }
            _ => {}
        };

        // do another integrity check
        // this is to check to see if we really need to split this packet.
        if packet.len() > fragment_limit {
            // we need to split this packet!
            // pass the buffer to the fragment queue.
            rakrs_debug!("Write is now splitting, too large: {:?}", reliability);

            let fragmented = self.fragment_queue.split_insert(&packet, self.mtu_size);

            if fragmented.is_ok() {
                let frag_id = fragmented.unwrap();
                let frames_to_send = {
                    let (_, frames) = self.fragment_queue.get_mut(&frag_id).unwrap();
                    let (ord_seq, ord_index) = self
                        .order_channels
                        .entry(channel.unwrap_or(0))
                        .or_insert((0, 0));

                    for frame in frames.iter_mut() {
                        frame.reliability = reliable;
                        frame.sequence_index = Some(*ord_seq);
                        frame.order_channel = Some(channel.unwrap_or(0));
                        frame.order_index = Some(*ord_index);

                        if frame.reliability.is_reliable() {
                            frame.reliable_index = Some(self.reliable_seq.next());
                        }
                    }

                    *ord_index = ord_index.wrapping_add(1);
                    *ord_seq = ord_seq.wrapping_add(1);
                    frames.clone()
                };

                let frames_len = frames_to_send.len();
                for (index, frame) in frames_to_send.into_iter().enumerate() {
                    let mut pk = FramePacket::new();
                    pk.sequence = self.send_seq.next();
                    pk.reliability = frame.reliability;
                    pk.frames.push(frame);
                    log_frame_packet("tx", &pk);

                    if let Ok(p) = pk.write_to_bytes() {
                        rakrs_debug!("Write is sending stream: {:?}", reliability);

                        self.send_stream(p.as_slice()).await;
                        self.remember_sent_packet(&pk);
                        self.ack.insert_id(pk.sequence, pk);
                        if index + 1 < frames_len {
                            fragment_send_delay().await;
                        }
                    } else {
                        return Err(SendQueueError::SendError);
                    }
                }

                return Ok(());
            } else {
                // we couldn't send this frame!
                return Err(SendQueueError::FragmentError(fragmented.unwrap_err()));
            }
        } else {
            // we're not gonna send this frame out yet!
            // we need to wait for the next tick.
            let mut frame = Frame::new(reliable, Some(packet));

            if frame.reliability.is_reliable() {
                frame.reliable_index = Some(self.reliable_seq.next());
            }

            if frame.reliability.is_ordered() {
                let order_channel = channel.unwrap_or(0);
                let (_, ord_index) = self.order_channels.entry(order_channel).or_insert((0, 0));
                frame.order_index = Some(*ord_index);
                frame.order_channel = Some(order_channel);
                frame.sequence_index = Some(self.send_seq.get());
                *ord_index = ord_index.wrapping_add(1);
            } else if frame.reliability.is_sequenced() {
                let order_channel = channel.unwrap_or(0);
                let (seq_index, ord_index) =
                    self.order_channels.entry(order_channel).or_insert((0, 0));
                *seq_index = seq_index.wrapping_add(1);
                frame.order_index = Some(*ord_index);
                frame.order_channel = Some(order_channel);
                frame.sequence_index = Some(*seq_index);
            }

            if immediate {
                self.send_frame(frame).await;
            } else {
                self.ready.push(frame);
            }

            return Ok(());
        }
    }

    /// A wrapper to send a single frame over the wire.
    /// While also reliabily tracking it.
    async fn send_frame(&mut self, mut frame: Frame) {
        let mut pk = FramePacket::new();
        pk.sequence = self.send_seq.next();
        pk.reliability = frame.reliability;

        if pk.reliability.is_reliable() && frame.reliable_index.is_none() {
            frame.reliable_index = Some(self.reliable_seq.next());
        }

        pk.frames.push(frame);
        log_frame_packet("tx", &pk);

        if pk.reliability.is_reliable() {
            // this seems redundant, but we need to insert the packet into the ACK queue
            self.ack.insert_id(pk.sequence, pk.clone());
            self.remember_sent_packet(&pk);
        }

        if let Ok(buf) = pk.write_to_bytes() {
            rakrs_debug!("[!] Write sent the packet.. {:?}", buf.as_slice());
            self.send_stream(buf.as_slice()).await;
        } else {
            rakrs_debug_buffers!(true, "SendQ: Failed to send frame: {:?}", pk);
        }
    }

    pub(crate) async fn send_stream(&mut self, packet: &[u8]) {
        rakrs_debug_buffers!(false, "SendQ: {}\n{:?}\n", packet.len(), packet);

        if let Err(e) = self.socket.send_to(packet, &self.address).await {
            // we couldn't sent the packet!
            rakrs_debug!(
                true,
                "[{}] Failed to send packet! {:?}",
                to_address_token(self.address),
                e
            );
        }
    }

    pub(crate) async fn resend_frame_packet(&mut self, packet: &FramePacket) {
        raknet_trace!(
            "[RR_PACKET_LOG] direction=tx stage=frame packet=FrameSet sequence={} frame_count={} (nack_resend)",
            packet.sequence,
            packet.frames.len()
        );
        self.ack.insert_id(packet.sequence, packet.clone());
        self.remember_sent_packet(packet);
        if let Ok(buf) = packet.write_to_bytes() {
            self.send_stream(buf.as_slice()).await;
            fragment_send_delay().await;
        }
    }

    pub(crate) fn debug_state(&self) -> String {
        format!(
            "pending_reliable={} ready_pending={} recent_sent={} keys={:?}",
            self.ack.len(),
            self.ready.len(),
            self.recent_sent.len(),
            self.ack.keys()
        )
    }

    pub async fn send_packet(
        &mut self,
        packet: RakPacket,
        reliability: Reliability,
        immediate: bool,
    ) -> Result<(), SendQueueError> {
        // parse the packet
        if let Ok(buf) = packet.write_to_bytes() {
            if let Err(e) = self
                .insert(buf.as_slice(), reliability, immediate, None)
                .await
            {
                rakrs_debug!(
                    true,
                    "[{}] Failed to insert packet into send queue: {:?}",
                    to_address_token(self.address),
                    e
                );
                return Err(e);
            }
            return Ok(());
        } else {
            return Err(SendQueueError::ParseError);
        }
    }

    pub async fn update(&mut self) {
        // send all the ready packets
        // TODO batch these packets together
        // TODO by lengths
        for frame in self.ready.drain(..).collect::<Vec<Frame>>() {
            self.send_frame(frame).await;
        }

        let ack_keys = self.ack.keys();
        let stale_keys = self.ack.stale_keys(8);
        raknet_trace!(
            "[RAKNET_SEND_STATE] current_epoch={} pending_reliable={} stale_reliable={} ready_pending={} recent_sent={} keys={:?} stale_keys={:?}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
            self.ack.len(),
            stale_keys.len(),
            self.ready.len(),
            self.recent_sent.len(),
            ack_keys,
            stale_keys
        );

        if timeout_resend_enabled() {
            let resend_threshold = timeout_resend_threshold_seconds();
            let resend_limit = timeout_resend_limit_per_tick();
            let mut resend_queue = self.ack.flush_old(resend_threshold);
            if resend_queue.len() > resend_limit {
                let deferred = resend_queue.split_off(resend_limit);
                for packet in deferred {
                    self.ack.insert_id(packet.sequence, packet);
                }
            }

            let resend_len = resend_queue.len();
            if resend_len > 0 {
                raknet_trace!(
                    "[RAKNET_SEND_STATE] event=timeout_resend count={} threshold={}s limit_per_tick={}",
                    resend_len,
                    resend_threshold,
                    resend_limit
                );
            }
            for (index, packet) in resend_queue.iter().enumerate() {
                raknet_trace!(
                    "[RR_PACKET_LOG] direction=tx stage=frame packet=FrameSet sequence={} frame_count={} (timeout_resend)",
                    packet.sequence,
                    packet.frames.len()
                );
                self.ack.insert_id(packet.sequence, packet.clone());
                self.remember_sent_packet(packet);
                if let Ok(buf) = packet.write_to_bytes() {
                    self.send_stream(buf.as_slice()).await;
                    if index + 1 < resend_len {
                        fragment_send_delay().await;
                    }
                }
            }
        } else {
            let dropped = self.ack.drop_older_than(12);
            if !dropped.is_empty() {
                raknet_trace!(
                    "[RAKNET_SEND_STATE] event=drop_stale_unacked count={} sequences={:?} reason=nack_driven_recovery",
                    dropped.len(),
                    dropped
                );
            }
        }
    }

    fn remember_sent_packet(&mut self, packet: &FramePacket) {
        const RECENT_SENT_LIMIT: usize = 512;

        if !self.recent_sent.contains_key(&packet.sequence) {
            self.recent_sent_order.push_back(packet.sequence);
        }
        self.recent_sent.insert(packet.sequence, packet.clone());

        while self.recent_sent_order.len() > RECENT_SENT_LIMIT {
            if let Some(oldest) = self.recent_sent_order.pop_front() {
                self.recent_sent.remove(&oldest);
            }
        }
    }
}

fn timeout_resend_enabled() -> bool {
    std::env::var("RAKNET_TIMEOUT_RESEND")
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(true)
}

fn raknet_trace_enabled() -> bool {
    std::env::var("RAKNET_TRACE")
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
}

fn timeout_resend_threshold_seconds() -> u64 {
    std::env::var("RAKNET_TIMEOUT_RESEND_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(2)
}

fn timeout_resend_limit_per_tick() -> usize {
    std::env::var("RAKNET_TIMEOUT_RESEND_LIMIT")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(16)
}

async fn fragment_send_delay() {
    let delay_ms = std::env::var("RAKNET_FRAGMENT_SEND_DELAY_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(2);
    if delay_ms == 0 {
        return;
    }

    #[cfg(feature = "async_tokio")]
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    #[cfg(feature = "async_std")]
    async_std::task::sleep(Duration::from_millis(delay_ms)).await;
}

fn packet_body_name(packet: &[u8]) -> &'static str {
    match packet.first().copied() {
        Some(0x00) => "ConnectedPing",
        Some(0x03) => "ConnectedPong",
        Some(0x09) => "ConnectionRequest",
        Some(0x10) => "ConnectionRequestAccepted",
        Some(0x13) => "NewIncomingConnection",
        Some(0x15) => "Disconnect",
        Some(0xfe) => match decode_bedrock_packet_id(packet) {
            Some(193) => "RequestNetworkSettings",
            Some(_) => "GamePacket",
            None => "GamePacketUnknown",
        },
        _ => "Unknown",
    }
}

fn log_frame_packet(direction: &str, packet: &FramePacket) {
    raknet_trace!(
        "[RR_PACKET_LOG] direction={} stage=frame packet=FrameSet sequence={} frame_count={}",
        direction,
        packet.sequence,
        packet.frames.len()
    );

    for (index, frame) in packet.frames.iter().enumerate() {
        let body_id = frame.body.first().copied().unwrap_or(0);
        raknet_trace!(
            "[RR_PACKET_LOG] direction={} stage=frame frame_index={} reliability={:?} body_id={:#04x} packet={} body_len={} reliable_index={:?} sequence_index={:?} order_index={:?} order_channel={:?} fragment={:?}",
            direction,
            index,
            frame.reliability,
            body_id,
            packet_body_name(&frame.body),
            frame.body.len(),
            frame.reliable_index,
            frame.sequence_index,
            frame.order_index,
            frame.order_channel,
            frame.fragment_meta
        );
    }
}

fn decode_bedrock_packet_id(packet: &[u8]) -> Option<u32> {
    if packet.first().copied()? != 0xfe {
        return None;
    }

    let mut offset = 1usize;
    let _payload_len = read_var_u32(packet, &mut offset)?;
    read_var_u32(packet, &mut offset)
}

fn read_var_u32(packet: &[u8], offset: &mut usize) -> Option<u32> {
    let mut value = 0u32;
    let mut shift = 0u32;

    for _ in 0..5 {
        let byte = *packet.get(*offset)?;
        *offset += 1;
        value |= ((byte & 0x7f) as u32) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
    }

    None
}

impl Ackable for SendQueue {
    type NackItem = FramePacket;

    fn ack(&mut self, ack: Ack) {
        if ack.is_nack() {
            return;
        }

        // these packets are acknowledged, so we can remove them from the queue.
        for record in ack.records.iter() {
            match record {
                Record::Single(SingleRecord { sequence }) => {
                    if let Ok(_) = self.ack.remove(sequence.0) {};
                }
                Record::Range(ranged) => {
                    for i in record_range_values(ranged.start.0, ranged.end.0) {
                        if let Ok(_) = self.ack.remove(i) {};
                    }
                }
            }
        }
    }

    fn nack(&mut self, nack: Ack) -> Vec<FramePacket> {
        if !nack.is_nack() {
            return Vec::new();
        }

        let mut resend_queue = Vec::<FramePacket>::new();
        let mut requested = 0usize;
        let mut missing = Vec::<u32>::new();

        // we need to get the packets to resend.
        for record in nack.records.iter() {
            match record {
                Record::Single(single) => {
                    requested += 1;
                    if let Ok(packet) = self.ack.get(single.sequence.0) {
                        resend_queue.push(packet.clone());
                    } else if let Some(packet) = self.recent_sent.get(&single.sequence.0) {
                        raknet_trace!(
                            "[RR_PACKET_LOG] direction=tx stage=nack_recover sequence={} source=recent_sent",
                            single.sequence.0
                        );
                        resend_queue.push(packet.clone());
                    } else {
                        missing.push(single.sequence.0);
                    }
                }
                Record::Range(ranged) => {
                    for i in record_range_values(ranged.start.0, ranged.end.0) {
                        requested += 1;
                        if let Ok(packet) = self.ack.get(i) {
                            resend_queue.push(packet.clone());
                        } else if let Some(packet) = self.recent_sent.get(&i) {
                            raknet_trace!(
                                "[RR_PACKET_LOG] direction=tx stage=nack_recover sequence={} source=recent_sent",
                                i
                            );
                            resend_queue.push(packet.clone());
                        } else {
                            missing.push(i);
                        }
                    }
                }
            }
        }

        raknet_trace!(
            "[RAKNET_NACK] requested={} resend={} missing={} missing_sequences={:?}",
            requested,
            resend_queue.len(),
            missing.len(),
            missing
        );

        return resend_queue;
    }
}
