# torchflower-level

Minecraft Bedrock world-folder utilities for LevelDB key formats, `level.dat`, chunk handles, and player data.

Dependencies: standalone serde/error crates.

```rust
use torchflower_level::{chunk_key, Dimension};

let key = chunk_key(0, 0, Dimension::Overworld, 47);
assert_eq!(key.len(), 9);
```
