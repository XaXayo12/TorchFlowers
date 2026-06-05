# Async And Blocking Audit

TorchFlower uses Tokio for runtime work. This audit records blocking or risky patterns found during the modularization pass.

| File | Line | Finding | Fix applied | Blocking remains |
|---|---:|---|---|---|
| `crates/torchflower-engine/src/bedrock/protocol_adapter.rs` | 3270 | `std::fs::read_to_string` loads an optional override login payload from disk. | Left isolated because it is a one-time debug/config read before packet generation, not a hot-path bot loop. | Yes, intentionally. Convert to `tokio::fs::read_to_string` when this path becomes part of long-running runtime control. |
| `crates/torchflower-net/src/client/*.rs` | multiple | `Mutex`/`RwLock` imports are conditional transport internals. Tokio builds use `tokio::sync::{Mutex,RwLock}`. | Confirmed async build uses Tokio locks. | No std lock held across `.await` in the Tokio build path. |
| `crates/torchflower-net/src/connection/*.rs` | multiple | `Mutex`/`RwLock` imports are conditional transport internals. Tokio builds use `tokio::sync::{Mutex,RwLock}`. | Confirmed async build uses Tokio locks. | No std lock held across `.await` in the Tokio build path. |
| `crates/torchflower-addon/src/lib.rs` | multiple | Synchronous filesystem and zip archive APIs. | Left synchronous because addon pack read/write APIs are direct library utilities, not async bot runtime tasks. | Yes, intentionally outside Tokio bot hot path. |
| `crates/torchflower-level/src/lib.rs` | multiple | Synchronous filesystem reads for world folders. | Left synchronous because the level crate exposes a direct disk API. Async callers should use `spawn_blocking` around world scans. | Yes, intentionally outside bot networking tasks. |

No `std::thread::sleep`, `reqwest::blocking`, or `std::sync::Mutex` held across async awaits were added in production bot/runtime code during this pass.
