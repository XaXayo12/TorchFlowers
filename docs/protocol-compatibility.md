# Protocol Compatibility

Compatibility entries describe what the current code is expected to support. Unknown means the feature has not been validated enough to claim support.

| Protocol | Login | Encryption | Chat | Movement | Inventory | Break/Place | Validation |
| -------- | ----- | ---------- | ---- | -------- | --------- | ----------- | ---------- |
| 898 | unknown | unknown | unknown | unknown | unknown | unknown | unknown |
| 944 | unknown | unknown | unknown | unknown | unknown | unknown | unknown |
| 975 | yes | yes | yes | yes | partial | partial | yes |

Notes:

- Protocol 975 has real-server validation coverage for login, encryption, spawn, keepalive, chat observation, movement, inventory observation, and guarded block actions.
- Inventory and break/place remain partial because server policy and protected hub terrain can prevent normal drops or block updates.
- Older protocol versions are present through dependencies but need dedicated fixtures or real-server validation before being marked supported.
