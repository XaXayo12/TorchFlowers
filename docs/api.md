# Public Bot API

The public Rust API is exposed from `torchflower_engine::core`.

Core types:

- `BotSession`
- `BotSessionBuilder`
- `ServerAddress`
- `Position`
- `Rotation`
- `BlockPosition`
- `BotEvent`
- `BotError`
- `AutomationPolicy`

Controllers:

- `MovementController`
- `InventoryTracker`
- `BlockInteractionController`
- `KeepAliveController`
- `ServerStateTracker`
- `ActionScheduler`

Example shape:

```rust
use torchflower_engine::core::{BotSession, ServerAddress};

let mut bot = BotSession::builder()
    .config(config)
    .database(db)
    .account(account_id)
    .server(ServerAddress::new("example.org", 19132))
    .build()
    .await?;

let status = bot.connect().await?;
```

The current session backend uses the validated real-server transport while persistent live command dispatch is separated from the validation loop.
