use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use bytes::BytesMut;
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    sync::{mpsc, Semaphore},
    task::JoinHandle,
};
use uuid::Uuid;

use crate::core::BotError;

pub type AccountId = String;
pub type BotId = Uuid;
pub type ServerAddr = SocketAddr;

#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub max_concurrent: usize,
    pub max_auth_concurrent: usize,
    pub spawn_interval: Duration,
    pub command_buffer: usize,
    pub buffer_size: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 100,
            max_auth_concurrent: 3,
            spawn_interval: Duration::from_millis(500),
            command_buffer: 64,
            buffer_size: 16 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BotCommand {
    SendChat(String),
    MoveTo { x: f32, y: f32, z: f32 },
    BreakBlock { x: i32, y: i32, z: i32 },
    PlaceBlock { x: i32, y: i32, z: i32 },
    Respawn,
    Disconnect,
}

#[derive(Debug, Error)]
pub enum PoolError {
    #[error("pool capacity reached: max_concurrent={0}")]
    CapacityReached(usize),
    #[error("bot not found: {0}")]
    BotNotFound(BotId),
    #[error("bot command channel is closed: {0}")]
    CommandChannelClosed(BotId),
    #[error("bot task join failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Bot(#[from] BotError),
}

pub struct BotHandle {
    pub id: BotId,
    pub tx: mpsc::Sender<BotCommand>,
    pub task: JoinHandle<Result<(), BotError>>,
}

pub struct BotPool {
    bots: HashMap<BotId, BotHandle>,
    config: PoolConfig,
    auth_gate: Arc<Semaphore>,
    buffer_pool: Arc<BufferPool>,
}

impl BotPool {
    pub async fn new(config: PoolConfig) -> Self {
        let pool_size = config.max_concurrent.saturating_mul(4).max(1);
        Self {
            bots: HashMap::new(),
            auth_gate: Arc::new(Semaphore::new(config.max_auth_concurrent.max(1))),
            buffer_pool: Arc::new(BufferPool::new(pool_size, config.buffer_size)),
            config,
        }
    }

    pub async fn spawn(
        &mut self,
        _account: AccountId,
        _server: ServerAddr,
    ) -> Result<BotId, PoolError> {
        if self.bots.len() >= self.config.max_concurrent {
            return Err(PoolError::CapacityReached(self.config.max_concurrent));
        }

        let _auth_permit = self
            .auth_gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| PoolError::CapacityReached(self.config.max_auth_concurrent))?;
        if !self.config.spawn_interval.is_zero() {
            tokio::time::sleep(self.config.spawn_interval).await;
        }

        let id = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(self.config.command_buffer.max(1));
        let buffer_pool = self.buffer_pool.clone();
        let task = tokio::spawn(run_bot_command_loop(id, rx, buffer_pool));
        self.bots.insert(id, BotHandle { id, tx, task });
        Ok(id)
    }

    pub async fn spawn_batch(
        &mut self,
        accounts: Vec<AccountId>,
        server: ServerAddr,
    ) -> Vec<Result<BotId, PoolError>> {
        let mut results = Vec::with_capacity(accounts.len());
        for account in accounts {
            results.push(self.spawn(account, server).await);
        }
        results
    }

    pub async fn send(&self, id: BotId, cmd: BotCommand) -> Result<(), PoolError> {
        let handle = self.bots.get(&id).ok_or(PoolError::BotNotFound(id))?;
        handle
            .tx
            .send(cmd)
            .await
            .map_err(|_| PoolError::CommandChannelClosed(id))
    }

    pub async fn broadcast(&self, cmd: BotCommand) {
        for handle in self.bots.values() {
            let _ = handle.tx.send(cmd.clone()).await;
        }
    }

    pub fn active_count(&self) -> usize {
        self.bots.len()
    }

    pub fn memory_stats(&self) -> MemoryStats {
        self.buffer_pool.memory_stats(self.active_count())
    }

    pub async fn shutdown_all(&mut self) {
        for handle in self.bots.values() {
            let _ = handle.tx.send(BotCommand::Disconnect).await;
        }
        let handles = std::mem::take(&mut self.bots);
        for handle in handles.into_values() {
            let _ = handle.task.await;
        }
    }
}

async fn run_bot_command_loop(
    _id: BotId,
    mut rx: mpsc::Receiver<BotCommand>,
    buffer_pool: Arc<BufferPool>,
) -> Result<(), BotError> {
    while let Some(command) = rx.recv().await {
        let mut scratch = buffer_pool.get();
        scratch.buf.clear();
        match command {
            BotCommand::SendChat(message) => scratch.buf.extend_from_slice(message.as_bytes()),
            BotCommand::MoveTo { x, y, z } => {
                scratch.buf.extend_from_slice(&x.to_le_bytes());
                scratch.buf.extend_from_slice(&y.to_le_bytes());
                scratch.buf.extend_from_slice(&z.to_le_bytes());
            }
            BotCommand::BreakBlock { x, y, z } | BotCommand::PlaceBlock { x, y, z } => {
                scratch.buf.extend_from_slice(&x.to_le_bytes());
                scratch.buf.extend_from_slice(&y.to_le_bytes());
                scratch.buf.extend_from_slice(&z.to_le_bytes());
            }
            BotCommand::Respawn => scratch.buf.extend_from_slice(b"respawn"),
            BotCommand::Disconnect => break,
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct BufferPool {
    pool: ArrayQueue<BytesMut>,
    buf_size: usize,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl BufferPool {
    pub fn new(pool_size: usize, buf_size: usize) -> Self {
        let pool = ArrayQueue::new(pool_size.max(1));
        for _ in 0..pool_size.max(1) {
            let _ = pool.push(BytesMut::with_capacity(buf_size));
        }
        Self {
            pool,
            buf_size,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    pub fn get(self: &Arc<Self>) -> PooledBuffer {
        let buf = match self.pool.pop() {
            Some(buf) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                buf
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                BytesMut::with_capacity(self.buf_size)
            }
        };
        PooledBuffer {
            buf,
            pool: self.clone(),
        }
    }

    pub fn memory_stats(&self, bots_active: usize) -> MemoryStats {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let total = hits + misses;
        let hit_rate = if total == 0 {
            1.0
        } else {
            hits as f64 / total as f64
        };
        MemoryStats {
            bots_active,
            heap_bytes_estimated: bots_active
                .saturating_mul(128 * 1024)
                .saturating_add(self.pool.capacity().saturating_mul(self.buf_size)),
            buffer_pool_hit_rate: hit_rate,
            buffer_pool_misses: misses,
        }
    }
}

pub struct PooledBuffer {
    pub buf: BytesMut,
    pool: Arc<BufferPool>,
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        if self.buf.capacity() > self.pool.buf_size.saturating_mul(4) {
            self.buf = BytesMut::with_capacity(self.pool.buf_size);
        }
        self.buf.clear();
        let mut returned = BytesMut::new();
        std::mem::swap(&mut returned, &mut self.buf);
        let _ = self.pool.pool.push(returned);
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct MemoryStats {
    pub bots_active: usize,
    pub heap_bytes_estimated: usize,
    pub buffer_pool_hit_rate: f64,
    pub buffer_pool_misses: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_batch_respects_capacity() {
        let config = PoolConfig {
            max_concurrent: 2,
            max_auth_concurrent: 1,
            spawn_interval: Duration::ZERO,
            command_buffer: 4,
            buffer_size: 128,
        };
        let mut pool = BotPool::new(config).await;
        let server: ServerAddr = "127.0.0.1:19132".parse().unwrap();
        let results = pool
            .spawn_batch(vec!["a".into(), "b".into(), "c".into()], server)
            .await;
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 2);
        assert_eq!(pool.active_count(), 2);
        pool.shutdown_all().await;
    }

    #[tokio::test]
    async fn commands_route_through_channels() {
        let mut pool = BotPool::new(PoolConfig {
            spawn_interval: Duration::ZERO,
            ..PoolConfig::default()
        })
        .await;
        let server: ServerAddr = "127.0.0.1:19132".parse().unwrap();
        let id = pool.spawn("account".into(), server).await.unwrap();
        pool.send(id, BotCommand::SendChat("hello".to_string()))
            .await
            .unwrap();
        pool.shutdown_all().await;
    }

    #[test]
    fn memory_stats_are_sane() {
        let pool = Arc::new(BufferPool::new(4, 1024));
        let buffer = pool.get();
        drop(buffer);
        let stats = pool.memory_stats(3);
        assert_eq!(stats.bots_active, 3);
        assert!(stats.heap_bytes_estimated >= 4 * 1024);
        assert!(stats.buffer_pool_hit_rate > 0.0);
    }
}
