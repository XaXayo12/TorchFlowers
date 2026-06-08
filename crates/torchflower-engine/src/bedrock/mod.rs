//! Bedrock compatibility namespace.
//!
//! The active engine build uses TorchFlower's native network layer. Historical
//! typed protocol experiments remain in the repository for reference, but they
//! are not part of the public build surface until they can be implemented
//! without external Bedrock protocol crates.

pub mod local_network;
pub mod protocol_adapter;
pub mod session;
pub mod transport;
