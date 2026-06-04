# TorchFlower Bedrock Engine

TorchFlower is a Rust Minecraft Bedrock client engine for authenticated real-server validation and bot-session experiments. The repository is intentionally Rust-only: authentication, entitlement provisioning, Bedrock networking, persistence, diagnostics, and validation all live in the engine.

## Workspace

- Engine crate: `bots/bedrock-engine`
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

Tokens and provisioning state are persisted in SQLite. Diagnostics record request status, response metadata, and authentication-stage failures.

## Bedrock Client Capabilities

The current engine covers the critical client pipeline used by modern Bedrock servers:

- RakNet handshake, ACK/NACK handling, fragmentation, and reassembly
- RequestNetworkSettings and ZLib negotiation
- Login packet generation, compression, encryption handshake, and encrypted batch decoding
- Resource pack acknowledgement and client cache status
- StartGame, player spawn, keepalive, chat, inventory observation, movement, and disconnect handling
- DonutSMP-compatible NetworkStackLatency response encoding
- Server-confirmed block breaking evidence through UpdateBlock observation
- Guarded block placing that only runs after a normal placeable item is confirmed in inventory

Server menu, teleport, region-selector, and UI-tool items are rejected as placeable inventory even when their raw item id resembles a block. If a server does not provide a normal collectible drop, the validator reports block-breaking evidence separately and fails placement explicitly instead of producing a false positive.

## Configuration

Copy `.env.example` to `.env` and set values as needed:

```powershell
MICROSOFT_AUTH_FLOW=live
MICROSOFT_CLIENT_ID=
TOKEN_ENCRYPTION_SECRET=replace-with-32-plus-random-characters
DATABASE_URL=sqlite://database/torchflower.sqlite
RUST_ENGINE_BIND=127.0.0.1:9080
LOG_LEVEL=info
BEDROCK_VALIDATE_ACCOUNT_ID=<account-id>
BEDROCK_VALIDATE_SERVER_HOST=<server-host>
BEDROCK_VALIDATE_SERVER_PORT=19132
BEDROCK_VALIDATE_DURATION_SECONDS=300
```

`MICROSOFT_CLIENT_ID` is optional in Live mode. `TOKEN_ENCRYPTION_SECRET` should be a high-entropy local secret used to encrypt stored refresh tokens.

## Run

Start the local engine API:

```powershell
cargo run -p bedrock-engine
```

Run real-server validation:

```powershell
$env:BEDROCK_VALIDATE_ACCOUNT_ID="<account-id>"
$env:BEDROCK_VALIDATE_SERVER_HOST="<server-host>"
$env:BEDROCK_VALIDATE_SERVER_PORT="19132"
$env:BEDROCK_VALIDATE_DURATION_SECONDS="90"
cargo run -p bedrock-engine -- validate-real-server
```

Expected successful gameplay output includes:

- `remained_connected=true`
- `disconnect_reason=null`
- `block_breaking=true`
- `block_placing=true`
- `gameplay_actions=true`
- `missing_capabilities=[]`

If block placing fails, inspect `[GAMEPLAY_PICKUP]` and `[GAMEPLAY_INVENTORY]` lines. Normal failures distinguish missing drops from rejected server UI items.

## Verification

```powershell
cargo test -p bedrock-engine
cargo build -p bedrock-engine
```
