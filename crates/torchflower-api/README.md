# torchflower-api

Authenticated Axum REST API wrapper for TorchFlower engine state and bot management.

Dependencies: `torchflower-engine`.

```rust
use torchflower_api::ApiConfig;

fn takes_config(config: ApiConfig) {
    let _ = config.bind;
}
```
