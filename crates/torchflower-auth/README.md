# torchflower-auth

Storage-agnostic authentication types for Microsoft, Xbox, PlayFab, Minecraft entitlement, and Bedrock JWT flows.

Dependencies: external HTTP/serde/time crates only; no dependency on engine storage.

```rust
use torchflower_auth::{AuthConfig, AuthTokens};

let config = AuthConfig::device_code();
let empty = AuthTokens::empty();
assert!(empty.bedrock_chain.is_empty());
```
