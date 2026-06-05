# torchflower-engine

Bot session engine, gameplay validation, persistence, diagnostics, and high-level controllers for TorchFlower.

Dependencies: `torchflower-auth`, `torchflower-proto`, `torchflower-net`, Bedrock protocol crates, SQLx SQLite, Axum, Tokio, and tracing.

```rust
use torchflower_engine::core::{BotSession, ServerAddress};

let server = ServerAddress::new("play.example.com", 19132);
let _builder = BotSession::builder().server(server);
```
