# ⚠️ THIS IS STILL UNDER DEVELOPEMENT!!!

# TorchFlower Bedrock Engine

TorchFlower is a Rust Minecraft Bedrock client engine for authenticated real-server validation and bot-session experiments. The repository is intentionally Rust-only: authentication, entitlement provisioning, Bedrock networking, persistence, diagnostics, validation, and the public bot API all live in the engine.

## Workspace

- Engine crate: `bots/bedrock-engine`
- Public Rust API: `torchflower_engine::core`
- SQLite schema: `database/migrations/0001_initial.sql`
- Local RakNet patch: `vendor/rak-rs`

## Authentication

The engine implements the full Bedrock account path:

1. Microsoft device-code OAuth using the Live flow by default
2. Xbox Live authentication
3. Standard XSTS authentication
4. PlayFab XSTS authentication with `RP=http://playfab.xboxlive.com/`
5. PlayFab `LoginWithXbox` against `https://20ca2.playfabapi.com/Client/LoginWithXbox` with `CreateAccount=true`
6. Minecraft entitlement session initialization against `https://authorization.franchise.minecraft-services.net/api/v1.0/session/start`
7. Legacy Bedrock authentication against `https://multiplayer.minecraft.net/authentication`
8. Bedrock JWT chain generation

Tokens and provisioning state are persisted in SQLite.

## Capabilities

- RakNet handshake, ACK/NACK handling, fragmentation, and reassembly
- RequestNetworkSettings and ZLib negotiation
- Login packet generation, compression, encryption handshake, and encrypted batch decoding
- Resource pack acknowledgement and client cache status
- StartGame, player spawn, keepalive, chat, inventory observation, movement, and disconnect handling
- DonutSMP-compatible NetworkStackLatency response encoding
- Server-confirmed block breaking evidence through UpdateBlock observation
- Guarded block placing after a normal placeable item is confirmed in inventory
- Public session/controller API for scheduling chat, movement, inventory, interaction, block, respawn, and state-tracking actions

Server menu, teleport, region-selector, and UI-tool items are rejected as placeable inventory even when their raw item id resembles a block.

## Security Defaults

- `/health` is public; every `/api/*` route requires `TORCHFLOWER_API_KEY`.
- Unauthenticated API mode is only allowed when `TORCHFLOWER_DEV_ALLOW_UNAUTH_API=true` and the bind address is loopback.
- CORS allows only exact origins from `TORCHFLOWER_CORS_ALLOWED_ORIGINS`.
- Direct real-server validation by host requires `TORCHFLOWER_ALLOWED_SERVER_HOSTS`; validation by stored `server_id` remains allowed.
- Token storage prefers `TOKEN_ENCRYPTION_KEY_B64`, a base64-encoded 32-byte key.
- Auth HTTP diagnostics do not store request/response bodies unless `TORCHFLOWER_DANGEROUS_LOG_AUTH_BODIES=true`; sensitive fields are still redacted.

## Configuration

Copy `.env.example` to `.env` and set values as needed:

```powershell
MICROSOFT_AUTH_FLOW=live
MICROSOFT_CLIENT_ID=
TOKEN_ENCRYPTION_KEY_B64=replace-with-base64-encoded-32-random-bytes
DATABASE_URL=sqlite://database/torchflower.sqlite
RUST_ENGINE_BIND=127.0.0.1:9080
LOG_LEVEL=info
TORCHFLOWER_API_KEY=replace-with-random-api-key
TORCHFLOWER_CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000
TORCHFLOWER_ALLOWED_SERVER_HOSTS=example.org
BEDROCK_VALIDATE_ACCOUNT_ID=<account-id>
BEDROCK_VALIDATE_SERVER_HOST=<server-host>
BEDROCK_VALIDATE_SERVER_PORT=19132
BEDROCK_VALIDATE_DURATION_SECONDS=300
```

`MICROSOFT_CLIENT_ID` is optional in Live mode. Keep `TOKEN_ENCRYPTION_KEY_B64` stable for the database; changing it makes stored token ciphertext undecryptable.

## Run

Start the local engine API:

```powershell
cargo run -p torchflower-engine
```

Run real-server validation:

```powershell
$env:BEDROCK_VALIDATE_ACCOUNT_ID="<account-id>"
$env:BEDROCK_VALIDATE_SERVER_HOST="<server-host>"
$env:BEDROCK_VALIDATE_SERVER_PORT="19132"
$env:BEDROCK_VALIDATE_DURATION_SECONDS="90"
cargo run -p torchflower-engine -- validate-real-server
```

Expected successful gameplay output includes:

- `remained_connected=true`
- `disconnect_reason=null`
- `block_breaking=true`
- `block_placing=true`
- `gameplay_actions=true`
- `missing_capabilities=[]`

If block placing fails, inspect `[GAMEPLAY_PICKUP]` and `[GAMEPLAY_INVENTORY]` lines. Normal failures distinguish missing drops from rejected server UI items.

## Documentation

- `docs/architecture.md`
- `docs/security.md`
- `docs/protocol-compatibility.md`
- `docs/testing.md`
- `docs/upstreaming-bedrock-rs.md`
- `docs/api.md`

## Examples

Examples live under `bots/bedrock-engine/examples` and use environment variables instead of hardcoded secrets:

- `login.rs`
- `connect.rs`
- `chat.rs`
- `move_to_block.rs`
- `break_and_pickup.rs`
- `place_block.rs`
- `multi_bot_supervisor.rs`
- `local_api_with_auth.rs`

## Verification

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build -p torchflower-engine
```
