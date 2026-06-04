use std::collections::{HashMap, HashSet};

use crate::connection::controller::window::ReliableWindow;
use crate::protocol::ack::{record_range_values, Ack, Ackable, Record, SingleRecord};
use crate::protocol::frame::{Frame, FramePacket};
use crate::protocol::reliability::Reliability;
use crate::protocol::MAX_FRAGS;
use crate::server::current_epoch;
use crate::{rakrs_debug, rakrs_debug_buffers};

use super::{FragmentQueue, OrderedQueue};

macro_rules! raknet_trace {
    ($($arg:tt)*) => {
        if raknet_trace_enabled() {
            println!($($arg)*);
        }
    };
}

macro_rules! raknet_trace_err {
    ($($arg:tt)*) => {
        if raknet_trace_enabled() {
            eprintln!($($arg)*);
        }
    };
}

#[derive(Debug, Clone)]
pub enum RecvQueueError {
    OldSeq,
}

#[derive(Debug, Clone)]
pub struct RecvQueue {
    frag_queue: FragmentQueue,
    pub(crate) window: ReliableWindow,
    pub(crate) reliable_window: ReliableWindow,
    order_channels: HashMap<u8, OrderedQueue<Vec<u8>>>,
    /// Set of sequences that we've acknowledged.
    /// (seq, time)
    ack: HashSet<(u32, u64)>,
    nack: HashMap<u32, NackState>,
    initialized: bool,
    reliable_initialized: bool,
    ready: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Copy)]
struct NackState {
    first_seen: u64,
    last_sent: u64,
    attempts: u8,
}

impl RecvQueue {
    pub fn new() -> Self {
        Self {
            frag_queue: FragmentQueue::new(),
            ack: HashSet::new(),
            nack: HashMap::new(),
            initialized: false,
            reliable_initialized: false,
            window: ReliableWindow::with_size(env_u32("RAKNET_SEQUENCE_WINDOW_SIZE", 4_096)),
            reliable_window: ReliableWindow::with_size(env_u32("RAKNET_RELIABLE_WINDOW_SIZE", 16_384)),
            ready: Vec::new(),
            order_channels: HashMap::new(),
        }
    }

    pub fn insert(&mut self, packet: FramePacket) -> Result<(), RecvQueueError> {
        if !self.initialized {
            self.window.seed_start(packet.sequence);
            self.initialized = true;
            raknet_trace!(
                "[RAKNET_RECV_STATE] event=seed_window first_sequence={} window={:?}",
                packet.sequence,
                self.window.range()
            );
        }

        let removed_missing_nack = self.nack.remove(&packet.sequence).is_some();

        if !self.window.insert(packet.sequence) {
            raknet_trace!(
                "[RAKNET_RECV_STATE] event=old_sequence sequence={} removed_missing_nack={} ack_pending={} nack_pending={} ready_pending={} {}",
                packet.sequence,
                removed_missing_nack,
                self.ack.len(),
                self.nack.len(),
                self.ready.len(),
                self.reassembly_state()
            );
            return Err(RecvQueueError::OldSeq);
        }

        if self.window.range().0 < packet.sequence {
            let (window_start, _) = self.window.range();
            let max_span = env_u32("RAKNET_NACK_MAX_SPAN", 128);
            let missing_start = window_start.max(packet.sequence.saturating_sub(max_span));
            let mut added = 0usize;
            let now = current_epoch();
            for i in missing_start..packet.sequence {
                if !self.window.contains(i) && !self.nack.contains_key(&i) {
                    self.nack.insert(
                        i,
                        NackState {
                            first_seen: now,
                            last_sent: 0,
                            attempts: 0,
                        },
                    );
                    added += 1;
                }
            }
            raknet_trace!(
                "[RAKNET_RECV_STATE] event=missing_range sequence={} window_start={} requested_start={} requested_end={} capped_span={} newly_tracked={} nack_pending={} {}",
                packet.sequence,
                window_start,
                missing_start,
                packet.sequence.saturating_sub(1),
                max_span,
                added,
                self.nack.len(),
                self.reassembly_state()
            );
        }

        self.ack.insert((packet.sequence, current_epoch()));
        raknet_trace!(
            "[RAKNET_RECV_STATE] event=insert sequence={} removed_missing_nack={} ack_pending={} nack_pending={} ready_pending={} {}",
            packet.sequence,
            removed_missing_nack,
            self.ack.len(),
            self.nack.len(),
            self.ready.len(),
            self.reassembly_state()
        );

        for frame in packet.frames.iter() {
            self.handle_frame(frame);
        }

        return Ok(());
    }

    pub fn flush(&mut self) -> Vec<Vec<u8>> {
        self.ready.drain(..).collect::<Vec<Vec<u8>>>()
    }

    pub fn ack_flush(&mut self) -> Vec<u32> {
        self.ack.drain().map(|(seq, _)| seq).collect()
    }

    pub fn nack_queue(&mut self) -> Vec<u32> {
        self.expire_stale_fragments();

        let now = current_epoch();
        let ttl = env_u64("RAKNET_NACK_TTL_SECONDS", 10);
        let interval = env_u64("RAKNET_NACK_RESEND_INTERVAL_SECONDS", 1);
        let max_attempts = env_u8("RAKNET_NACK_MAX_ATTEMPTS", 8);

        let mut expired = self
            .nack
            .iter()
            .filter_map(|(sequence, state)| {
                let too_old = state.first_seen.saturating_add(ttl) < now;
                let exhausted = state.attempts >= max_attempts;
                (too_old || exhausted).then_some(*sequence)
            })
            .collect::<Vec<_>>();
        expired.sort_unstable();

        if !expired.is_empty() {
            raknet_trace!(
                "[RAKNET_NACK] event=prune expired_count={} ttl={} max_attempts={} sequences={:?}",
                expired.len(),
                ttl,
                max_attempts,
                summarize_sequences(&expired)
            );
            for sequence in expired {
                self.nack.remove(&sequence);
                self.window.skip_through(sequence);
            }
            raknet_trace!(
                "[RAKNET_RECV_STATE] event=skip_expired_nacks window={:?} nack_pending={} {}",
                self.window.range(),
                self.nack.len(),
                self.reassembly_state()
            );
        }

        let ready = self
            .nack
            .iter()
            .filter_map(|(sequence, state)| {
                let never_sent = state.last_sent == 0;
                let interval_elapsed = state.last_sent.saturating_add(interval) <= now;
                (never_sent || interval_elapsed).then_some(*sequence)
            })
            .collect::<Vec<_>>();

        for sequence in ready.iter() {
            if let Some(state) = self.nack.get_mut(sequence) {
                state.last_sent = now;
                state.attempts = state.attempts.saturating_add(1);
            }
        }

        if !ready.is_empty() {
            raknet_trace!(
                "[RAKNET_NACK] event=ready ready_count={} total_pending={} interval={} attempts_cap={} sequences={:?}",
                ready.len(),
                self.nack.len(),
                interval,
                max_attempts,
                summarize_sequences(&ready)
            );
        }

        ready
    }

    pub fn debug_state(&self) -> String {
        let ordered_channels = self.order_channels.len();
        let ordered_pending = self
            .order_channels
            .values()
            .map(|queue| queue.queue.len())
            .sum::<usize>();
        format!(
            "ack_pending={} nack_pending={} ready_pending={} ordered_channels={} ordered_pending={} {}",
            self.ack.len(),
            self.nack.len(),
            self.ready.len(),
            ordered_channels,
            ordered_pending,
            self.reassembly_state()
        )
    }

    fn reassembly_state(&self) -> String {
        let (fragment_sets, fragment_parts) = self.frag_queue.pending_counts();
        format!(
            "reassembly_sets={} reassembly_fragments={} reassembly_pending={}",
            fragment_sets,
            fragment_parts,
            self.frag_queue.pending_summary()
        )
    }

    fn expire_stale_fragments(&mut self) {
        let threshold = env_u64("RAKNET_REASSEMBLY_STALE_SECONDS", 3);
        let expired = self.frag_queue.expire_stale(threshold);
        for fragment in expired {
            raknet_trace!(
                "[RAKNET_REASSEMBLY] event=expire_stale split_id={} fragments={}/{} order_channel={:?} order_index={:?} threshold={}s",
                fragment.id,
                fragment.count,
                fragment.size,
                fragment.order_channel,
                fragment.order_index,
                threshold
            );

            if let (Some(channel), Some(index)) = (fragment.order_channel, fragment.order_index) {
                let queue = self
                    .order_channels
                    .entry(channel)
                    .or_insert(OrderedQueue::new());
                queue.skip_through(index);
                raknet_trace!(
                    "[RAKNET_ORDERED] event=skip_stale_fragment channel={} index={} pending_before_flush={} window={:?}",
                    channel,
                    index,
                    queue.queue.len(),
                    queue.window
                );
                for pk in queue.flush() {
                    self.ready.push(pk);
                }
                raknet_trace!(
                    "[RAKNET_ORDERED] event=flush_after_skip channel={} pending_after_flush={} window={:?} ready_pending={}",
                    channel,
                    queue.queue.len(),
                    queue.window,
                    self.ready.len()
                );
            }
        }
    }

    fn handle_frame(&mut self, frame: &Frame) {
        if let Some(reliable_index) = frame.reliable_index {
            if !self.reliable_initialized {
                self.reliable_window.seed_start(reliable_index);
                self.reliable_initialized = true;
                raknet_trace!(
                    "[RAKNET_RECV_STATE] event=seed_reliable_window first_reliable_index={} window={:?}",
                    reliable_index,
                    self.reliable_window.range()
                );
            }

            if !self.reliable_window.insert(reliable_index) {
                raknet_trace!(
                    "[RAKNET_RELIABLE] event=drop_duplicate_or_outside_window reliable_index={} window={:?} reliability={:?} order_channel={:?} order_index={:?} fragment={:?}",
                    reliable_index,
                    self.reliable_window.range(),
                    frame.reliability,
                    frame.order_channel,
                    frame.order_index,
                    frame.fragment_meta
                );
                return;
            }
        }

        if let Some(meta) = frame.fragment_meta.as_ref() {
            if meta.size > MAX_FRAGS {
                rakrs_debug!(true, "Fragment size is too large, rejected {}!", meta.size);
                return;
            }
            if let Err(err) = self.frag_queue.insert(frame.clone()) {
                raknet_trace_err!(
                    "[RAKNET_REASSEMBLY] fragment_error split_id={} split_count={} split_index={} error={}",
                    meta.id, meta.size, meta.index, err
                );
            }

            let current_count = self
                .frag_queue
                .get(&meta.id)
                .map(|(_, frames)| frames.len())
                .unwrap_or(0);
            raknet_trace_err!(
                "[RAKNET_REASSEMBLY] fragment split_id={} split_count={} split_index={} current_count={} total_count={}",
                meta.id, meta.size, meta.index, current_count, meta.size
            );

            let res = self.frag_queue.collect(meta.id);
            if let Ok(data) = res {
                let route = if matches!(frame.reliability, Reliability::ReliableOrd)
                    && frame.order_channel.is_some()
                    && frame.order_index.is_some()
                {
                    "ordered"
                } else {
                    "ready"
                };
                raknet_trace_err!(
                    "[RAKNET_REASSEMBLY] complete split_id={} fragments={} final_size={} reliability={:?} order_channel={:?} order_index={:?} route={}",
                    meta.id,
                    meta.size,
                    data.len(),
                    frame.reliability,
                    frame.order_channel,
                    frame.order_index,
                    route
                );
                raknet_trace_err!(
                    "[RAKNET_REASSEMBLY] complete_payload split_id={} final_size={} first_32={:02x?}",
                    meta.id,
                    data.len(),
                    &data[..data.len().min(32)]
                );
                self.queue_frame_body(
                    frame.reliability,
                    frame.order_channel,
                    frame.order_index,
                    data,
                );
            } else {
                rakrs_debug!(
                    true,
                    "Still Missing some fragments! {:?}",
                    frame.fragment_meta.as_ref().unwrap()
                );
            }
            return;
        }

        rakrs_debug_buffers!(
            true,
            "RecvQueue: {}\n{:?}\n",
            frame.body.len(),
            frame.body.clone()
        );

        match frame.reliability {
            Reliability::Unreliable => {
                self.ready.push(frame.body.clone());
            }
            Reliability::Reliable => {
                self.ready.push(frame.body.clone());
            }
            Reliability::ReliableOrd => {
                self.queue_ordered_frame(
                    frame.order_channel,
                    frame.order_index,
                    frame.body.clone(),
                );
            }
            _ => {
                self.ready.push(frame.body.clone());
            }
        }
    }

    fn queue_frame_body(
        &mut self,
        reliability: Reliability,
        order_channel: Option<u8>,
        order_index: Option<u32>,
        body: Vec<u8>,
    ) {
        if matches!(reliability, Reliability::ReliableOrd) {
            self.queue_ordered_frame(order_channel, order_index, body);
        } else {
            self.ready.push(body);
        }
    }

    fn queue_ordered_frame(
        &mut self,
        order_channel: Option<u8>,
        order_index: Option<u32>,
        body: Vec<u8>,
    ) {
        let Some(channel) = order_channel else {
            self.ready.push(body);
            return;
        };
        let Some(index) = order_index else {
            self.ready.push(body);
            return;
        };

        let queue = self
            .order_channels
            .entry(channel)
            .or_insert(OrderedQueue::new());

        if queue.insert(index, body) {
            raknet_trace!(
                "[RAKNET_ORDERED] event=insert channel={} index={} pending_before_flush={} window={:?}",
                channel,
                index,
                queue.queue.len(),
                queue.window
            );
            for pk in queue.flush() {
                self.ready.push(pk);
            }
            raknet_trace!(
                "[RAKNET_ORDERED] event=flush channel={} pending_after_flush={} window={:?} ready_pending={}",
                channel,
                queue.queue.len(),
                queue.window,
                self.ready.len()
            );
        } else {
            raknet_trace!(
                "[RAKNET_ORDERED] event=drop_duplicate_or_old channel={} index={} pending={} window={:?}",
                channel,
                index,
                queue.queue.len(),
                queue.window
            );
        }
    }
}

impl Ackable for RecvQueue {
    type NackItem = ();

    fn ack(&mut self, ack: Ack) {
        if ack.is_nack() {
            rakrs_debug!(true, "Invalid ack: {:?}", ack.clone());
            return;
        }

        rakrs_debug!(true, "Got ack item: {:?}", ack.clone());

        // these packets are acknowledged, so we can remove them from the queue.
        for record in ack.records.iter() {
            match record {
                Record::Single(SingleRecord { sequence }) => {
                    self.nack.remove(&sequence.0);
                }
                Record::Range(ranged) => {
                    for i in record_range_values(ranged.start.0, ranged.end.0) {
                        self.nack.remove(&i);
                    }
                }
            }
        }
    }
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_u8(name: &str, default: u8) -> u8 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u8>().ok())
        .unwrap_or(default)
}

fn raknet_trace_enabled() -> bool {
    std::env::var("RAKNET_TRACE")
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
}

fn summarize_sequences(sequences: &[u32]) -> Vec<String> {
    let mut sorted = sequences.to_vec();
    sorted.sort_unstable();
    sorted
        .chunks(16)
        .take(4)
        .map(|chunk| {
            let start = chunk.first().copied().unwrap_or_default();
            let end = chunk.last().copied().unwrap_or_default();
            if start == end {
                start.to_string()
            } else {
                format!("{}..{}", start, end)
            }
        })
        .collect()
}
