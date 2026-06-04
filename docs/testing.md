# Testing TorchFlower

Run the normal local suite:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Security checks:

```powershell
cargo audit
cargo deny check
```

Install tools when needed:

```powershell
cargo install cargo-audit --locked
cargo install cargo-deny --locked
```

Real-server validation is opt-in and requires a provisioned account:

```powershell
$env:BEDROCK_VALIDATE_ACCOUNT_ID="<account-id>"
$env:BEDROCK_VALIDATE_SERVER_HOST="<authorized-host>"
$env:BEDROCK_VALIDATE_SERVER_PORT="19132"
$env:BEDROCK_VALIDATE_DURATION_SECONDS="90"
cargo run -p torchflower-engine -- validate-real-server
```

Ignored real-BDS tests should require explicit environment variables and must never run in normal CI.
