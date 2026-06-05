# torchflower-proto

Bedrock protocol packet types, version abstraction, form data models, and lightweight codec helpers.

Dependencies: `bytes`, `serde`, `serde_json`, compression support, and `thiserror`.

```rust
use torchflower_proto::{PacketCodec, ProtocolVersion, TextPacket};

let packet = TextPacket { source: "bot".into(), message: "hello".into() };
let bytes = packet.encode(ProtocolVersion::V1_21_100)?;
# Ok::<(), torchflower_proto::ProtoError>(())
```
