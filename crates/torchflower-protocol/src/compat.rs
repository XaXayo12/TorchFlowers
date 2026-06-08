//! Compatibility shims so that `torchflower-engine/session.rs` (which was
//! originally written against `bedrock-rs` / `bedrock-protocol`) compiles
//! against the native `torchflower-protocol` types without changing 7 000+
//! lines of session logic.
//!
//! Every item here is either a type alias, a newtype wrapper, or a constant
//! that maps the old API to the new one.

use crate::*;

// ---------------------------------------------------------------------------
// Actor ID
// ---------------------------------------------------------------------------

/// Wrapper around `u64` that was `ActorRuntimeID` in bedrock-rs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActorRuntimeID(pub u64);

impl From<u64> for ActorRuntimeID {
    fn from(v: u64) -> Self {
        Self(v)
    }
}
impl From<ActorRuntimeID> for u64 {
    fn from(v: ActorRuntimeID) -> Self {
        v.0
    }
}

// ---------------------------------------------------------------------------
// Enum shims
// ---------------------------------------------------------------------------

/// Resource-pack response byte values (old `ResourcePackResponse` enum).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum ResourcePackResponse {
    None = 0,
    Refused = 1,
    SendPacks = 2,
    AllPacksDownloaded = 3,
    Completed = 4,
}
impl ResourcePackResponse {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Play-status byte values (old `PlayStatus` enum).
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayStatus {
    LoginSuccess = 0,
    LoginFailedClient = 1,
    LoginFailedServer = 2,
    PlayerSpawn = 3,
    LoginFailedInvalidTenant = 4,
    LoginFailedVanillaEdu = 5,
    LoginFailedEduVanilla = 6,
    LoginFailedServerFull = 7,
}

/// Player action types (old `PlayerActionType` enum).
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerActionType {
    StartBreak = 0,
    AbortBreak = 1,
    StopBreak = 2,
    GetUpdatedBlock = 3,
    DropItem = 4,
    StartSleeping = 5,
    StopSleeping = 6,
    Respawn = 7,
    Jump = 8,
    StartSprint = 9,
    StopSprint = 10,
    StartSneak = 11,
    StopSneak = 12,
    CreativePlayerDestroyBlock = 13,
    DimensionChangeAck = 14,
    StartGlide = 15,
    StopGlide = 16,
    BuildDenied = 17,
    ContinueDestroyBlock = 18,
    PredictiveBreak = 38,
}

/// Player position mode byte values (old `PlayerPositionMode` enum).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerPositionMode {
    Normal = 0,
    Reset = 1,
    Teleport = 2,
    OnlyHeadRot = 3,
}

/// Container-ID byte values (old `ContainerID` enum).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerID {
    Inventory = 0,
    Offhand = 119,
    Armor = 120,
    CreativeOutput = 121,
    Hotbar = 27,
    PlayerInventory = 28,
    Ui = 124,
}

/// Complex inventory transaction types (old `ComplexInventoryTransactionType`).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplexInventoryTransactionType {
    NormalTransaction = 0,
    InventoryMismatch = 1,
    ItemUseTransaction = 2,
    ItemUseOnEntityTransaction = 3,
    ItemReleaseTransaction = 4,
}

// ---------------------------------------------------------------------------
// PlayerAuthInputFlags – bitfield constants
// ---------------------------------------------------------------------------

/// Bit-field constants for PlayerAuthInput's `input_data` field.
/// These are the raw u128 bit-mask values used throughout session.rs.
#[allow(non_upper_case_globals)]
pub struct PlayerAuthInputFlags;

impl PlayerAuthInputFlags {
    pub const Ascend: u128 = 1 << 0;
    pub const Descend: u128 = 1 << 1;
    pub const NorthJump: u128 = 1 << 2;
    pub const JumpDown: u128 = 1 << 3;
    pub const SprintDown: u128 = 1 << 4;
    pub const ChangeHeight: u128 = 1 << 5;
    pub const Jumping: u128 = 1 << 6;
    pub const AutoJumpingInWater: u128 = 1 << 7;
    pub const Sneaking: u128 = 1 << 8;
    pub const SneakDown: u128 = 1 << 9;
    pub const Up: u128 = 1 << 10;
    pub const Down: u128 = 1 << 11;
    pub const Left: u128 = 1 << 12;
    pub const Right: u128 = 1 << 13;
    pub const UpLeft: u128 = 1 << 14;
    pub const UpRight: u128 = 1 << 15;
    pub const WantUp: u128 = 1 << 16;
    pub const WantDown: u128 = 1 << 17;
    pub const WantDownSlow: u128 = 1 << 18;
    pub const WantUpSlow: u128 = 1 << 19;
    pub const Sprinting: u128 = 1 << 20;
    pub const AscendScaffolding: u128 = 1 << 21;
    pub const DescendScaffolding: u128 = 1 << 22;
    pub const SneakToggleDown: u128 = 1 << 23;
    pub const PersistSneak: u128 = 1 << 24;
    pub const StartSprinting: u128 = 1 << 25;
    pub const StopSprinting: u128 = 1 << 26;
    pub const StartSneaking: u128 = 1 << 27;
    pub const StopSneaking: u128 = 1 << 28;
    pub const StartSwimmingDown: u128 = 1 << 29;
    pub const StopSwimmingDown: u128 = 1 << 30;
    pub const StartGliding: u128 = 1 << 31;
    pub const StopGliding: u128 = 1 << 32;
    pub const PerformItemInteraction: u128 = 1 << 33;
    pub const PerformBlockActions: u128 = 1 << 34;
    pub const PerformItemStackRequest: u128 = 1 << 35;
    pub const HandledTeleport: u128 = 1 << 36;
    pub const Emoting: u128 = 1 << 37;
    pub const MissedSwing: u128 = 1 << 38;
    pub const StartCrawling: u128 = 1 << 39;
    pub const StopCrawling: u128 = 1 << 40;
    pub const StartFlying: u128 = 1 << 41;
    pub const StopFlying: u128 = 1 << 42;
    pub const AckEntityData: u128 = 1 << 43;
    pub const IsInClientPredictedVehicle: u128 = 1 << 44;
    pub const PaddlingLeft: u128 = 1 << 45;
    pub const PaddlingRight: u128 = 1 << 46;
    pub const BlockBreakingDelayEnabled: u128 = 1 << 47;
    pub const HorizontalCollision: u128 = 1 << 48;
    pub const VerticalCollision: u128 = 1 << 49;
    pub const DownLeft: u128 = 1 << 50;
    pub const DownRight: u128 = 1 << 51;
    pub const ReceivedServerData: u128 = 1 << 52;
}

// ---------------------------------------------------------------------------
// TextPacketType shim
// ---------------------------------------------------------------------------

/// Text packet type byte values (old `TextPacketType` enum).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPacketType {
    Raw = 0,
    Chat = 1,
    Translation = 2,
    Popup = 3,
    JukeboxPopup = 4,
    Tip = 5,
    System = 6,
    Whisper = 7,
    Announcement = 8,
    JsonWhisper = 9,
    Json = 10,
    JsonAnnouncement = 11,
}

// ---------------------------------------------------------------------------
// NetworkItemStackDescriptor – minimal shim
// ---------------------------------------------------------------------------

/// Minimal stand-in for the old `NetworkItemStackDescriptor`.
/// session.rs stores it but only passes it as raw bytes; the actual
/// serialisation uses a pre-encoded `item_bytes` field.
#[derive(Debug, Clone, Default)]
pub struct NetworkItemStackDescriptor {
    pub network_id: i32,
    pub count: u16,
    pub metadata: u32,
    pub block_runtime_id: i32,
    pub extra_bytes: Vec<u8>,
}

impl NetworkItemStackDescriptor {
    pub fn serialize(&self, out: &mut Vec<u8>) -> Result<(), &'static str> {
        let mut value = self.network_id as u32;
        loop {
            if value & !0x7f == 0 {
                out.push(value as u8);
                break;
            }
            out.push(((value & 0x7f) | 0x80) as u8);
            value >>= 7;
        }
        if self.network_id > 0 {
            out.extend_from_slice(&self.count.to_le_bytes());
            let mut val_u = self.metadata;
            loop {
                if val_u & !0x7f == 0 {
                    out.push(val_u as u8);
                    break;
                }
                out.push(((val_u & 0x7f) | 0x80) as u8);
                val_u >>= 7;
            }
            let zz = ((self.block_runtime_id << 1) ^ (self.block_runtime_id >> 31)) as u32;
            let mut val_zz = zz;
            loop {
                if val_zz & !0x7f == 0 {
                    out.push(val_zz as u8);
                    break;
                }
                out.push(((val_zz & 0x7f) | 0x80) as u8);
                val_zz >>= 7;
            }
            let mut val_len = self.extra_bytes.len() as u32;
            loop {
                if val_len & !0x7f == 0 {
                    out.push(val_len as u8);
                    break;
                }
                out.push(((val_len & 0x7f) | 0x80) as u8);
                val_len >>= 7;
            }
            out.extend_from_slice(&self.extra_bytes);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// InventoryTransaction – minimal shim
// ---------------------------------------------------------------------------

/// Minimal stand-in for the old generic `InventoryTransaction<T>`.
/// session.rs only uses it as `{ action: vec![] }` so we just need
/// the `action` field.
#[derive(Debug, Clone, Default)]
pub struct InventoryTransaction {
    pub action: Vec<()>,
}

// ---------------------------------------------------------------------------
// Packet variant aliases
// ---------------------------------------------------------------------------
// session.rs uses `BedrockProto::PlayStatusPacket(p)` etc.
// Provide `impl Packet { fn PlayStatusPacket ... }` would be wrong because
// we need variant names, not methods.
//
// Instead, provide `type` aliases for each packet struct that match the
// old names so struct constructions compile.  The _variant_ names
// (BedrockProto::PlayStatusPacket etc.) must be fixed in session.rs via
// the bulk rename in the import block below.

// These are all re-exported under the old names so the code compiles:
pub use crate::AnimatePacket;
pub use crate::ClientCacheStatusPacket;
pub use crate::ClientToServerHandshakePacket;
pub use crate::CommandOutputPacket;
pub use crate::CorrectPlayerMovePredictionPacket;
pub use crate::DisconnectPacket;
pub use crate::InventoryContentPacket;
pub use crate::InventorySlotPacket;
pub use crate::InventoryTransactionPacket;
pub use crate::ItemStackResponsePacket;
pub use crate::LevelEventPacket;
pub use crate::MobEquipmentPacket;
pub use crate::ModalFormRequestPacket;
pub use crate::ModalFormResponsePacket;
pub use crate::MovePlayerPacket;
pub use crate::NetworkStackLatencyPacket;
pub use crate::PlayStatusPacket;
pub use crate::RequestChunkRadiusPacket;
pub use crate::ResourcePackClientResponsePacket;
pub use crate::ResourcePackStackPacket;
pub use crate::ResourcePacksInfoPacket;
pub use crate::RespawnPacket;
pub use crate::ServerToClientHandshakePacket;
pub use crate::SetLocalPlayerAsInitializedPacket;
pub use crate::StartGamePacket;
pub use crate::TextPacket;
pub use crate::UpdateBlockPacket;
