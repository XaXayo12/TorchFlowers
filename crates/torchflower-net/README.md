# torchflower-net

RakNet transport crate for TorchFlower, including the existing patched ACK/NACK, fragmentation, reassembly, and MCPE transport code.

Dependencies: no other TorchFlower crates.

```rust
use std::net::SocketAddr;
use torchflower_net::Connection;

# async fn demo(addr: SocketAddr) -> Result<(), torchflower_net::NetError> {
let mut conn = Connection::connect(addr).await?;
conn.close().await;
# Ok(())
# }
```
