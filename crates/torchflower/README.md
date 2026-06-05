# torchflower

Facade crate that re-exports the public TorchFlower auth, protocol, network, and engine APIs under one dependency.

Dependencies: `torchflower-auth`, `torchflower-proto`, `torchflower-net`, and `torchflower-engine`; optional `torchflower-level` and `torchflower-addon`.

```rust
use torchflower::{AuthConfig, BotBuilder, ProtocolVersion};

# async fn demo() -> Result<(), Box<dyn std::error::Error>> {
let bot = BotBuilder::new()
    .address("play.example.com", 19132)
    .protocol_version(ProtocolVersion::V1_21_100)
    .auth(AuthConfig::device_code())
    .build()
    .await?;
# Ok(())
# }
```
