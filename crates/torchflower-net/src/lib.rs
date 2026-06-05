#![allow(unknown_lints)]
#![allow(warnings)]
#![allow(clippy::all)]

//! # torchflower-net
//!
//! A fully functional RakNet implementation in pure rust, asynchronously driven.
//!
//! ## Getting Started
//!
//! RakNet (torchflower-net) is available on [crates.io](https://crates.io/crates/torchflower-net), to use it, add the following to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! torchflower-net = "0.3.3"
//! ```
//!
//! ## Features
//!
//! This RakNet implementation comes with 3 primary features, `async_std`, `async_tokio` and `mcpe`.  However, by default, only `async_std` is enabled, and `mcpe` requires you to modify your `Cargo.toml`.
//!
//! If you wish to use these features, add them to your `Cargo.toml` as seen below:
//!
//! ```toml
//! [dependencies]
//! torchflower-net = { version = "0.3.3", default-features = false, features = [ "async_tokio", "mcpe" ] }
//! ```
//!
//!
//!
//! torchflower-net also provides the following modules:
//!
//! - [`torchflower_net::client`](crate::client) - A client implementation of RakNet, allowing you to connect to a RakNet server.
//! - [`torchflower_net::connection`](crate::connection) - A bare-bones implementation of a Raknet peer, this is mainly used for types.
//! - [`torchflower_net::error`](crate::error) - A module with errors that both the Client and Server can respond with.
//! - [`torchflower_net::protocol`](crate::protocol) - A lower level implementation of RakNet, responsible for encoding and decoding packets.
//! - [`torchflower_net::server`](crate::server) - The base server implementation of RakNet.
//! - [`torchflower_net::util`](crate::util)  - General utilities used within `torchflower-net`.
//!
//! # Client
//!
//! The `client` module provides a way for you to interact with RakNet servers with code.
//!
//! **Example:**
//!
//! ```ignore
//! use torchflower_net::client::{Client, DEFAULT_MTU};
//! use std::net::ToSocketAddrs;
//!
//! #[async_std::main]
//! async fn main() {
//!     let version: u8 = 10;
//!     let mut client = Client::new(version, DEFAULT_MTU);
//!
//!     client.connect("my_server.net:19132").await.unwrap();
//!
//!     // receive packets
//!     loop {
//!         let packet = client.recv().await.unwrap();
//!
//!         println!("Received a packet! {:?}", packet);
//!
//!         client.send_ord(vec![254, 0, 1, 1], Some(1));
//!     }
//! }
//!
//! ```
//!
//! # Server
//!
//! A RakNet server implementation in pure rust.
//!
//! **Example:**
//!
//! ```ignore
//! use rakrs::connection::Connection;
//! use rakrs::Listener;
//! use rakrs::
//!
//! #[async_std::main]
//! async fn main() {
//!     let mut server = Listener::bind("0.0.0.0:19132").await.unwrap();
//!     server.start().await.unwrap();
//!
//!     loop {
//!         let conn = server.accept().await;
//!         async_std::task::spawn(handle(conn.unwrap()));
//!     }
//! }
//!
//! async fn handle(mut conn: Connection) {
//!     loop {
//!         // keeping the connection alive
//!         if conn.is_closed() {
//!             println!("Connection closed!");
//!             break;
//!         }
//!         if let Ok(pk) = conn.recv().await {
//!             println!("Got a connection packet {:?} ", pk);
//!         }
//!     }
//! }
//! ```
/// TorchFlower-specific adapter policy around RakNet behavior.
pub mod adapter;
/// A client implementation of RakNet, allowing you to connect to a RakNet server.
pub mod client;
/// The connection implementation of RakNet, allowing you to send and receive packets.
/// This is barebones, and you should use the client or server implementations instead, this is mainly
/// used internally.
pub mod connection;
/// The error implementation of RakNet, allowing you to handle errors.
pub mod error;
/// Native Bedrock/RakNet vertical slices that do not depend on external protocol crates.
pub mod native;
/// The packet implementation of RakNet.
/// This is a lower level implementation responsible for serializing and deserializing packets.
pub mod protocol;
// Server implementation of RakNet was removed.
/// Re-exports server-related types and functions.
pub mod server {
    pub use crate::util::{current_epoch, PossiblySocketAddr};
}
/// Utilties for RakNet, like epoch time.
pub mod util;

pub use protocol::mcpe::{self, motd::Motd};

/// An internal module for notifying the connection of state updates.
pub(crate) mod notify;

use std::net::SocketAddr;

use bytes::Bytes;
use protocol::reliability::Reliability;

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("RakNet client error: {0}")]
    Client(#[from] error::client::ClientError),
    #[error("RakNet receive error: {0}")]
    Recv(#[from] connection::RecvError),
    #[error("unexpected empty RakNet payload")]
    EmptyPayload,
}

pub struct Connection {
    inner: client::Client,
}

impl Connection {
    pub async fn connect(addr: SocketAddr) -> Result<Self, NetError> {
        let mut inner = client::Client::new(10, client::DEFAULT_MTU);
        inner.connect(addr.to_string().as_str()).await?;
        Ok(Self { inner })
    }

    pub async fn send(&mut self, data: Bytes) -> Result<(), NetError> {
        self.inner
            .send_immediate(data.as_ref(), Reliability::ReliableOrd, 0)
            .await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Bytes, NetError> {
        let payload = self.inner.recv().await?;
        if payload.is_empty() {
            return Err(NetError::EmptyPayload);
        }
        Ok(Bytes::from(payload))
    }

    pub async fn close(&mut self) {
        self.inner.close().await;
    }
}
