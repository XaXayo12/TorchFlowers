# TorchFlower Progress

Last updated: 2026-06-05

## Verified Working

- Microsoft device-code OAuth, Xbox Live, XSTS, PlayFab provisioning, Minecraft entitlement session start, legacy Bedrock authentication, and JWT chain generation are implemented in the Rust authentication path.
- SQLite persistence tracks accounts, auth sessions, entitlements, servers, bots, and diagnostics.
- The engine API starts with authenticated `/api/*` routes and public `/health`.
- Real-server validation has reached login, encrypted packet processing, ResourcePacksInfo acknowledgement, StartGame, player spawn, keepalive, chat probe, inventory observation, movement validation, and 30-second remained-connected validation.
- DonutSMP-specific NetworkStackLatency handling uses the scaled microsecond response and no longer triggers the previous `Invalid latency id` disconnect.
- Server menu/RTP/region-selector items are filtered out of placeable item selection.

## Current Real-Server Status

Latest DonutSMP validation log: `tmp/donutsmp_gameplay_validation_latest23.log`.

- `login=true`
- `spawn=true`
- `player_spawn=true`
- `remained_connected=true`
- `keepalive=true`
- `chat=true`
- `inventory_transactions=true`
- `movement=true`
- `block_breaking=false`
- `block_placing=false`
- `gameplay_actions=false`
- `disconnect_reason=null`

The current code now:

- rejects DonutSMP menu/RTP/region selector items as non-placeable,
- treats modern Bedrock block item IDs as valid placeable candidates when they are normal item stacks,
- caches multiple normal `AddItemEntity` candidates,
- walks to each normal candidate before `/rtp`,
- sends both `MovePlayer` and `PlayerAuthInput` during pickup movement,
- retries all normal observed item entities before marking pickup terminal.

Latest live evidence:

- The bot walked onto all observed normal item entities in the loaded DonutSMP area.
- No `TakeItemEntity` packet was received for the player.
- No `InventorySlot` or `InventoryContent` update added a normal placeable item.
- Only server menu/RTP inventory items were present in player inventory.

Current blocker: DonutSMP is not providing a collectible normal placeable item or a safe confirmed block target in the observed validation area. Block placing cannot be validated without a real held placeable item unless the test runs on a local/owned/permissioned server or an account/server state that already has a normal placeable block.

## In Progress

- Keep the DonutSMP observations as compatibility evidence, but shift block break/place acceptance testing to a local or explicitly permissioned server with known normal blocks/items.
- Continue moving validation-only behavior into reusable bot/session controllers once the local owned-server gameplay test is stable.

## Known Gaps

- Block placing still requires a real collected or selected normal placeable item and a server-confirmed `UpdateBlock` at the placement result.
- Form handling is still not validated as successful on the current target server.
- Public reusable `BotSession` commands exist only partially; much of the mature behavior is still inside validation/session internals.
- Fake-server integration coverage is still missing.
- Long-running bot supervision and event streaming are not yet production complete.

## Validation Policy

Real-server validation should only be run against servers that are owned, explicitly allowed, or otherwise safe to test. Gameplay automation remains conservative and should not include anticheat bypass or evasion behavior.
