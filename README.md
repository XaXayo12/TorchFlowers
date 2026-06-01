<<<<<<< HEAD
# RustRock Bedrock Bot Panel

Production-oriented Minecraft Bedrock bot panel with a Rust authentication and networking engine, Express API, and Next.js dashboard.

The Rust engine owns Microsoft OAuth, Xbox Live, XSTS, PlayFab provisioning, Minecraft entitlement session start, legacy Bedrock authentication, Bedrock JWT chain creation, and Bedrock network sessions. `bedrock-rs` is pinned and used for protocol, codec, compression, encryption, and network surfaces where available. The missing client dialer is implemented behind local adapters using the same `rak-rs` transport foundation used by `bedrock-rs`.

## Services

- Rust engine: `bots/bedrock-engine`
- Express API: `server`
- Next.js dashboard: `client`
- Shared TypeScript contracts: `shared`
- SQLite schema: `database/migrations/0001_initial.sql`

## Live Validation

Set `BEDROCK_VALIDATE_ACCOUNT_ID`, `BEDROCK_VALIDATE_SERVER_HOST`, and `BEDROCK_VALIDATE_SERVER_PORT`, then run:

```powershell
cargo run -p bedrock-engine -- validate-real-server
```

The validation harness attempts login, spawn, keepalive, chat, forms, inventory transactions, movement, and disconnect handling against the configured server using the selected account.

=======
# RustBedrockProtocol
>>>>>>> 0266c6ae0171bd40a30e5bf93c33a8b04894dc99
