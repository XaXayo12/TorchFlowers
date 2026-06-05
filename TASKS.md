# TorchFlower Tasks

This tracker is for implementation work that remains after the current real-server validation milestone. Items are intentionally concrete so progress can be audited from tests and logs.

## Current Focus

- [x] Allow the gameplay validator to walk toward farther normal item/drop targets before marking pickup failure terminal.
- [x] Retry multiple normal observed item entities before failing pickup.
- [x] Send `MovePlayer` plus `PlayerAuthInput` during pickup movement.
- [ ] DonutSMP gameplay validation: document current public-server blocker and avoid treating menu/RTP items as valid placeables.
- [ ] Add a local/owned/permissioned server validation target with known collectible normal blocks/items.
- [ ] Confirm server-accepted block placing from a normal collected/equipped item.
- [ ] Keep server menu, RTP, region selector, and UI-tool items excluded from placeable inventory.

## Runtime Client

- [ ] Promote validation-only session behavior into reusable `BotSession` controller methods.
- [ ] Add persistent session supervision for long-running bots.
- [ ] Add reconnect policy with bounded retry/backoff and explicit stop controls.
- [ ] Add event streams for login, spawn, movement, chat, inventory, block, form, disconnect, and reconnect events.
- [ ] Add stable typed errors for authentication, transport, protocol, policy, and gameplay actions.

## Public API

- [ ] Expose reusable high-level types: `BotSession`, `BotSessionBuilder`, `ServerAddress`, `Position`, `Rotation`, `BlockPosition`, `BotEvent`, and `BotError`.
- [ ] Expose controllers for movement, inventory, block interaction, keepalive, server state, and action scheduling.
- [ ] Route existing validation through the public controllers instead of one-off validation code.
- [ ] Keep low-level packet and transport types behind internal adapter modules.

## Protocol Coverage

- [ ] Preserve current login, encryption, compression, reassembly, StartGame, UpdateSoftEnum, movement, chat, inventory, and NetworkStackLatency behavior.
- [ ] Add fixture tests for packet decode boundary alignment.
- [ ] Add fake-server tests for connect, chat, movement, keepalive, inventory observation, block actions, disconnect, and reconnect.
- [ ] Keep real-server tests ignored by default and gated by explicit environment variables.

## Security

- [ ] Keep `/health` public and require API authentication for `/api/*`.
- [ ] Keep exact-origin CORS only.
- [ ] Continue using strong token-encryption key validation.
- [ ] Keep diagnostics redacted by default.
- [ ] Add regression tests for auth rejection, origin rejection, unsafe dev configuration, and sensitive log redaction.
- [ ] Keep automation policy explicit for break, place, attack, and interact actions.
- [ ] Do not implement anticheat bypass or evasion behavior.

## Documentation

- [ ] Keep `README.md` current with verified capabilities only.
- [ ] Expand `docs/api.md` for authenticated API examples.
- [ ] Expand `docs/testing.md` with fake-server and ignored real-server workflows.
- [ ] Maintain `docs/protocol-compatibility.md` with observed protocol versions and capability status.
- [ ] Document how local protocol patches are isolated and reviewed.
