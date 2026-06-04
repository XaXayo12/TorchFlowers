pub(crate) mod recv;
pub(crate) mod send;

pub use self::recv::*;
pub use self::send::*;

use std::collections::BTreeMap;
use std::collections::HashMap;

use crate::protocol::frame::FragmentMeta;
use crate::protocol::frame::Frame;
use crate::protocol::reliability::Reliability;
use crate::protocol::RAKNET_HEADER_FRAME_OVERHEAD;
use crate::rakrs_debug;
use crate::server::current_epoch;

#[derive(Debug, Clone)]
pub enum NetQueueError<E> {
    /// The insertion failed for any given reason.
    InvalidInsertion,
    /// The insertion failed and the reason is known.
    InvalidInsertionKnown(String),
    /// The `Item` failed to be removed from the queue.
    ItemDeletionFail,
    /// The `Item` is invalid and can not be retrieved.
    InvalidItem,
    /// The queue is empty.
    EmptyQueue,
    /// The error is a custom error.
    Other(E),
}

pub trait NetQueue<Item> {
    /// The `Item` of the queue.
    // type Item = V;

    /// The "key" that each `Item` is stored under
    /// (used for removal)
    type KeyId;

    /// A custom error specifier for NetQueueError
    type Error;

    /// Inserts `Item` into the queue, given the conditions are fulfilled.
    fn insert(&mut self, item: Item) -> Result<Self::KeyId, NetQueueError<Self::Error>>;

    /// Remove an `Item` from the queue by providing an instance of `Self::KeyId`
    fn remove(&mut self, key: Self::KeyId) -> Result<Item, NetQueueError<Self::Error>>;

    /// Retrieves an `Item` from the queue, by reference.
    fn get(&mut self, key: Self::KeyId) -> Result<&Item, NetQueueError<Self::Error>>;

    /// Clears the entire queue.
    fn flush(&mut self) -> Result<Vec<Item>, NetQueueError<Self::Error>>;
}

/// A recovery queue is used to store packets that need to be resent.
/// This is used for sequenced and ordered packets.
#[derive(Debug, Clone)]
pub struct RecoveryQueue<Item> {
    /// The current queue of packets by timestamp
    /// (seq, (packet, timestamp))
    // TODO use the timestamp for round trip time (RTT)
    queue: HashMap<u32, (u64, Item)>,
}

impl<Item> RecoveryQueue<Item>
where
    Item: Clone,
{
    pub fn new() -> Self {
        Self {
            queue: HashMap::new(),
        }
    }

    pub fn insert_id(&mut self, seq: u32, item: Item) {
        self.queue.insert(seq, (current_epoch(), item));
    }

    pub fn get_all(&mut self) -> Vec<(u32, Item)> {
        self.queue
            .iter()
            .map(|(seq, (_, item))| (*seq, item.clone()))
            .collect::<Vec<_>>()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn keys(&self) -> Vec<u32> {
        self.queue.keys().copied().collect()
    }

    pub fn stale_keys(&self, threshold: u64) -> Vec<u32> {
        self.queue
            .iter()
            .filter_map(|(seq, (time, _))| ((*time + threshold) < current_epoch()).then_some(*seq))
            .collect()
    }

    pub fn drop_older_than(&mut self, threshold: u64) -> Vec<u32> {
        let stale = self.stale_keys(threshold);
        for seq in stale.iter() {
            self.queue.remove(seq);
        }
        stale
    }

    pub fn flush_old(&mut self, threshold: u64) -> Vec<Item> {
        let old = self
            .queue
            .iter()
            .filter(|(_, (time, _))| (*time + threshold) < current_epoch())
            .map(|(_, (_, item))| item.clone())
            .collect::<Vec<_>>();
        self.queue
            .retain(|_, (time, _)| (*time + threshold) >= current_epoch());
        old
    }
}

impl<Item> NetQueue<Item> for RecoveryQueue<Item> {
    type KeyId = u32;
    type Error = ();

    fn insert(&mut self, item: Item) -> Result<Self::KeyId, NetQueueError<Self::Error>> {
        let index = self.queue.len() as u32;
        self.queue.insert(index, (current_epoch(), item));
        Ok(index)
    }

    fn remove(&mut self, key: Self::KeyId) -> Result<Item, NetQueueError<Self::Error>> {
        if let Some((_, item)) = self.queue.remove(&key) {
            Ok(item)
        } else {
            Err(NetQueueError::ItemDeletionFail)
        }
    }

    fn get(&mut self, key: Self::KeyId) -> Result<&Item, NetQueueError<Self::Error>> {
        if let Some((_, item)) = self.queue.get(&key) {
            Ok(item)
        } else {
            Err(NetQueueError::ItemDeletionFail)
        }
    }

    fn flush(&mut self) -> Result<Vec<Item>, NetQueueError<Self::Error>> {
        let mut items = Vec::new();
        for (_, (_, item)) in self.queue.drain() {
            items.push(item);
        }
        Ok(items)
    }
}

/// An ordered queue is used to Index incoming packets over a channel
/// within a reliable window time.
///
/// Usage:
/// ```ignore
/// use rak_rs::connection::queue::OrderedQueue;
/// let mut ord_qu: OrderedQueue<Vec<u8>> = OrderedQueue::new();
/// // Insert a packet with the id of "1"
/// ord_qu.insert(1, vec![0, 1]);
/// ord_qu.insert(5, vec![1, 0]);
/// ord_qu.insert(3, vec![2, 0]);
///
/// // Get the packets we still need.
/// let needed: Vec<u32> = ord_qu.missing();
/// assert_eq!(needed, vec![0, 2, 4]);
///
/// // We would in theory, request these packets, but we're going to insert them
/// ord_qu.insert(4, vec![2, 0, 0, 1]);
/// ord_qu.insert(2, vec![1, 0, 0, 2]);
///
/// // Now let's return our packets in order.
/// // Will return a vector of these packets in order by their "id".
/// let ordered: Vec<Vec<u8>> = ord_qu.flush();
/// ```
#[derive(Debug, Clone)]
pub struct OrderedQueue<Item: Clone + std::fmt::Debug> {
    /// The current ordered queue channels
    /// Channel, (Highest Index, Ord Index, Item)
    pub queue: BTreeMap<u32, Item>,
    /// The window for this queue.
    pub window: (u32, u32),
}

impl<Item> OrderedQueue<Item>
where
    Item: Clone + std::fmt::Debug,
{
    pub fn new() -> Self {
        Self {
            queue: BTreeMap::new(),
            window: (0, 0),
        }
    }

    pub fn next(&mut self) -> u32 {
        self.window.0 = self.window.0.wrapping_add(1);
        return self.window.0;
    }

    pub fn insert(&mut self, index: u32, item: Item) -> bool {
        if index < self.window.0 {
            return false;
        }

        if self.queue.contains_key(&index) {
            return false;
        }

        if index >= self.window.1 {
            self.window.1 = index + 1;
        }

        self.queue.insert(index, item);
        true
    }

    pub fn insert_abs(&mut self, index: u32, item: Item) {
        if index >= self.window.1 {
            self.window.1 = index + 1;
        }

        self.queue.insert(index, item);
    }

    pub fn missing(&self) -> Vec<u32> {
        let mut missing = Vec::new();
        for i in self.window.0..self.window.1 {
            if !self.queue.contains_key(&i) {
                missing.push(i);
            }
        }
        missing
    }

    /// Forcefully flushes the incoming queue resetting the highest window
    /// to the lowest window.
    ///
    /// THIS IS A PATCH FIX UNTIL I CAN FIGURE OUT WHY THE OTHER FLUSH IS BROKEN
    pub fn flush(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        while let Some(item) = self.queue.remove(&self.window.0) {
            items.push(item);
            self.window.0 = self.window.0.wrapping_add(1);
        }
        items
    }

    pub fn skip_through(&mut self, index: u32) {
        if self.window.0 <= index {
            self.window.0 = index.wrapping_add(1);
        }
        if self.window.1 <= index {
            self.window.1 = index.wrapping_add(1);
        }
    }

    /// Older, broken implementation, idk what is causing this to break
    /// after index 3
    /// The logic here is supposed to be, remove all indexes until the highest most up to date index.
    /// and retain older indexes until the order is correct.
    pub fn flush_old_impl(&mut self) -> Vec<Item> {
        let mut items = Vec::<(u32, Item)>::new();

        let mut i = self.window.0;

        while self.queue.contains_key(&i) {
            rakrs_debug!("[!>] Removing: {}", &i);
            if let Some(item) = self.queue.remove(&i) {
                items.push((self.window.0, item));
                i += 1;
            } else {
                break;
            }
        }

        self.window.0 = i;

        items.sort_by(|a, b| a.0.cmp(&b.0));
        return items
            .iter()
            .map(|(_, item)| item.clone())
            .collect::<Vec<Item>>();
    }
}

/// A specialized structure for re-ordering fragments over the wire.
/// You can use this structure to fragment frames as well.
///
/// **NOTE:** This structure will NOT update a frame's reliable index!
/// The sender is required to this!
#[derive(Clone, Debug)]
pub struct FragmentQueue {
    /// The current fragment id to use
    /// If for some reason this wraps back to 0,
    /// and the fragment queue is full, 0 is then cleared and reused.
    fragment_id: u16,

    /// The current Fragments
    /// Hashmap is by Fragment id, with the value being
    /// (`size`, Vec<Frame>)
    fragments: HashMap<u16, (u32, Vec<Frame>)>,
    fragment_updated_at: HashMap<u16, u64>,
}

#[derive(Clone, Debug)]
pub struct ExpiredFragment {
    pub id: u16,
    pub size: u32,
    pub count: usize,
    pub order_channel: Option<u8>,
    pub order_index: Option<u32>,
}

impl FragmentQueue {
    pub fn new() -> Self {
        Self {
            fragment_id: 0,
            fragments: HashMap::new(),
            fragment_updated_at: HashMap::new(),
        }
    }

    /// Inserts the frame into the fragment queue.
    /// Returns a result tuple of (`fragment_size`, `fragment_index`)
    pub fn insert(&mut self, fragment: Frame) -> Result<(u32, u32), FragmentQueueError> {
        if let Some(meta) = fragment.fragment_meta.clone() {
            if let Some((size, frames)) = self.fragments.get_mut(&meta.id) {
                // check if the frame index is out of bounds
                // todo: Check if == or >, I think it's > but I'm not sure.
                // todo: This is because the index starts at 0 and the size starts at 1.
                if meta.index >= *size {
                    return Err(FragmentQueueError::FrameIndexOutOfBounds);
                }
                // the frame exists, and we have parts, check if we have this particular frame already.
                if let Some(_) = frames
                    .iter()
                    .find(|&f| f.fragment_meta.as_ref().unwrap().index == meta.index)
                {
                    // We already have this frame! Do not replace it!!
                    return Err(FragmentQueueError::FrameExists);
                } else {
                    frames.push(fragment);
                    self.fragment_updated_at.insert(meta.id, current_epoch());
                    return Ok((meta.size, meta.index));
                }
            } else {
                // We don't already have this fragment index!
                let (size, mut frames) = (meta.size, Vec::<Frame>::new());
                frames.push(fragment);

                self.fragments.insert(meta.id, (size, frames));
                self.fragment_updated_at.insert(meta.id, current_epoch());
                return Ok((meta.size, meta.index));
            }
        }

        return Err(FragmentQueueError::FrameNotFragmented);
    }

    /// Attempts to collect all fragments from a given fragment id.
    /// Will fail if not all fragments are specified.
    pub fn collect(&mut self, id: u16) -> Result<Vec<u8>, FragmentQueueError> {
        if let Some((size, frames)) = self.fragments.get_mut(&id) {
            if *size == frames.len() as u32 {
                // sort all frames by id,
                // because we now have all frames.
                frames.sort_by(|a, b| {
                    a.fragment_meta
                        .as_ref()
                        .unwrap()
                        .index
                        .cmp(&b.fragment_meta.as_ref().unwrap().index)
                });

                let mut buffer = Vec::<u8>::new();

                for frame in frames.iter() {
                    buffer.extend_from_slice(&frame.body);
                }

                self.fragments.remove(&id);
                self.fragment_updated_at.remove(&id);
                return Ok(buffer);
            }
            return Err(FragmentQueueError::FragmentsMissing);
        }

        return Err(FragmentQueueError::FragmentInvalid);
    }

    /// This will split a given frame into a bunch of smaller frames within the specified
    /// restriction.
    pub fn split_insert(&mut self, buffer: &[u8], mtu: u16) -> Result<u16, FragmentQueueError> {
        self.fragment_id = self.fragment_id.wrapping_add(1);

        let id = self.fragment_id;

        if self.fragments.contains_key(&id) {
            self.fragments.remove(&id);
            self.fragment_updated_at.remove(&id);
        }

        if let Ok(frames) = Self::split(buffer, id, mtu) {
            self.fragments.insert(id, (frames.len() as u32, frames));
            self.fragment_updated_at.insert(id, current_epoch());
            return Ok(id);
        }

        return Err(FragmentQueueError::DoesNotNeedSplit);
    }

    pub fn split(buffer: &[u8], id: u16, mtu: u16) -> Result<Vec<Frame>, FragmentQueueError> {
        let max_mtu = fragment_body_limit(mtu);

        if buffer.len() > max_mtu.into() {
            let splits = buffer
                .chunks(max_mtu.into())
                .map(|c| c.to_vec())
                .collect::<Vec<Vec<u8>>>();
            let mut frames: Vec<Frame> = Vec::new();
            let mut index: u32 = 0;

            for buf in splits.iter() {
                let mut f = Frame::new(Reliability::ReliableOrd, Some(&buf[..]));
                f.fragment_meta = Some(FragmentMeta {
                    index,
                    size: splits.len() as u32,
                    id,
                });

                index += 1;

                frames.push(f);
            }

            return Ok(frames);
        }

        return Err(FragmentQueueError::DoesNotNeedSplit);
    }

    pub fn get(&self, id: &u16) -> Result<&(u32, Vec<Frame>), FragmentQueueError> {
        if let Some(v) = self.fragments.get(id) {
            return Ok(v);
        }

        return Err(FragmentQueueError::FragmentInvalid);
    }

    pub fn get_mut(&mut self, id: &u16) -> Result<&mut (u32, Vec<Frame>), FragmentQueueError> {
        if let Some(v) = self.fragments.get_mut(id) {
            return Ok(v);
        }

        return Err(FragmentQueueError::FragmentInvalid);
    }

    pub fn remove(&mut self, id: &u16) -> bool {
        self.fragment_updated_at.remove(id);
        self.fragments.remove(id).is_some()
    }

    pub fn pending_counts(&self) -> (usize, usize) {
        let sets = self.fragments.len();
        let fragments = self
            .fragments
            .values()
            .map(|(_, frames)| frames.len())
            .sum::<usize>();
        (sets, fragments)
    }

    pub fn pending_summary(&self) -> String {
        if self.fragments.is_empty() {
            return "[]".to_string();
        }

        let mut parts = self
            .fragments
            .iter()
            .map(|(id, (size, frames))| {
                let age = self
                    .fragment_updated_at
                    .get(id)
                    .map(|updated| current_epoch().saturating_sub(*updated))
                    .unwrap_or(0);
                let present = frames
                    .iter()
                    .filter_map(|frame| frame.fragment_meta.as_ref().map(|meta| meta.index))
                    .collect::<Vec<_>>();
                let missing = (0..*size)
                    .filter(|index| !present.contains(index))
                    .take(12)
                    .collect::<Vec<_>>();
                format!(
                    "split_id={} {}/{} age={}s missing_first={:?}",
                    id,
                    frames.len(),
                    size,
                    age,
                    missing
                )
            })
            .collect::<Vec<_>>();
        parts.sort();
        format!("[{}]", parts.join(";"))
    }

    pub fn expire_stale(&mut self, threshold: u64) -> Vec<ExpiredFragment> {
        let now = current_epoch();
        let stale = self
            .fragment_updated_at
            .iter()
            .filter_map(|(id, updated)| ((*updated + threshold) < now).then_some(*id))
            .collect::<Vec<_>>();

        let mut expired = Vec::new();
        for id in stale {
            self.fragment_updated_at.remove(&id);
            if let Some((size, frames)) = self.fragments.remove(&id) {
                let first = frames
                    .first()
                    .and_then(|frame| frame.fragment_meta.as_ref().map(|_| frame));
                expired.push(ExpiredFragment {
                    id,
                    size,
                    count: frames.len(),
                    order_channel: first.and_then(|frame| frame.order_channel),
                    order_index: first.and_then(|frame| frame.order_index),
                });
            }
        }

        expired
    }

    /// This will hard clear the fragment queue, this should only be used if memory becomes an issue!
    pub fn clear(&mut self) {
        self.fragment_id = 0;
        self.fragments.clear();
        self.fragment_updated_at.clear();
    }
}

pub(crate) fn fragment_body_limit(mtu: u16) -> u16 {
    let wire_limit = mtu.saturating_sub(RAKNET_HEADER_FRAME_OVERHEAD).max(1);
    let default_limit = wire_limit.min(1000);
    let Some(limit) = std::env::var("RAKNET_FRAGMENT_BODY_LIMIT")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
    else {
        return default_limit;
    };

    limit.clamp(1, wire_limit)
}

#[cfg(test)]
mod tests {
    use super::{fragment_body_limit, FragmentQueue};

    #[test]
    fn fragment_body_limit_defaults_to_bedrock_safe_size() {
        assert_eq!(fragment_body_limit(1386), 1000);
    }

    #[test]
    fn login_sized_payload_uses_conservative_fragment_count() {
        let payload = vec![0xfe; 27_571];
        let frames = FragmentQueue::split(&payload, 1, 1386).expect("split login-sized payload");

        assert_eq!(frames.len(), 28);
        assert!(frames[..27].iter().all(|frame| frame.body.len() == 1000));
        assert_eq!(frames[27].body.len(), 571);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FragmentQueueError {
    FrameExists,
    FrameNotFragmented,
    DoesNotNeedSplit,
    FragmentInvalid,
    FragmentsMissing,
    FrameIndexOutOfBounds,
}

impl std::fmt::Display for FragmentQueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                FragmentQueueError::FrameExists => "Frame already exists",
                FragmentQueueError::FrameNotFragmented => "Frame is not fragmented",
                FragmentQueueError::DoesNotNeedSplit => "Frame does not need to be split",
                FragmentQueueError::FragmentInvalid => "Fragment is invalid",
                FragmentQueueError::FragmentsMissing => "Fragments are missing",
                FragmentQueueError::FrameIndexOutOfBounds => "Frame index is out of bounds",
            }
        )
    }
}

impl std::error::Error for FragmentQueueError {}
