# rak-rs Change Classification

| File | Lines | Category | Description | Action | Outcome |
|------|-------|----------|-------------|--------|---------|
| `crates/torchflower-net/Cargo.toml` | package metadata | C | Renames the local transport package and declares TorchFlower metadata. | Keep in TorchFlower. | Not upstreamable. |
| `crates/torchflower-net/src/lib.rs` | public wrapper | C | Adds `Connection` and `NetError` as the stable TorchFlower API hiding transport internals. | Keep in TorchFlower adapter. | Implemented. |
| `crates/torchflower-net/src/adapter.rs` | full file | C | Adds timeout and handshake policy parsing for TorchFlower runtime behavior. | Keep as thin adapter unless upstream wants a generic configuration hook. | Implemented. |
| `crates/torchflower-net/src/client/*` | transport fixes and diagnostics | A/B | Existing validated RakNet client behavior includes handshake, split packets, ACK/NACK, and resend handling fixes needed by TorchFlower. | Prepare exact upstream patch after comparing against latest upstream main. | Tracked in `docs/upstream-patches/raknet-client-stability.*`. |
| `crates/torchflower-net/src/protocol/mcpe/motd.rs` | MOTD parsing | A | Hardened parsing avoids panics on non-standard server MOTD data. | Upstream as bug fix. | Tracked in `docs/upstream-patches/motd-parse-hardening.*`. |
| `crates/torchflower-net/tests/*` | tests | B | Fragment, frame, and ordered queue tests preserve behavior under load. | Upstream generally useful tests where applicable. | Tracked in upstream patch placeholders. |
