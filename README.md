# RustRock Bedrock Discord Bot

RustRock is a production-oriented Minecraft Bedrock bot controller. Operators manage accounts, entitlement provisioning, servers, bot sessions, diagnostics, and real-server validation from Discord slash commands.

There is no web dashboard in this build. The Node.js TypeScript service is a Discord bot command surface, and the Rust engine remains the source of truth for authentication, persistence, entitlement provisioning, and Bedrock networking.

## Workspace

- Rust engine: `bots/bedrock-engine`
- Discord command service: `server`
- Shared TypeScript contracts: `shared`
- SQLite schema: `database/migrations/0001_initial.sql`

## Rust Engine

The engine implements the complete Bedrock entitlement and authentication path:

1. Microsoft device-code OAuth
2. Xbox Live authentication
3. Standard XSTS authentication
4. PlayFab XSTS authentication with `RP=http://playfab.xboxlive.com/`
5. PlayFab `LoginWithXbox` against `https://20ca2.playfabapi.com/Client/LoginWithXbox` with `CreateAccount=true`
6. Minecraft entitlement session initialization against `https://authorization.franchise.minecraft-services.net/api/v1.0/session/start`
7. Legacy Bedrock authentication against `https://multiplayer.minecraft.net/authentication`
8. Bedrock JWT chain generation

`bedrock-rs` is pinned and used for packet definitions, protocol versions, serialization, compression, and codec surfaces where available. `ismaileke/bedrock-client` was inspected for client-loop and packet-coverage ideas, but it is not used as a dependency because it brings a separate RakNet/protocol implementation and its own authentication path. Any missing production behavior remains behind local RustRock adapters.

## Discord Commands

Register and run the Discord bot with:

```powershell
npm run dev --workspace server
```

Available command group:

- `/rustrock status`
- `/rustrock import`
- `/rustrock poll-auth`
- `/rustrock provision`
- `/rustrock accounts`
- `/rustrock add-server`
- `/rustrock servers`
- `/rustrock create-bot`
- `/rustrock bots`
- `/rustrock start-bot`
- `/rustrock stop-bot`
- `/rustrock validate`
- `/rustrock logs`

## Configuration

Copy `.env.example` to `.env` and set:

- `MICROSOFT_CLIENT_ID`
- `TOKEN_ENCRYPTION_SECRET`
- `DATABASE_URL`
- `RUST_ENGINE_BIND`
- `RUST_ENGINE_URL`
- `DISCORD_BOT_TOKEN`
- `DISCORD_CLIENT_ID`
- `DISCORD_GUILD_ID` for fast guild command registration during development
- `DISCORD_ALLOWED_ROLE_IDS` or `DISCORD_ADMIN_USER_IDS` for operator access

## Live Validation

The Discord command `/rustrock validate` and the Rust CLI both exercise the real-server probes for login, spawn, keepalive, chat, forms, inventory transactions, movement, and disconnect handling.

CLI validation:

```powershell
cargo run -p bedrock-engine -- validate-real-server
```

Set `BEDROCK_VALIDATE_ACCOUNT_ID`, `BEDROCK_VALIDATE_SERVER_HOST`, and `BEDROCK_VALIDATE_SERVER_PORT` before running the CLI.

## Verification

```powershell
cargo test -p bedrock-engine
npm run build
npm run test
```
