# TorchFlower Next Steps

Last updated: 2026-06-05

## Immediate Blocker

DonutSMP validation now reaches login, spawn, movement, chat, inventory observation, and sustained connection, but gameplay validation still fails because the bot cannot obtain a normal held placeable item in the observed public-server area.

Latest evidence from `tmp/donutsmp_gameplay_validation_latest23.log`:

- Multiple normal `AddItemEntity` candidates were observed.
- The bot moved to those candidates with both `MovePlayer` and `PlayerAuthInput`.
- The server never sent `TakeItemEntity` for the player.
- The server never sent an inventory update adding a normal placeable block.
- The only player inventory items were server menu/RTP/region selector items, which must remain rejected.

## Required Next Validation Target

Set up a local, owned, or explicitly permissioned Bedrock/Geyser test server where:

- the account starts with a normal placeable block, or
- a normal block can be broken and collected, and
- block placing is allowed in the test area.

Use DonutSMP as a compatibility target for login, spawn, keepalive, movement, and inventory observation only until a legitimate gameplay test area is available.

## Engineering Tasks

- Add a stored server/test profile for local owned gameplay validation.
- Add a fixture or fake-server test that emits a collectible item flow:
  - `AddItemEntity`
  - pickup movement
  - `TakeItemEntity`
  - `InventorySlot`
  - `MobEquipment`
  - place transaction
  - `UpdateBlock` placement confirmation
- Keep menu/RTP/server UI item filtering in place.
- Preserve packet 115, RakNet, login, encryption, compression, and fragmentation behavior unchanged.
