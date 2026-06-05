# Upstreaming Protocol Work

TorchFlower should not permanently fork protocol behavior when the fix belongs in `bedrock-rs`.

Use this decision rule:

- Packet schema, protocol version mapping, serialization, deserialization, compression, and encryption primitives belong upstream.
- Bot policy, state tracking, movement strategy, inventory decisions, validation scenarios, and API security belong in TorchFlower.

Completed in this pass:

- Removed the root vendored `rak-rs` patch override.
- Moved transport-facing TorchFlower behavior into `crates/torchflower-net`.
- Added `crates/torchflower-net/src/adapter.rs` for local policy that should not leak into public engine APIs.
- Removed `bedrock-rs` network feature usage from `torchflower-engine` to avoid a broken transitive crates.io `rak-rs` build while keeping `bedrock-rs` protocol definitions.

Upstream patch placeholders:

- `docs/upstream-patches/raknet-client-stability.patch`
- `docs/upstream-patches/raknet-client-stability-pr-description.md`
- `docs/upstream-patches/motd-parse-hardening.patch`
- `docs/upstream-patches/motd-parse-hardening-pr-description.md`

GitHub PR URLs:

- RakNet client stability: pending
- MOTD parse hardening: pending

Remaining TorchFlower-specific adapter code:

- `torchflower-net::adapter::AdapterPolicy` for timeout and handshake policy.
- `torchflower-engine::bedrock::local_network` for the exact compression/encryption/codec surface needed while avoiding the transitive `bedrock_network -> rak-rs` build issue.

When a local protocol adapter is required:

1. Add a minimal regression test with sanitized bytes.
2. Keep the adapter behind `bedrock::protocol_adapter`.
3. Document why the local adapter exists.
4. Prepare the smallest upstreamable patch separately.
