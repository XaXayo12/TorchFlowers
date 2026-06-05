//! Bedrock networking adapters.
//!
//! The engine keeps `bedrock-rs` as the protocol foundation because it exposes
//! the protocol-version and packet serialization layers required by this
//! workspace. `ismaileke/bedrock-client` was inspected as an alternate client
//! reference, but it ships its own RakNet/protocol stack and a separate
//! `minecraft-auth` flow, so it is not wired as a dependency here. Behaviors
//! that `bedrock-rs` does not expose as stable client APIs are contained behind
//! these local adapters.

pub mod local_network;
pub mod protocol_adapter;
pub mod session;
pub mod transport;
