# TorchFlower Architecture

TorchFlower is a single Rust workspace centered on `torchflower-engine`.

The engine is split into four layers:

- `auth`: Microsoft, Xbox, XSTS, PlayFab, entitlement, legacy Bedrock auth, and encrypted token storage.
- `bedrock`: low-level protocol adapters, packet encoding/decoding, transport, and the existing real-server validation loop.
- `core`: public bot API, controllers, state trackers, safe automation policy, and action scheduling.
- `api` and `bot`: authenticated HTTP control surface and multi-bot supervision.

Protocol work should stay in `bedrock` unless it belongs upstream in `bedrock-rs`. Bot behavior, orchestration, scheduling, and state policy belong in `core`.

The current `BotSession` public API wraps the proven validation transport. Persistent live socket control is intentionally isolated behind controllers so it can be added without exposing packet internals.
