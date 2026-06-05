# Contributing

## Build

```powershell
cargo build --workspace
```

## Tests

```powershell
cargo test --workspace
```

Real-server tests are ignored by default. Use a local Bedrock Dedicated Server or permissioned test server and set the required environment variables before running ignored integration tests.

## Formatting And Lints

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

## Crate Architecture

- `torchflower`: public facade crate.
- `torchflower-auth`: storage-agnostic authentication types and flow entry points.
- `torchflower-proto`: packet models, forms, codecs, and protocol version abstraction.
- `torchflower-net`: RakNet transport, ACK/NACK, fragmentation, and reassembly.
- `torchflower-engine`: bot sessions, persistence, diagnostics, validation, and gameplay actions.
- `torchflower-api`: authenticated REST API wrapper.
- `torchflower-level`: Bedrock world-folder utilities.
- `torchflower-addon`: addon manifest and pack helpers.

## Local BDS Integration Testing

1. Start a local BDS or compatible server that you own or have permission to test.
2. Configure the server for the auth mode you intend to validate.
3. Set `BEDROCK_VALIDATE_SERVER_HOST`, `BEDROCK_VALIDATE_SERVER_PORT`, and the account environment variables.
4. Run the ignored real-server validation test or the `validate-real-server` command.

## Pull Requests

- Keep one feature or fix per PR.
- Add tests for new behavior.
- Preserve existing security defaults.
- Do not add anticheat bypass or evasion behavior.
- `cargo fmt`, `cargo clippy`, and `cargo test` must pass before review.
