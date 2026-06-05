# torchflower-addon

Minecraft Bedrock addon and pack manifest reader/writer with `.mcpack` and `.mcaddon` archive helpers.

Dependencies: `serde`, `uuid`, and `zip`.

```rust
use torchflower_addon::{AddonManifest, ValidationWarning};

fn warnings(manifest: &AddonManifest) -> Vec<ValidationWarning> {
    manifest.validate()
}
```
