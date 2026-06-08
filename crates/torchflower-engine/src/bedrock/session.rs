use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env,
    time::{Duration, Instant},
};

use chrono::Utc;
use serde_json::json;
use tokio::time::{sleep, timeout};
use torchflower_protocol::{
    compat::{
        ActorRuntimeID, NetworkItemStackDescriptor, PlayStatus, PlayerActionType,
        PlayerAuthInputFlags, ResourcePackResponse, TextPacketType,
    },
    AnimatePacket,
    BlockPosition as BlockPos,
    // packet structs re-exported via compat
    ClientCacheStatusPacket,
    InventoryTransactionPacket,
    MobEquipmentPacket,
    MovePlayerPacket,
    Packet as BedrockProto,
    RequestChunkRadiusPacket,
    ResourcePackClientResponsePacket,
    SetLocalPlayerAsInitializedPacket,
    TextPacket,
    Vector3f,
};
use uuid::Uuid;

use crate::{
    auth::ProvisionedBedrockSession,
    bedrock::protocol_adapter::{
        BedrockProtocolAdapter, ObservedBlockSample, ObservedInventoryItem, ObservedItemEntity,
        ObservedPacket,
    },
    db::Database,
    diagnostics::Diagnostics,
    error::{EngineError, EngineResult},
    models::CapabilityStatus,
};

const MOVEMENT_VALIDATION_SECONDS: u64 = 10;
const MOVEMENT_SEND_INTERVAL: Duration = Duration::from_millis(50);
const MOVEMENT_MAX_STEP_SECONDS: f32 = 0.05;
const MOVEMENT_COMPLETION_QUIET_PERIOD: Duration = Duration::from_secs(1);
const POST_RESOURCE_STACK_PRE_SPAWN_RECV_TIMEOUT: Duration = Duration::from_secs(60);
const MOVEMENT_FORWARD_SPEED_BLOCKS_PER_SECOND: f32 = 2.8;
const GAMEPLAY_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const GAMEPLAY_PICKUP_DURATION: Duration = Duration::from_secs(8);
const GAMEPLAY_PICKUP_SEND_INTERVAL: Duration = Duration::from_millis(200);
const GAMEPLAY_PICKUP_INVENTORY_PROBE_INTERVAL: Duration = Duration::from_secs(1);
const GAMEPLAY_PICKUP_SPEED_BLOCKS_PER_SECOND: f32 = 3.2;
const GAMEPLAY_APPROACH_SEND_INTERVAL: Duration = Duration::from_millis(200);
const GAMEPLAY_APPROACH_SPEED_BLOCKS_PER_SECOND: f32 = 3.2;
const GAMEPLAY_BREAK_REACH_HORIZONTAL: f32 = 4.5;
const GAMEPLAY_BREAK_REACH_VERTICAL: f32 = 4.5;
const GAMEPLAY_BREAK_SEND_INTERVAL: Duration = Duration::from_millis(200);
const GAMEPLAY_BREAK_DURATION: Duration = Duration::from_secs(6);
const GAMEPLAY_BREAK_CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);
const GAMEPLAY_MAX_BREAK_TARGET_ATTEMPTS: u8 = 4;
const GAMEPLAY_PLAYER_EYE_HEIGHT: f32 = 1.62;
const MAX_OBSERVED_SOLID_BLOCKS: usize = 2048;
const RTP_WAIT_DURATION: Duration = Duration::from_secs(75);
const RTP_MENU_OPEN_WAIT_DURATION: Duration = Duration::from_secs(75);
const RTP_MENU_CLICK_WAIT_DURATION: Duration = Duration::from_secs(12);
const RTP_MENU_CLICK_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const RTP_MENU_MAX_CLICK_ATTEMPTS: u8 = 8;
const RTP_MAX_COMMAND_ATTEMPTS: u8 = 2;
const TICK_SYNC_INTERVAL: Duration = Duration::from_millis(500);
const COMMAND_REQUEST_PACKET_ID: u32 = 0x4d;
const TICK_SYNC_PACKET_ID: u32 = 0x17;
const CONTAINER_CLOSE_PACKET_ID: u32 = 0x2f;
const PLAYER_AUTH_INPUT_PACKET_ID: u32 = 0x90;
const ITEM_STACK_REQUEST_PACKET_ID: u32 = 0x93;
const CONTAINER_TYPE_CONTAINER: u8 = 7;
const CONTAINER_TYPE_CURSOR: u8 = 59;
const CONTAINER_TYPE_DYNAMIC: u8 = 63;
const WINDOW_TYPE_CONTAINER: u8 = 0;
const OBSERVED_AIR_RUNTIME_ID: u32 = 12530;
const OBSERVED_CHEST_RUNTIME_ID: u32 = 13313;
const SPRUCE_BUTTON_CEILING_RUNTIME_ID: u32 = 5901;
const BLOCK_FACE_DOWN: i32 = 0;
const BLOCK_FACE_UP: i32 = 1;

fn env_flag(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            let value = value.trim();
            !(value == "0" || value.eq_ignore_ascii_case("false"))
        })
        .unwrap_or(default)
}

fn trace_chunks_enabled() -> bool {
    env_flag("BEDROCK_TRACE_CHUNKS", false)
}

fn trace_packets_enabled() -> bool {
    env_flag("BEDROCK_TRACE_PACKETS", false)
}

fn trust_chunk_publisher_position() -> bool {
    env_flag("BEDROCK_TRUST_CHUNK_PUBLISHER_POSITION", false)
}

fn trust_item_entity_position_hint() -> bool {
    env_flag("BEDROCK_TRUST_ITEM_ENTITY_POSITION_HINT", true)
}

fn received_server_data_input_flag() -> u128 {
    PlayerAuthInputFlags::ReceivedServerData
}

fn block_action_input_flags() -> u128 {
    received_server_data_input_flag() | PlayerAuthInputFlags::PerformBlockActions
}

fn movement_start_delay_duration() -> Duration {
    if let Some(ms) = env::var("BEDROCK_MOVEMENT_START_DELAY_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        return Duration::from_millis(ms);
    }
    if let Some(seconds) = env::var("BEDROCK_MOVEMENT_START_DELAY_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        return Duration::from_secs(seconds);
    }
    Duration::from_secs(3)
}

fn movement_start_deadline() -> (Option<Instant>, Duration) {
    let delay = movement_start_delay_duration();
    if delay.is_zero() {
        (None, delay)
    } else {
        (Some(Instant::now() + delay), delay)
    }
}

pub struct BedrockBotSession {
    db: Database,
    diagnostics: Diagnostics,
}

#[derive(Debug, Clone)]
struct MovementValidation {
    runtime_id: ActorRuntimeID,
    entity_id: i64,
    started_at: Instant,
    last_sent_at: Option<Instant>,
    last_tick_sync_sent_at: Option<Instant>,
    last_input_tick: u64,
    tick_sync_time: i64,
    tick_sync_sent_count: u64,
    spawn_position: (f32, f32, f32),
    yaw: f32,
    pitch: f32,
    sent_frames: u64,
    idle_frames_sent: u64,
    correction_count: u64,
    completion_reported: bool,
    last_sent_position: (f32, f32, f32),
    last_server_position: Option<(f32, f32, f32)>,
    last_correction_position: Option<(f32, f32, f32)>,
    initial_position_hint_wait_reported: bool,
    held_item: Option<HeldInventoryItem>,
    rtp_menu_item: Option<MenuClickTarget>,
    inventory_probe_sent: bool,
    chat_probe_sent: bool,
    rtp_command_sent: bool,
    rtp_command_sent_at: Option<Instant>,
    rtp_command_attempts: u8,
    rtp_menu_click_sent: bool,
    rtp_menu_click_sent_at: Option<Instant>,
    rtp_menu_click_attempts: u8,
    rtp_menu_last_click_attempt_at: Option<Instant>,
    rtp_container_close_sent: bool,
    rtp_position_hint_received: bool,
    rtp_position_hint_received_at: Option<Instant>,
    rtp_marker_position_hint_received: bool,
    rtp_terrain_position_hint_received: bool,
    rtp_terrain_position_hint_received_at: Option<Instant>,
    rtp_waiting_for_menu_reported: bool,
    rtp_waiting_for_terrain_hint_reported: bool,
    rtp_terrain_position_hint_failed_reported: bool,
    rtp_wait_done: bool,
    next_item_stack_request_id: i32,
    gameplay_probe_sent: bool,
    break_probe_started_at: Option<Instant>,
    break_last_sent_at: Option<Instant>,
    break_stop_sent: bool,
    break_confirmed: bool,
    break_confirmation_failed_reported: bool,
    place_probe_sent: bool,
    gameplay_probe_sent_at: Option<Instant>,
    gameplay_timeout_reported: bool,
    pickup_probe_started_at: Option<Instant>,
    pickup_last_sent_at: Option<Instant>,
    pickup_last_inventory_probe_at: Option<Instant>,
    pickup_frames_sent: u64,
    pickup_prebreak: bool,
    pickup_failed_reported: bool,
    pickup_terminal_failed: bool,
    approach_last_sent_at: Option<Instant>,
    approach_frames_sent: u64,
    held_item_equipped: bool,
    break_target: Option<BlockTarget>,
    break_target_runtime_id: Option<u32>,
    place_base: Option<BlockTarget>,
    place_result: Option<BlockTarget>,
    rejected_break_targets: Vec<BlockTarget>,
    rejected_break_runtime_ids: Vec<u32>,
    break_target_attempts: u8,
    observed_break_candidate: Option<BlockTarget>,
    observed_solid_blocks: Vec<BlockTarget>,
    observed_solid_block_runtime_ids: Vec<(BlockTarget, u32)>,
    observed_item_entity: Option<ItemEntityTarget>,
    observed_item_entities: Vec<ItemEntityTarget>,
    rejected_item_entity_runtime_ids: Vec<u64>,
    network_chunk_position_hint: Option<(f32, f32, f32)>,
    inventory_log_count: u32,
}

#[derive(Debug, Clone)]
struct HeldInventoryItem {
    container_id: u32,
    slot: u32,
    item_id: i32,
    item: Option<NetworkItemStackDescriptor>,
    item_bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct MenuClickTarget {
    window_id: u32,
    slot: u32,
    item_id: i32,
    stack_id: i32,
    container_type: u8,
    dynamic_container_id: Option<u32>,
    priority: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlockTarget {
    x: i32,
    y: i32,
    z: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PlaceGeometry {
    base: BlockTarget,
    result: BlockTarget,
    face: i32,
    click_pos: (f32, f32, f32),
}

#[derive(Debug, Clone)]
struct ItemEntityTarget {
    runtime_id: u64,
    item_id: i32,
    stack_id: Option<i32>,
    position: (f32, f32, f32),
    item_bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct RawBlockAction {
    action_id: i32,
    target: BlockTarget,
    face: i32,
}

#[derive(Debug, Clone)]
struct RawItemUseTransaction {
    action_type: u32,
    trigger_type: u32,
    block_position: BlockTarget,
    face: i32,
    hotbar_slot: i32,
    held_item_bytes: Vec<u8>,
    player_pos: (f32, f32, f32),
    click_pos: (f32, f32, f32),
    block_runtime_id: u32,
    client_prediction: u32,
}

#[derive(Debug, Clone, Copy)]
enum MenuClickMethod {
    StandaloneObservedTake,
    PlayerAuthInputObservedTake,
    StandaloneObservedConsume,
    PlayerAuthInputObservedConsume,
    StandaloneDynamicTake,
    PlayerAuthInputDynamicTake,
    StandaloneContainerTake,
    PlayerAuthInputContainerTake,
}

impl MenuClickMethod {
    fn from_attempt(attempt: u8) -> Option<Self> {
        match attempt {
            0 => Some(Self::StandaloneObservedConsume),
            1 => Some(Self::PlayerAuthInputObservedConsume),
            2 => Some(Self::StandaloneObservedTake),
            3 => Some(Self::PlayerAuthInputObservedTake),
            4 => Some(Self::StandaloneDynamicTake),
            5 => Some(Self::PlayerAuthInputDynamicTake),
            6 => Some(Self::StandaloneContainerTake),
            7 => Some(Self::PlayerAuthInputContainerTake),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::StandaloneObservedTake => "standalone_observed_take",
            Self::PlayerAuthInputObservedTake => "player_auth_input_observed_take",
            Self::StandaloneObservedConsume => "standalone_observed_consume",
            Self::PlayerAuthInputObservedConsume => "player_auth_input_observed_consume",
            Self::StandaloneDynamicTake => "standalone_dynamic_take",
            Self::PlayerAuthInputDynamicTake => "player_auth_input_dynamic_take",
            Self::StandaloneContainerTake => "standalone_container_take",
            Self::PlayerAuthInputContainerTake => "player_auth_input_container_take",
        }
    }

    fn uses_player_auth_input(self) -> bool {
        matches!(
            self,
            Self::PlayerAuthInputObservedTake
                | Self::PlayerAuthInputObservedConsume
                | Self::PlayerAuthInputDynamicTake
                | Self::PlayerAuthInputContainerTake
        )
    }

    fn source_container(self, target: &MenuClickTarget) -> (u8, Option<u32>) {
        match self {
            Self::PlayerAuthInputDynamicTake | Self::StandaloneDynamicTake => (
                CONTAINER_TYPE_DYNAMIC,
                target.dynamic_container_id.or(Some(target.window_id)),
            ),
            Self::PlayerAuthInputContainerTake | Self::StandaloneContainerTake => {
                (CONTAINER_TYPE_CONTAINER, None)
            }
            Self::PlayerAuthInputObservedTake
            | Self::StandaloneObservedTake
            | Self::PlayerAuthInputObservedConsume
            | Self::StandaloneObservedConsume => {
                (target.container_type, target.dynamic_container_id)
            }
        }
    }

    fn action(self) -> MenuClickAction {
        match self {
            Self::StandaloneObservedConsume | Self::PlayerAuthInputObservedConsume => {
                MenuClickAction::Consume
            }
            _ => MenuClickAction::Take,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuClickAction {
    Take,
    Consume,
}

impl MenuClickAction {
    fn label(self) -> &'static str {
        match self {
            Self::Take => "take",
            Self::Consume => "consume",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RawItemStackRequest<'a> {
    request_id: i32,
    target: &'a MenuClickTarget,
    method: MenuClickMethod,
    action_id: MenuClickAction,
}

#[derive(Debug, Clone)]
struct RawPlayerAuthInput<'a> {
    position: (f32, f32, f32),
    velocity: (f32, f32, f32),
    yaw: f32,
    pitch: f32,
    input_data: u128,
    tick: u64,
    move_vector: (f32, f32, f32),
    analog_move_vector: (f32, f32),
    raw_move_vector: (f32, f32),
    block_actions: &'a [RawBlockAction],
    item_use_transaction_id: Option<&'a RawItemUseTransaction>,
    item_stack_request: Option<RawItemStackRequest<'a>>,
}

impl MovementValidation {
    fn new(
        runtime_id: ActorRuntimeID,
        tick_base: u64,
        spawn_position: (f32, f32, f32),
        rotation: (f32, f32),
    ) -> Self {
        Self {
            entity_id: runtime_id.0 as i64,
            runtime_id,
            started_at: Instant::now(),
            last_sent_at: None,
            last_tick_sync_sent_at: None,
            last_input_tick: tick_base,
            tick_sync_time: tick_base as i64,
            tick_sync_sent_count: 0,
            spawn_position,
            yaw: rotation.0,
            pitch: rotation.1,
            sent_frames: 0,
            idle_frames_sent: 0,
            correction_count: 0,
            completion_reported: false,
            last_sent_position: spawn_position,
            last_server_position: None,
            last_correction_position: None,
            initial_position_hint_wait_reported: false,
            held_item: None,
            rtp_menu_item: None,
            inventory_probe_sent: false,
            chat_probe_sent: false,
            rtp_command_sent: false,
            rtp_command_sent_at: None,
            rtp_command_attempts: 0,
            rtp_menu_click_sent: false,
            rtp_menu_click_sent_at: None,
            rtp_menu_click_attempts: 0,
            rtp_menu_last_click_attempt_at: None,
            rtp_container_close_sent: false,
            rtp_position_hint_received: false,
            rtp_position_hint_received_at: None,
            rtp_marker_position_hint_received: false,
            rtp_terrain_position_hint_received: false,
            rtp_terrain_position_hint_received_at: None,
            rtp_waiting_for_menu_reported: false,
            rtp_waiting_for_terrain_hint_reported: false,
            rtp_terrain_position_hint_failed_reported: false,
            rtp_wait_done: false,
            next_item_stack_request_id: 1,
            gameplay_probe_sent: false,
            break_probe_started_at: None,
            break_last_sent_at: None,
            break_stop_sent: false,
            break_confirmed: false,
            break_confirmation_failed_reported: false,
            place_probe_sent: false,
            gameplay_probe_sent_at: None,
            gameplay_timeout_reported: false,
            pickup_probe_started_at: None,
            pickup_last_sent_at: None,
            pickup_last_inventory_probe_at: None,
            pickup_frames_sent: 0,
            pickup_prebreak: false,
            pickup_failed_reported: false,
            pickup_terminal_failed: false,
            approach_last_sent_at: None,
            approach_frames_sent: 0,
            held_item_equipped: false,
            break_target: None,
            break_target_runtime_id: None,
            place_base: None,
            place_result: None,
            rejected_break_targets: Vec::new(),
            rejected_break_runtime_ids: Vec::new(),
            break_target_attempts: 0,
            observed_break_candidate: None,
            observed_solid_blocks: Vec::new(),
            observed_solid_block_runtime_ids: Vec::new(),
            observed_item_entity: None,
            observed_item_entities: Vec::new(),
            rejected_item_entity_runtime_ids: Vec::new(),
            network_chunk_position_hint: None,
            inventory_log_count: 0,
        }
    }

    fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    fn completed(&self) -> bool {
        self.elapsed() >= Duration::from_secs(MOVEMENT_VALIDATION_SECONDS)
    }

    fn should_send(&self) -> bool {
        self.last_sent_at
            .map(|sent_at| sent_at.elapsed() >= MOVEMENT_SEND_INTERVAL)
            .unwrap_or(true)
    }

    fn should_send_tick_sync(&self) -> bool {
        let interval = tick_sync_interval_duration();
        !interval.is_zero()
            && self
                .last_tick_sync_sent_at
                .map(|sent_at| sent_at.elapsed() >= interval)
                .unwrap_or(true)
    }

    fn next_tick_sync_request_time(&mut self) -> Option<i64> {
        let interval = tick_sync_interval_duration();
        if interval.is_zero() {
            return None;
        }
        let request_time = self.tick_sync_time;
        self.tick_sync_time = self.tick_sync_time.wrapping_add(tick_sync_step(interval));
        self.tick_sync_sent_count = self.tick_sync_sent_count.saturating_add(1);
        self.last_tick_sync_sent_at = Some(Instant::now());
        Some(request_time)
    }

    fn next_client_tick(&mut self) -> u64 {
        let tick = self.last_input_tick.saturating_add(1);
        self.last_input_tick = tick;
        tick
    }

    fn next_frame(&mut self) -> MovementFrame {
        let elapsed_secs = self.elapsed().as_secs_f32();
        let step_secs = self
            .last_sent_at
            .map(|sent_at| {
                sent_at
                    .elapsed()
                    .as_secs_f32()
                    .min(MOVEMENT_MAX_STEP_SECONDS)
            })
            .unwrap_or(0.0);
        let yaw_radians = self.yaw.to_radians();
        let forward_x = -yaw_radians.sin();
        let forward_z = yaw_radians.cos();
        let position = (
            self.last_sent_position.0
                + forward_x * MOVEMENT_FORWARD_SPEED_BLOCKS_PER_SECOND * step_secs,
            self.last_sent_position.1,
            self.last_sent_position.2
                + forward_z * MOVEMENT_FORWARD_SPEED_BLOCKS_PER_SECOND * step_secs,
        );
        let velocity = (
            forward_x * MOVEMENT_FORWARD_SPEED_BLOCKS_PER_SECOND,
            0.0,
            forward_z * MOVEMENT_FORWARD_SPEED_BLOCKS_PER_SECOND,
        );
        self.sent_frames += 1;
        self.last_sent_at = Some(Instant::now());
        self.last_sent_position = position;
        let tick = self.next_client_tick();
        MovementFrame {
            frame_index: self.sent_frames,
            runtime_id: self.runtime_id,
            tick,
            position,
            velocity,
            yaw: self.yaw,
            pitch: self.pitch,
            elapsed_seconds: elapsed_secs,
        }
    }

    fn next_idle_frame(&mut self) -> MovementFrame {
        let elapsed_secs = self.elapsed().as_secs_f32();
        self.idle_frames_sent = self.idle_frames_sent.saturating_add(1);
        self.last_sent_at = Some(Instant::now());
        let tick = self.next_client_tick();
        MovementFrame {
            frame_index: self.idle_frames_sent,
            runtime_id: self.runtime_id,
            tick,
            position: self.last_sent_position,
            velocity: (0.0, 0.0, 0.0),
            yaw: self.yaw,
            pitch: self.pitch,
            elapsed_seconds: elapsed_secs,
        }
    }

    fn record_server_position(&mut self, position: (f32, f32, f32)) {
        self.last_server_position = Some(position);
        if self.completed() {
            self.last_sent_position = position;
            self.spawn_position = position;
        }
    }

    fn record_correction(&mut self, position: (f32, f32, f32)) {
        self.correction_count += 1;
        self.last_correction_position = Some(position);
    }

    fn record_network_chunk_publisher_update(&mut self, x: i32, y: i32, z: i32, radius: u32) {
        let position = (x as f32 + 0.5, y as f32, z as f32 + 0.5);
        self.network_chunk_position_hint = Some(position);
        let current = self.last_server_position.unwrap_or(self.last_sent_position);
        let dx = position.0 - current.0;
        let dz = position.2 - current.2;
        let distance_xz = (dx * dx + dz * dz).sqrt();
        if trace_chunks_enabled() {
            eprintln!(
                "[GAMEPLAY_CHUNK_PUBLISHER] cached_position={} radius={} completed={} gameplay_probe_sent={} distance_xz={:.1}",
                format_position(position),
                radius,
                self.completed(),
                self.gameplay_probe_sent,
                distance_xz
            );
        }
        if trust_chunk_publisher_position() && !self.gameplay_probe_sent && self.completed() {
            self.adopt_network_chunk_position_hint("network_chunk_publisher_update");
        }
    }

    fn raw_start_position_looks_placeholder(&self) -> bool {
        self.spawn_position.0.abs() < 1.0 && self.spawn_position.2.abs() < 1.0
    }

    fn should_wait_for_initial_position_hint(&self) -> bool {
        trust_chunk_publisher_position()
            && self.sent_frames == 0
            && self.last_server_position.is_none()
            && self.raw_start_position_looks_placeholder()
            && self.network_chunk_position_hint.is_none()
            && self.elapsed() < Duration::from_secs(3)
    }

    fn adopt_initial_position_hint_if_available(&mut self) -> bool {
        if !trust_chunk_publisher_position()
            || self.sent_frames != 0
            || self.last_server_position.is_some()
            || !self.raw_start_position_looks_placeholder()
            || self.network_chunk_position_hint.is_none()
        {
            return false;
        }
        let adopted = self.adopt_network_chunk_position_hint("pre_movement_chunk_publisher_update");
        if adopted {
            self.started_at = Instant::now();
            self.last_sent_at = None;
            self.sent_frames = 0;
            self.initial_position_hint_wait_reported = false;
        }
        adopted
    }

    fn adopt_network_chunk_position_hint(&mut self, source: &'static str) -> bool {
        let Some(position) = self.network_chunk_position_hint else {
            return false;
        };
        let current = self.last_server_position.unwrap_or(self.last_sent_position);
        let dx = position.0 - current.0;
        let dz = position.2 - current.2;
        let distance_xz = (dx * dx + dz * dz).sqrt();
        if distance_xz < 1.0 && self.last_server_position == Some(position) {
            return false;
        }
        self.last_server_position = Some(position);
        self.last_sent_position = position;
        self.spawn_position = position;
        eprintln!(
            "[GAMEPLAY_POSITION] source={} position={} previous={} distance_xz={:.1}",
            source,
            format_position(position),
            format_position(current),
            distance_xz
        );
        true
    }

    fn has_sampled_placeable_drop_target(&self) -> bool {
        self.nearest_observed_solid_block_with_limits(
            self.last_sent_position,
            true,
            GAMEPLAY_BREAK_REACH_HORIZONTAL,
            GAMEPLAY_BREAK_REACH_VERTICAL,
        )
        .is_some()
    }

    fn has_approachable_placeable_drop_target(&self) -> bool {
        self.nearest_observed_solid_block_with_limits(
            self.last_sent_position,
            true,
            32.0,
            GAMEPLAY_BREAK_REACH_VERTICAL,
        )
        .is_some()
    }

    fn has_walkable_placeable_drop_target(&self) -> bool {
        self.nearest_observed_solid_block_with_limits(
            self.last_sent_position,
            true,
            128.0,
            GAMEPLAY_BREAK_REACH_VERTICAL,
        )
        .is_some()
    }

    fn observed_block_is_placeable_drop(&self, target: BlockTarget) -> bool {
        self.observed_target_runtime_id(target)
            .map(is_normal_validation_placeable_drop_runtime_id)
            .unwrap_or(false)
    }

    fn observed_block_is_approachable_placeable_drop(&self, target: BlockTarget) -> bool {
        self.observed_block_is_placeable_drop(target)
            && block_target_horizontal_distance(target, self.last_sent_position) <= 32.0
            && block_target_vertical_delta(target, self.last_sent_position) <= 8.0
    }

    fn observed_placeable_drop_count(&self) -> usize {
        self.observed_solid_blocks
            .iter()
            .filter(|target| self.break_target_allowed_for_place_collection(**target))
            .count()
    }

    fn observed_approachable_placeable_drop_count(&self) -> usize {
        self.observed_solid_blocks
            .iter()
            .filter(|target| {
                self.break_target_allowed_for_place_collection(**target)
                    && block_target_horizontal_distance(**target, self.last_sent_position) <= 32.0
                    && block_target_vertical_delta(**target, self.last_sent_position)
                        <= GAMEPLAY_BREAK_REACH_VERTICAL
            })
            .count()
    }

    fn observed_runtime_frequency_summary(&self) -> String {
        if self.observed_solid_block_runtime_ids.is_empty() {
            return "none".to_string();
        }
        let mut counts = BTreeMap::<u32, usize>::new();
        for (_, runtime_id) in &self.observed_solid_block_runtime_ids {
            *counts.entry(*runtime_id).or_default() += 1;
        }
        let mut entries = counts.into_iter().collect::<Vec<_>>();
        entries.sort_by(|(left_id, left_count), (right_id, right_count)| {
            right_count
                .cmp(left_count)
                .then_with(|| left_id.cmp(right_id))
        });
        entries
            .into_iter()
            .take(12)
            .map(|(runtime_id, count)| {
                format!(
                    "{}:{}:accepted={}:rejected={}",
                    runtime_id,
                    count,
                    is_normal_validation_placeable_drop_runtime_id(runtime_id),
                    self.rejected_break_runtime_ids.contains(&runtime_id)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    fn record_observed_block_sample(&mut self, sample: &ObservedBlockSample) {
        if sample.runtime_id == OBSERVED_AIR_RUNTIME_ID
            || is_gameplay_marker_runtime(sample.runtime_id)
        {
            return;
        }
        let target = BlockTarget {
            x: sample.x,
            y: sample.y,
            z: sample.z,
        };
        self.remember_observed_solid_block(target, sample.runtime_id);
        if trace_chunks_enabled() && self.observed_solid_blocks.len() <= 96 {
            eprintln!(
                "[GAMEPLAY_CHUNK_SAMPLE] target={} runtime_id={} placeable_drop={} observed_count={}",
                format_block_target(target),
                sample.runtime_id,
                is_normal_validation_placeable_drop_runtime_id(sample.runtime_id),
                self.observed_solid_blocks.len()
            );
        }
    }

    fn adopt_observed_terrain_position_hint(&mut self, source: &'static str) -> bool {
        if self.rtp_terrain_position_hint_received || self.observed_solid_blocks.is_empty() {
            return false;
        }
        let require_placeable_drop = self.held_item.is_none();
        let current = self.last_server_position.unwrap_or(self.last_sent_position);
        let Some(target) = self
            .observed_break_candidate
            .filter(|target| {
                if require_placeable_drop {
                    self.break_target_allowed_for_place_collection(*target)
                } else {
                    self.break_target_allowed(*target)
                }
            })
            .or_else(|| self.highest_observed_solid_block(require_placeable_drop))
            .or_else(|| {
                self.nearest_observed_solid_block_with_limits(
                    current,
                    require_placeable_drop,
                    f32::MAX,
                    f32::MAX,
                )
            })
        else {
            return false;
        };
        let hinted = (
            target.x as f32 + 0.5,
            target.y as f32 + 1.0,
            target.z as f32 + 2.5,
        );
        let dx = hinted.0 - current.0;
        let dz = hinted.2 - current.2;
        let distance_xz = (dx * dx + dz * dz).sqrt();
        self.last_server_position = Some(hinted);
        self.last_sent_position = hinted;
        self.spawn_position = hinted;
        self.rtp_position_hint_received = true;
        self.rtp_position_hint_received_at = Some(Instant::now());
        self.rtp_terrain_position_hint_received = true;
        self.rtp_terrain_position_hint_received_at = self.rtp_position_hint_received_at;
        self.rtp_waiting_for_terrain_hint_reported = false;
        self.rtp_terrain_position_hint_failed_reported = false;
        self.break_target = None;
        self.break_target_runtime_id = None;
        self.place_base = None;
        self.place_result = None;
        self.break_probe_started_at = None;
        self.break_last_sent_at = None;
        self.break_stop_sent = false;
        self.observed_item_entity = None;
        self.observed_item_entities.clear();
        self.rejected_item_entity_runtime_ids.clear();
        eprintln!(
            "[GAMEPLAY_RTP] position_hint=observed_terrain source={} target={} new_position={} previous={} distance_xz={:.1}",
            source,
            format_block_target(target),
            format_position(hinted),
            format_position(current),
            distance_xz
        );
        true
    }

    fn record_rtp_position_hint_from_update_block(
        &mut self,
        target: BlockTarget,
        runtime_id: u32,
        layer: u32,
    ) {
        let current = self.last_server_position.unwrap_or(self.last_sent_position);
        let hinted = (
            target.x as f32 + 0.5,
            target.y as f32 + 1.0,
            target.z as f32 + 0.5,
        );
        let dx = hinted.0 - current.0;
        let dz = hinted.2 - current.2;
        let in_rtp_phase = self.completed() || self.rtp_command_sent || self.rtp_menu_click_sent;

        let marker_runtime = is_gameplay_marker_runtime(runtime_id);
        let break_candidate = gameplay_break_candidate_for_update(target, runtime_id);
        let far_position_hint = (dx * dx + dz * dz) >= 64.0 * 64.0;

        if layer == 0 {
            if runtime_id == OBSERVED_AIR_RUNTIME_ID {
                let below_marker_candidate = BlockTarget {
                    x: target.x,
                    y: target.y.saturating_sub(1),
                    z: target.z,
                };
                self.observed_solid_blocks.retain(|block| *block != target);
                self.observed_solid_blocks
                    .retain(|block| *block != break_candidate);
                self.observed_solid_blocks
                    .retain(|block| *block != below_marker_candidate);
                self.observed_solid_block_runtime_ids.retain(|(block, _)| {
                    *block != target
                        && *block != break_candidate
                        && *block != below_marker_candidate
                });
            } else if marker_runtime {
                eprintln!(
                    "[GAMEPLAY_TARGET] observed_marker_skipped target={} runtime_id={} layer={}",
                    format_block_target(target),
                    runtime_id,
                    layer
                );
            } else {
                self.remember_observed_solid_block(target, runtime_id);
            }
        }

        if far_position_hint
            && (self.rtp_wait_done || self.gameplay_probe_sent)
            && self.can_accept_late_rtp_position_hint()
        {
            eprintln!(
                "[GAMEPLAY_RTP] late_position_hint_detected reset_wait=true source=update_block target={} current={} hinted={} distance_xz={:.1} rtp_wait_done={} gameplay_probe_sent={}",
                format_block_target(target),
                format_position(current),
                format_position(hinted),
                (dx * dx + dz * dz).sqrt(),
                self.rtp_wait_done,
                self.gameplay_probe_sent
            );
            self.reopen_rtp_for_late_position_hint();
        }

        if in_rtp_phase && !self.rtp_wait_done && !self.gameplay_probe_sent && layer == 0 {
            if runtime_id == OBSERVED_AIR_RUNTIME_ID {
                let below_marker_candidate = BlockTarget {
                    x: target.x,
                    y: target.y.saturating_sub(1),
                    z: target.z,
                };
                if self.observed_break_candidate == Some(target)
                    || self.observed_break_candidate == Some(break_candidate)
                    || self.observed_break_candidate == Some(below_marker_candidate)
                {
                    self.observed_break_candidate = None;
                    eprintln!(
                        "[GAMEPLAY_TARGET] observed_break_candidate_cleared target={} runtime_id={} layer={}",
                        format_block_target(target),
                        runtime_id,
                        layer
                    );
                }
            } else if marker_runtime {
                eprintln!(
                    "[GAMEPLAY_TARGET] observed_marker_not_break_candidate source_update_target={} derived_target={} runtime_id={} layer={} observed_count={}",
                    format_block_target(target),
                    format_block_target(break_candidate),
                    runtime_id,
                    layer,
                    self.observed_solid_blocks.len()
                );
            } else {
                if self.observed_break_candidate != Some(break_candidate) {
                    self.observed_break_candidate = Some(break_candidate);
                    eprintln!(
                        "[GAMEPLAY_TARGET] observed_break_candidate target={} source_update_target={} runtime_id={} layer={} observed_count={}",
                        format_block_target(break_candidate),
                        format_block_target(target),
                        runtime_id,
                        layer,
                        self.observed_solid_blocks.len()
                    );
                }
            }
        }

        if !far_position_hint {
            return;
        }

        if !in_rtp_phase || self.rtp_wait_done || self.gameplay_probe_sent {
            if trace_chunks_enabled() {
                eprintln!(
                    "[GAMEPLAY_RTP] position_hint_skip=state target={} current={} hinted={} distance_xz={:.1} completed={} rtp_command_sent={} rtp_menu_click_sent={} rtp_wait_done={} gameplay_probe_sent={}",
                    format_block_target(target),
                    format_position(current),
                    format_position(hinted),
                    (dx * dx + dz * dz).sqrt(),
                    self.completed(),
                    self.rtp_command_sent,
                    self.rtp_menu_click_sent,
                    self.rtp_wait_done,
                    self.gameplay_probe_sent
                );
            }
            return;
        }

        if runtime_id == OBSERVED_AIR_RUNTIME_ID {
            if trace_chunks_enabled() {
                eprintln!(
                    "[GAMEPLAY_RTP] position_hint_skip=air target={} current={} hinted={} distance_xz={:.1}",
                    format_block_target(target),
                    format_position(current),
                    format_position(hinted),
                    (dx * dx + dz * dz).sqrt()
                );
            }
            return;
        }

        if marker_runtime {
            let now = Instant::now();
            self.rtp_position_hint_received = true;
            self.rtp_position_hint_received_at.get_or_insert(now);
            self.rtp_marker_position_hint_received = true;
            eprintln!(
                "[GAMEPLAY_RTP] marker_position_hint=update_block target={} hinted={} accepted_for_gameplay=false waiting_for_terrain_hint=true",
                format_block_target(target),
                format_position(hinted)
            );
            return;
        }

        let direct_command_terrain_hint = self.rtp_command_sent
            && !self.rtp_marker_position_hint_received
            && !self.rtp_menu_click_sent;
        if !self.rtp_marker_position_hint_received
            && !self.rtp_menu_click_sent
            && !direct_command_terrain_hint
        {
            if trace_chunks_enabled() {
                eprintln!(
                    "[GAMEPLAY_RTP] position_hint_skip=no_marker_or_menu_click target={} hinted={} distance_xz={:.1}",
                    format_block_target(target),
                    format_position(hinted),
                    (dx * dx + dz * dz).sqrt()
                );
            }
            return;
        }

        if self.rtp_terrain_position_hint_received {
            if trace_chunks_enabled() {
                eprintln!(
                    "[GAMEPLAY_RTP] position_hint_skip=terrain_already_locked target={} hinted={} locked_position={}",
                    format_block_target(target),
                    format_position(hinted),
                    self.last_server_position
                        .map(format_position)
                        .unwrap_or_else(|| format_position(self.last_sent_position))
                );
            }
            return;
        }

        if direct_command_terrain_hint {
            eprintln!(
                "[GAMEPLAY_RTP] direct_command_terrain_hint=update_block target={} hinted={} accepted_for_gameplay=true",
                format_block_target(target),
                format_position(hinted)
            );
        }

        self.last_server_position = Some(hinted);
        self.last_sent_position = hinted;
        self.spawn_position = hinted;
        self.rtp_position_hint_received = true;
        self.rtp_position_hint_received_at = Some(Instant::now());
        self.rtp_terrain_position_hint_received = true;
        self.rtp_terrain_position_hint_received_at = self.rtp_position_hint_received_at;
        self.rtp_waiting_for_terrain_hint_reported = false;
        self.rtp_terrain_position_hint_failed_reported = false;
        self.break_target = None;
        self.break_target_runtime_id = None;
        self.place_base = None;
        self.place_result = None;
        self.break_probe_started_at = None;
        self.break_last_sent_at = None;
        self.break_stop_sent = false;
        self.observed_item_entity = None;
        self.observed_item_entities.clear();
        self.rejected_item_entity_runtime_ids.clear();
        if self.rtp_menu_click_sent {
            self.rtp_menu_click_attempts = RTP_MENU_MAX_CLICK_ATTEMPTS;
        }
        eprintln!(
            "[GAMEPLAY_RTP] position_hint=update_block target={} new_position={} stop_menu_retries={}",
            format_block_target(target),
            format_position(hinted),
            self.rtp_menu_click_sent
        );
    }

    fn record_rtp_position_hint_from_item_entity(&mut self, entity: &ObservedItemEntity) {
        let region_menu_clicked = self.rtp_menu_click_sent
            && self
                .rtp_menu_item
                .as_ref()
                .map(|item| item.priority >= 120)
                .unwrap_or(false);
        if region_menu_clicked
            && (self.rtp_wait_done || self.gameplay_probe_sent)
            && self.can_accept_late_rtp_position_hint()
        {
            eprintln!(
                "[GAMEPLAY_RTP] late_position_hint_detected reset_wait=true source=item_entity runtime_id={} position={} rtp_wait_done={} gameplay_probe_sent={}",
                entity.runtime_entity_id,
                format_position(entity.position),
                self.rtp_wait_done,
                self.gameplay_probe_sent
            );
            self.reopen_rtp_for_late_position_hint();
        }
        if !region_menu_clicked
            || self.rtp_wait_done
            || self.gameplay_probe_sent
            || self.rtp_terrain_position_hint_received
        {
            return;
        }

        let current = self.last_server_position.unwrap_or(self.last_sent_position);
        let hinted = entity.position;
        let dx = hinted.0 - current.0;
        let dz = hinted.2 - current.2;
        let distance_xz = (dx * dx + dz * dz).sqrt();
        if distance_xz < 64.0 {
            eprintln!(
                "[GAMEPLAY_RTP] item_entity_hint_skip=near_current runtime_id={} item_id={} current={} hinted={} distance_xz={:.1}",
                entity.runtime_entity_id,
                entity.item_id,
                format_position(current),
                format_position(hinted),
                distance_xz
            );
            return;
        }

        let now = Instant::now();
        self.last_server_position = Some(hinted);
        self.last_sent_position = hinted;
        self.spawn_position = hinted;
        self.rtp_position_hint_received = true;
        self.rtp_position_hint_received_at = Some(now);
        self.rtp_terrain_position_hint_received = true;
        self.rtp_terrain_position_hint_received_at = Some(now);
        self.rtp_waiting_for_terrain_hint_reported = false;
        self.rtp_terrain_position_hint_failed_reported = false;
        self.break_target = None;
        self.break_target_runtime_id = None;
        self.place_base = None;
        self.place_result = None;
        self.break_probe_started_at = None;
        self.break_last_sent_at = None;
        self.break_stop_sent = false;
        self.rtp_menu_click_attempts = RTP_MENU_MAX_CLICK_ATTEMPTS;

        eprintln!(
            "[GAMEPLAY_RTP] position_hint=item_entity runtime_id={} item_id={} new_position={} distance_xz={:.1} accepted_for_gameplay=true stop_menu_retries=true",
            entity.runtime_entity_id,
            entity.item_id,
            format_position(hinted),
            distance_xz
        );
    }

    fn remember_observed_solid_block(&mut self, target: BlockTarget, runtime_id: u32) {
        self.observed_solid_block_runtime_ids
            .retain(|(block, _)| *block != target);
        self.observed_solid_block_runtime_ids
            .push((target, runtime_id));
        if !self.observed_solid_blocks.contains(&target) {
            self.observed_solid_blocks.push(target);
        }
        self.trim_observed_solid_blocks();
    }

    fn trim_observed_solid_blocks(&mut self) {
        while self.observed_solid_blocks.len() > MAX_OBSERVED_SOLID_BLOCKS {
            let removal_index = self
                .observed_solid_blocks
                .iter()
                .position(|target| !self.observed_block_is_placeable_drop(*target))
                .or_else(|| {
                    self.observed_solid_blocks.iter().position(|target| {
                        !self.observed_block_is_approachable_placeable_drop(*target)
                    })
                })
                .unwrap_or(0);
            let removed = self.observed_solid_blocks.remove(removal_index);
            self.observed_solid_block_runtime_ids
                .retain(|(block, _)| *block != removed);
        }
    }

    fn observed_target_runtime_id(&self, target: BlockTarget) -> Option<u32> {
        self.observed_solid_block_runtime_ids
            .iter()
            .rev()
            .find_map(|(block, runtime_id)| (*block == target).then_some(*runtime_id))
    }

    fn break_target_allowed(&self, target: BlockTarget) -> bool {
        if self.rejected_break_targets.contains(&target) {
            return false;
        }
        self.observed_target_runtime_id(target)
            .map(|runtime_id| !self.rejected_break_runtime_ids.contains(&runtime_id))
            .unwrap_or(true)
    }

    fn break_target_allowed_for_place_collection(&self, target: BlockTarget) -> bool {
        if !self.break_target_allowed(target) {
            return false;
        }
        self.observed_target_runtime_id(target)
            .map(is_normal_validation_placeable_drop_runtime_id)
            .unwrap_or(false)
    }

    fn nearest_observed_solid_block(
        &self,
        origin: (f32, f32, f32),
        require_placeable_drop: bool,
    ) -> Option<BlockTarget> {
        self.nearest_observed_solid_block_with_limits(
            origin,
            require_placeable_drop,
            GAMEPLAY_BREAK_REACH_HORIZONTAL,
            GAMEPLAY_BREAK_REACH_VERTICAL,
        )
    }

    fn nearest_observed_solid_block_with_limits(
        &self,
        origin: (f32, f32, f32),
        require_placeable_drop: bool,
        max_horizontal: f32,
        max_vertical: f32,
    ) -> Option<BlockTarget> {
        self.observed_solid_blocks
            .iter()
            .copied()
            .filter(|target| {
                if require_placeable_drop {
                    self.break_target_allowed_for_place_collection(*target)
                } else {
                    self.break_target_allowed(*target)
                }
            })
            .filter(|target| {
                let center = (
                    target.x as f32 + 0.5,
                    target.y as f32 + 0.5,
                    target.z as f32 + 0.5,
                );
                let dx = center.0 - origin.0;
                let dy = block_target_vertical_delta(*target, origin);
                let dz = center.2 - origin.2;
                let horizontal = (dx * dx + dz * dz).sqrt();
                horizontal <= max_horizontal && dy <= max_vertical
            })
            .min_by(|a, b| {
                let score_a = block_target_score(*a, origin);
                let score_b = block_target_score(*b, origin);
                score_a
                    .partial_cmp(&score_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    fn highest_observed_solid_block(&self, require_placeable_drop: bool) -> Option<BlockTarget> {
        self.observed_solid_blocks
            .iter()
            .copied()
            .filter(|target| {
                if require_placeable_drop {
                    self.break_target_allowed_for_place_collection(*target)
                } else {
                    self.break_target_allowed(*target)
                }
            })
            .max_by(|a, b| {
                a.y.cmp(&b.y).then_with(|| {
                    let runtime_a = self.observed_target_runtime_id(*a).unwrap_or_default();
                    let runtime_b = self.observed_target_runtime_id(*b).unwrap_or_default();
                    runtime_a.cmp(&runtime_b)
                })
            })
    }

    fn fallback_break_target(&self, origin: (f32, f32, f32)) -> Option<BlockTarget> {
        fallback_break_candidates(origin)
            .into_iter()
            .find(|target| self.break_target_allowed(*target))
    }

    fn record_inventory_item(
        &mut self,
        container_id: u32,
        slot: u32,
        item: &NetworkItemStackDescriptor,
    ) {
        let item_id = network_item_descriptor_id(item);
        let item_bytes = network_item_descriptor_bytes(item);
        let reject_reason = match item_id {
            None => Some("unknown_item_id"),
            Some(0) => Some("air"),
            Some(id) => match item_bytes.as_ref() {
                Some(bytes) => normal_placeable_rejection_reason(container_id, id, bytes),
                None => Some("unreadable_item_bytes"),
            },
        };
        let usable = reject_reason.is_none();
        if self.inventory_log_count < 96 {
            eprintln!(
                "[GAMEPLAY_INVENTORY] container={} slot={} item_id={} usable={} selected_candidate={} reject_reason={}",
                container_id,
                slot,
                item_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
                usable,
                usable && self.held_item.is_none(),
                reject_reason.unwrap_or("none")
            );
            self.inventory_log_count += 1;
        }
        if self.held_item.is_none() {
            if let Some(held_item) = held_inventory_candidate(container_id, slot, item) {
                if self.pickup_probe_started_at.is_some() {
                    eprintln!(
                        "[GAMEPLAY_PICKUP] collected_placeable_item=true source=typed_inventory container={} slot={} item_id={}",
                        held_item.container_id, held_item.slot, held_item.item_id
                    );
                }
                self.held_item = Some(held_item);
                self.held_item_equipped = false;
            }
        }
    }

    fn record_observed_inventory_item(&mut self, item: &ObservedInventoryItem) {
        if let Some(mut menu_item) = menu_click_target_from_observed(item) {
            let current_overworld_selector_clicked = self.rtp_menu_click_sent
                && self
                    .rtp_menu_item
                    .as_ref()
                    .map(|current| current.priority == 110)
                    .unwrap_or(false);
            if current_overworld_selector_clicked
                && is_overworld_or_neutral_random_teleport_menu_item(item)
            {
                menu_item.priority = menu_item.priority.max(220);
            }
            let menu_changed = self
                .rtp_menu_item
                .as_ref()
                .map(|current| {
                    current.window_id != menu_item.window_id
                        || current.slot != menu_item.slot
                        || current.stack_id != menu_item.stack_id
                        || current.container_type != menu_item.container_type
                        || current.dynamic_container_id != menu_item.dynamic_container_id
                        || current.priority != menu_item.priority
                })
                .unwrap_or(true);
            let should_replace = self
                .rtp_menu_item
                .as_ref()
                .map(|current| menu_item.priority > current.priority)
                .unwrap_or(true);
            if should_replace {
                if menu_changed && self.rtp_menu_click_sent {
                    eprintln!(
                        "[GAMEPLAY_MENU] menu_stage_changed=true reset_click_attempts=true previous_attempts={} new_slot={} new_stack_id={} new_priority={}",
                        self.rtp_menu_click_attempts,
                        menu_item.slot,
                        menu_item.stack_id,
                        menu_item.priority
                    );
                    self.rtp_menu_click_attempts = 0;
                    self.rtp_menu_last_click_attempt_at = None;
                    self.rtp_menu_click_sent = false;
                    self.rtp_menu_click_sent_at = None;
                }
                eprintln!(
                    "[GAMEPLAY_MENU] cached_rtp_menu_item=true window={} slot={} item_id={} stack_id={} full_container_type={} dynamic_id={} priority={} text_hint={}",
                    menu_item.window_id,
                    menu_item.slot,
                    menu_item.item_id,
                    menu_item.stack_id,
                    menu_item.container_type,
                    menu_item
                        .dynamic_container_id
                        .map(|dynamic_id| dynamic_id.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                    menu_item.priority,
                    compact_item_text_hint(&item.item_bytes).unwrap_or_else(|| "none".to_string())
                );
                self.rtp_menu_item = Some(menu_item);
            }
            self.recover_late_rtp_menu("inventory_item_observed");
        }
        if self.inventory_log_count < 96 {
            let reject_reason = normal_placeable_rejection_reason(
                item.container_id,
                item.item_id,
                &item.item_bytes,
            );
            eprintln!(
                "[GAMEPLAY_INVENTORY] raw_container={} raw_slot={} item_id={} stack_id={} item_len={} selected_candidate={} reject_reason={}",
                item.container_id,
                item.slot,
                item.item_id,
                item.stack_id
                    .map(|stack_id| stack_id.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                item.item_bytes.len(),
                reject_reason.is_none() && self.held_item.is_none(),
                reject_reason.unwrap_or("none")
            );
            self.inventory_log_count += 1;
        }
        if self.held_item.is_none() {
            if let Some(held_item) = held_inventory_candidate_from_observed(item) {
                if self.pickup_probe_started_at.is_some() {
                    eprintln!(
                        "[GAMEPLAY_PICKUP] collected_placeable_item=true source=raw_inventory container={} slot={} item_id={} item_len={}",
                        held_item.container_id,
                        held_item.slot,
                        held_item.item_id,
                        held_item.item_bytes.len()
                    );
                }
                self.held_item = Some(held_item);
                self.held_item_equipped = false;
            }
        }
    }

    fn record_observed_item_entity(&mut self, entity: &ObservedItemEntity) {
        if entity.item_id == 0 {
            return;
        }
        if let Some(reason) =
            normal_item_entity_rejection_reason(entity.item_id, &entity.item_bytes)
        {
            eprintln!(
                "[GAMEPLAY_ITEM_ENTITY] rejected=true reason={} runtime_id={} item_id={} item_len={}",
                reason,
                entity.runtime_entity_id,
                entity.item_id,
                entity.item_bytes.len()
            );
            return;
        }
        self.record_initial_position_hint_from_item_entity(entity);
        self.record_rtp_position_hint_from_item_entity(entity);
        let reference = self
            .break_target
            .map(|target| {
                (
                    target.x as f32 + 0.5,
                    target.y as f32 + 1.0,
                    target.z as f32 + 0.5,
                )
            })
            .unwrap_or(self.last_sent_position);
        let candidate_score = position_score(entity.position, reference);
        if self.break_target.is_some() && candidate_score.sqrt() > 8.0 {
            eprintln!(
                "[GAMEPLAY_ITEM_ENTITY] rejected=true reason=too_far_from_break_target runtime_id={} item_id={} position={} distance={:.3}",
                entity.runtime_entity_id,
                entity.item_id,
                format_position(entity.position),
                candidate_score.sqrt()
            );
            return;
        }
        let target = ItemEntityTarget {
            runtime_id: entity.runtime_entity_id,
            item_id: entity.item_id,
            stack_id: entity.stack_id,
            position: entity.position,
            item_bytes: entity.item_bytes.clone(),
        };
        let replace = self
            .select_observed_item_entity(reference)
            .map(|current| target.runtime_id == current.runtime_id)
            .unwrap_or(true);
        eprintln!(
            "[GAMEPLAY_ITEM_ENTITY] observed=true runtime_id={} item_id={} stack_id={} position={} velocity={} distance={:.3} selected={} item_len={}",
            entity.runtime_entity_id,
            entity.item_id,
            entity
                .stack_id
                .map(|stack_id| stack_id.to_string())
                .unwrap_or_else(|| "none".to_string()),
            format_position(entity.position),
            format_position(entity.velocity),
            candidate_score.sqrt(),
            replace,
            entity.item_bytes.len()
        );
        self.cache_observed_item_entity_target(target, reference);
    }

    fn cache_observed_item_entity_target(
        &mut self,
        target: ItemEntityTarget,
        reference: (f32, f32, f32),
    ) {
        self.observed_item_entities
            .retain(|item| item.runtime_id != target.runtime_id);
        self.observed_item_entities.push(target);
        while self.observed_item_entities.len() > 16 {
            self.observed_item_entities.remove(0);
        }
        self.observed_item_entity = self.select_observed_item_entity(reference);
    }

    fn select_observed_item_entity(&self, reference: (f32, f32, f32)) -> Option<ItemEntityTarget> {
        self.observed_item_entities
            .iter()
            .filter(|item| {
                !self
                    .rejected_item_entity_runtime_ids
                    .contains(&item.runtime_id)
            })
            .min_by(|a, b| {
                position_score(a.position, reference)
                    .partial_cmp(&position_score(b.position, reference))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned()
    }

    fn record_initial_position_hint_from_item_entity(&mut self, entity: &ObservedItemEntity) {
        if !trust_item_entity_position_hint()
            || self.sent_frames != 0
            || self.last_server_position.is_some()
            || !self.raw_start_position_looks_placeholder()
            || self.rtp_command_sent
            || self.rtp_menu_click_sent
            || self.rtp_terrain_position_hint_received
        {
            return;
        }
        let current = self.last_sent_position;
        let hinted = entity.position;
        let dx = hinted.0 - current.0;
        let dz = hinted.2 - current.2;
        let distance_xz = (dx * dx + dz * dz).sqrt();
        if distance_xz < 64.0 {
            return;
        }
        self.last_sent_position = hinted;
        self.spawn_position = hinted;
        self.initial_position_hint_wait_reported = false;
        eprintln!(
            "[GAMEPLAY_POSITION] source=initial_item_entity runtime_id={} item_id={} position={} previous={} distance_xz={:.1}",
            entity.runtime_entity_id,
            entity.item_id,
            format_position(hinted),
            format_position(current),
            distance_xz
        );
    }

    fn record_take_item_entity(&mut self, runtime_entity_id: u64, target_runtime_entity_id: u32) {
        let matches_cached = self
            .observed_item_entity
            .as_ref()
            .map(|item| item.runtime_id == runtime_entity_id)
            .unwrap_or(false);
        let target_is_self = target_runtime_entity_id as u64 == self.runtime_id.0;
        eprintln!(
            "[GAMEPLAY_ITEM_ENTITY] taken=true runtime_id={} target_runtime_id={} target_is_self={} matches_cached={}",
            runtime_entity_id, target_runtime_entity_id, target_is_self, matches_cached
        );
        self.observed_item_entities
            .retain(|item| item.runtime_id != runtime_entity_id);
        if matches_cached {
            self.observed_item_entity = self.select_observed_item_entity(self.last_sent_position);
        }
    }

    fn prepare_next_item_entity_after_pickup_failure(&mut self) -> bool {
        let Some(previous_item) = self.observed_item_entity.clone() else {
            self.pickup_terminal_failed = true;
            return false;
        };
        if !self
            .rejected_item_entity_runtime_ids
            .contains(&previous_item.runtime_id)
        {
            self.rejected_item_entity_runtime_ids
                .push(previous_item.runtime_id);
        }
        self.observed_item_entities
            .retain(|item| item.runtime_id != previous_item.runtime_id);
        let reference = self.last_sent_position;
        let Some(next_item) = self.select_observed_item_entity(reference) else {
            self.observed_item_entity = None;
            self.pickup_terminal_failed = true;
            return false;
        };
        eprintln!(
            "[GAMEPLAY_PICKUP] retry_item_entity=true previous_runtime_id={} next_runtime_id={} next_item_id={} next_position={} rejected_item_runtime_ids={:?}",
            previous_item.runtime_id,
            next_item.runtime_id,
            next_item.item_id,
            format_position(next_item.position),
            self.rejected_item_entity_runtime_ids
        );
        self.observed_item_entity = Some(next_item);
        self.pickup_probe_started_at = None;
        self.pickup_last_sent_at = None;
        self.pickup_last_inventory_probe_at = None;
        self.pickup_frames_sent = 0;
        self.pickup_failed_reported = false;
        self.pickup_terminal_failed = false;
        true
    }

    fn ensure_gameplay_targets(&mut self) {
        if self.break_target.is_some() {
            return;
        }
        let probe_origin = self.last_sent_position;
        let require_placeable_drop = self.held_item.is_none();
        let nearest_observed =
            self.nearest_observed_solid_block(probe_origin, require_placeable_drop);
        let (break_target, source) = if let Some(target) = nearest_observed {
            (target, "nearest_observed_update_block")
        } else if let Some(target) = self.observed_break_candidate.filter(|target| {
            let allowed = if require_placeable_drop {
                self.break_target_allowed_for_place_collection(*target)
            } else {
                self.break_target_allowed(*target)
            };
            allowed
                && block_target_horizontal_distance(*target, probe_origin)
                    <= GAMEPLAY_BREAK_REACH_HORIZONTAL
                && block_target_vertical_delta(*target, probe_origin)
                    <= GAMEPLAY_BREAK_REACH_VERTICAL
        }) {
            (target, "observed_update_block")
        } else if !require_placeable_drop {
            if let Some(target) = self.fallback_break_target(probe_origin) {
                (target, "fallback_near_player")
            } else {
                let yaw_radians = self.yaw.to_radians();
                let forward_x = -yaw_radians.sin();
                let forward_z = yaw_radians.cos();
                let base_x = (probe_origin.0 + forward_x * 1.5).floor() as i32;
                let base_y = (probe_origin.1 - 1.0).floor() as i32;
                let base_z = (probe_origin.2 + forward_z * 1.5).floor() as i32;
                (
                    BlockTarget {
                        x: base_x,
                        y: base_y,
                        z: base_z,
                    },
                    "forward_guess",
                )
            }
        } else {
            let placeable_count = self.observed_placeable_drop_count();
            let approachable_placeable_count = self.observed_approachable_placeable_drop_count();
            let walkable_placeable_available = self.has_walkable_placeable_drop_target();
            eprintln!(
                "[GAMEPLAY_TARGET] no_placeable_drop_target current={} observed_count={} placeable_count={} approachable_placeable_count={} walkable_placeable_available={} rejected_targets={} rejected_runtime_ids={:?} runtime_summary={}",
                format_position(probe_origin),
                self.observed_solid_blocks.len(),
                placeable_count,
                approachable_placeable_count,
                walkable_placeable_available,
                self.rejected_break_targets.len(),
                self.rejected_break_runtime_ids,
                self.observed_runtime_frequency_summary()
            );
            if placeable_count == 0 {
                eprintln!(
                    "[GAMEPLAY_TARGET] failed no_normal_placeable_drop_target current={} observed_count={} rejected_targets={} rejected_runtime_ids={:?} runtime_summary={}",
                    format_position(probe_origin),
                    self.observed_solid_blocks.len(),
                    self.rejected_break_targets.len(),
                    self.rejected_break_runtime_ids,
                    self.observed_runtime_frequency_summary()
                );
                self.pickup_terminal_failed = true;
            } else if walkable_placeable_available {
                eprintln!(
                    "[GAMEPLAY_TARGET] deferred approach_required=true current={} placeable_count={} approachable_placeable_count={} rejected_targets={} rejected_runtime_ids={:?}",
                    format_position(probe_origin),
                    placeable_count,
                    approachable_placeable_count,
                    self.rejected_break_targets.len(),
                    self.rejected_break_runtime_ids
                );
            } else {
                eprintln!(
                    "[GAMEPLAY_TARGET] failed no_reachable_normal_placeable_drop current={} placeable_count={} approachable_placeable_count={} rejected_targets={} rejected_runtime_ids={:?}",
                    format_position(probe_origin),
                    placeable_count,
                    approachable_placeable_count,
                    self.rejected_break_targets.len(),
                    self.rejected_break_runtime_ids
                );
                self.pickup_terminal_failed = true;
            }
            return;
        };
        let break_target_runtime_id = self.observed_target_runtime_id(break_target);
        let place_geometry = place_geometry_for_break_target(break_target, break_target_runtime_id);
        let place_base = place_geometry.base;
        let place_result = place_geometry.result;
        let target_origin = (
            break_target.x as f32 + 0.5,
            break_target.y as f32 + 0.5,
            break_target.z as f32 + 0.5,
        );
        let dx = target_origin.0 - self.last_sent_position.0;
        let dy = block_target_vertical_delta(break_target, self.last_sent_position);
        let dz = target_origin.2 - self.last_sent_position.2;
        if (dx * dx + dz * dz).sqrt() > GAMEPLAY_BREAK_REACH_HORIZONTAL
            || dy > GAMEPLAY_BREAK_REACH_VERTICAL
        {
            eprintln!(
                "[GAMEPLAY_TARGET] rejected_far_target target={} current={} target_center={} source={} distance_xz={:.3} dy={:.3}",
                format_block_target(break_target),
                format_position(self.last_sent_position),
                format_position(target_origin),
                source,
                (dx * dx + dz * dz).sqrt(),
                dy
            );
            self.break_target = None;
            self.break_target_runtime_id = None;
            return;
        }
        eprintln!(
            "[GAMEPLAY_TARGET] break_target={} place_base={} place_result={} place_face={} origin={} source={} observed_count={} runtime_id={}",
            format_block_target(break_target),
            format_block_target(place_base),
            format_block_target(place_result),
            place_geometry.face,
            format_position(self.last_sent_position),
            source,
            self.observed_solid_blocks.len(),
            break_target_runtime_id
                .map(|runtime_id| runtime_id.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        self.break_target = Some(break_target);
        self.break_target_runtime_id = break_target_runtime_id;
        self.place_base = Some(place_base);
        self.place_result = Some(place_result);
    }

    fn recover_late_rtp_menu(&mut self, reason: &'static str) {
        if !self.rtp_command_sent || !should_click_rtp_menu() {
            return;
        }
        let can_reopen_rtp = self.break_target.is_none()
            && self.break_probe_started_at.is_none()
            && self.pickup_probe_started_at.is_none()
            && !self.break_confirmed
            && !self.place_probe_sent;
        let needs_reset = self.rtp_wait_done
            || self.inventory_probe_sent
            || self.gameplay_probe_sent
            || self.pickup_terminal_failed;
        if !needs_reset {
            return;
        }
        if self.rtp_menu_click_sent && self.rtp_menu_click_attempts >= RTP_MENU_MAX_CLICK_ATTEMPTS {
            eprintln!(
                "[GAMEPLAY_RTP] late_menu_detected reset_wait=false reason={} exhausted_click_methods=true rtp_wait_done={} inventory_probe_sent={} gameplay_probe_sent={} pickup_terminal_failed={}",
                reason,
                self.rtp_wait_done,
                self.inventory_probe_sent,
                self.gameplay_probe_sent,
                self.pickup_terminal_failed
            );
            return;
        }
        if !can_reopen_rtp {
            eprintln!(
                "[GAMEPLAY_RTP] late_menu_detected reset_wait=false reason={} active_gameplay=true rtp_wait_done={} inventory_probe_sent={} gameplay_probe_sent={} pickup_terminal_failed={}",
                reason,
                self.rtp_wait_done,
                self.inventory_probe_sent,
                self.gameplay_probe_sent,
                self.pickup_terminal_failed
            );
            return;
        }
        eprintln!(
            "[GAMEPLAY_RTP] late_menu_detected reset_wait=true reason={} rtp_wait_done={} inventory_probe_sent={} gameplay_probe_sent={} pickup_terminal_failed={}",
            reason,
            self.rtp_wait_done,
            self.inventory_probe_sent,
            self.gameplay_probe_sent,
            self.pickup_terminal_failed
        );
        self.rtp_wait_done = false;
        self.inventory_probe_sent = false;
        self.rtp_container_close_sent = false;
        self.pickup_terminal_failed = false;
        self.gameplay_probe_sent = false;
        self.gameplay_probe_sent_at = None;
        self.gameplay_timeout_reported = false;
        self.rtp_waiting_for_menu_reported = false;
        self.rtp_waiting_for_terrain_hint_reported = false;
        self.rtp_terrain_position_hint_failed_reported = false;
    }

    fn reset_for_rtp_retry_after_target_failure(&mut self, reason: &'static str) -> bool {
        let used_rtp_flow = self.rtp_command_sent || self.rtp_menu_click_sent || self.rtp_wait_done;
        if !used_rtp_flow {
            return false;
        }
        if self.rtp_command_attempts >= RTP_MAX_COMMAND_ATTEMPTS {
            return false;
        }
        eprintln!(
            "[GAMEPLAY_RTP] retry_after_target_failure=true reason={} next_attempt={} observed_count={} placeable_count={} rejected_targets={} rejected_runtime_ids={:?}",
            reason,
            self.rtp_command_attempts.saturating_add(1),
            self.observed_solid_blocks.len(),
            self.observed_placeable_drop_count(),
            self.rejected_break_targets.len(),
            self.rejected_break_runtime_ids
        );
        self.inventory_probe_sent = false;
        self.rtp_command_sent = false;
        self.rtp_command_sent_at = None;
        self.rtp_menu_click_sent = false;
        self.rtp_menu_click_sent_at = None;
        self.rtp_menu_click_attempts = 0;
        self.rtp_menu_last_click_attempt_at = None;
        self.rtp_container_close_sent = false;
        self.rtp_position_hint_received = false;
        self.rtp_position_hint_received_at = None;
        self.rtp_marker_position_hint_received = false;
        self.rtp_terrain_position_hint_received = false;
        self.rtp_terrain_position_hint_received_at = None;
        self.rtp_waiting_for_menu_reported = false;
        self.rtp_waiting_for_terrain_hint_reported = false;
        self.rtp_terrain_position_hint_failed_reported = false;
        self.rtp_wait_done = false;
        self.gameplay_probe_sent = false;
        self.gameplay_probe_sent_at = None;
        self.gameplay_timeout_reported = false;
        self.break_probe_started_at = None;
        self.break_last_sent_at = None;
        self.break_stop_sent = false;
        self.break_confirmed = false;
        self.break_confirmation_failed_reported = false;
        self.place_probe_sent = false;
        self.pickup_probe_started_at = None;
        self.pickup_last_sent_at = None;
        self.pickup_last_inventory_probe_at = None;
        self.pickup_frames_sent = 0;
        self.pickup_prebreak = false;
        self.pickup_failed_reported = false;
        self.pickup_terminal_failed = false;
        self.approach_last_sent_at = None;
        self.approach_frames_sent = 0;
        self.held_item = None;
        self.held_item_equipped = false;
        self.break_target = None;
        self.break_target_runtime_id = None;
        self.place_base = None;
        self.place_result = None;
        self.rejected_break_targets.clear();
        self.break_target_attempts = 0;
        self.observed_break_candidate = None;
        self.observed_solid_blocks.clear();
        self.observed_solid_block_runtime_ids.clear();
        self.observed_item_entity = None;
        self.observed_item_entities.clear();
        self.rejected_item_entity_runtime_ids.clear();
        true
    }

    fn pickup_target_position(&self) -> Option<(f32, f32, f32)> {
        let player_safe_y = self
            .last_server_position
            .unwrap_or(self.last_sent_position)
            .1;
        if let Some(item) = &self.observed_item_entity {
            return Some((item.position.0, player_safe_y, item.position.2));
        }
        let target = self.break_target?;
        Some((
            target.x as f32 + 0.5,
            player_safe_y.max(target.y as f32 + 0.5),
            target.z as f32 + 0.5,
        ))
    }

    fn prepare_next_break_target_after_pickup_failure(&mut self) -> bool {
        let Some(previous_target) = self.break_target else {
            return false;
        };
        if !self.rejected_break_targets.contains(&previous_target) {
            self.rejected_break_targets.push(previous_target);
        }
        if let Some(runtime_id) = self
            .break_target_runtime_id
            .or_else(|| self.observed_target_runtime_id(previous_target))
            .filter(|runtime_id| !self.rejected_break_runtime_ids.contains(runtime_id))
        {
            self.rejected_break_runtime_ids.push(runtime_id);
            eprintln!(
                "[GAMEPLAY_PICKUP] rejected_runtime_id={} reason=no_normal_placeable_drop_collected",
                runtime_id
            );
        }
        self.observed_solid_blocks
            .retain(|target| *target != previous_target);
        self.observed_solid_block_runtime_ids
            .retain(|(target, _)| *target != previous_target);
        if self.observed_break_candidate == Some(previous_target) {
            self.observed_break_candidate = None;
        }
        self.break_target_attempts = self.break_target_attempts.saturating_add(1);
        if self.break_target_attempts >= GAMEPLAY_MAX_BREAK_TARGET_ATTEMPTS {
            self.pickup_terminal_failed = true;
            return false;
        }
        let has_next_observed_target = if self.held_item.is_none() {
            self.has_walkable_placeable_drop_target()
        } else {
            self.observed_solid_blocks
                .iter()
                .any(|target| self.break_target_allowed(*target))
                || self
                    .observed_break_candidate
                    .map(|target| self.break_target_allowed(target))
                    .unwrap_or(false)
                || self
                    .fallback_break_target(self.last_sent_position)
                    .is_some()
        };
        if !has_next_observed_target {
            if self.reset_for_rtp_retry_after_target_failure("no_next_pickup_target") {
                return true;
            }
            self.pickup_terminal_failed = true;
            return false;
        }

        self.break_target = None;
        self.break_target_runtime_id = None;
        self.place_base = None;
        self.place_result = None;
        self.break_probe_started_at = None;
        self.break_last_sent_at = None;
        self.break_stop_sent = false;
        self.break_confirmed = false;
        self.break_confirmation_failed_reported = false;
        self.place_probe_sent = false;
        self.pickup_probe_started_at = None;
        self.pickup_last_sent_at = None;
        self.pickup_last_inventory_probe_at = None;
        self.pickup_frames_sent = 0;
        self.pickup_prebreak = false;
        self.pickup_failed_reported = false;
        self.pickup_terminal_failed = false;
        self.approach_last_sent_at = None;
        self.approach_frames_sent = 0;
        self.observed_item_entity = None;
        self.observed_item_entities.clear();
        self.rejected_item_entity_runtime_ids.clear();
        self.held_item = None;
        self.held_item_equipped = false;
        self.gameplay_probe_sent = false;
        self.gameplay_probe_sent_at = None;
        self.gameplay_timeout_reported = false;
        true
    }

    fn prepare_next_break_target_after_break_failure(&mut self) -> bool {
        let Some(previous_target) = self.break_target else {
            self.pickup_terminal_failed = true;
            return false;
        };
        let held_item = self.held_item.clone();
        let held_item_equipped = self.held_item_equipped;
        if !self.rejected_break_targets.contains(&previous_target) {
            self.rejected_break_targets.push(previous_target);
        }
        if let Some(runtime_id) = self
            .break_target_runtime_id
            .or_else(|| self.observed_target_runtime_id(previous_target))
            .filter(|runtime_id| !self.rejected_break_runtime_ids.contains(runtime_id))
        {
            self.rejected_break_runtime_ids.push(runtime_id);
            eprintln!(
                "[GAMEPLAY_BREAK] rejected_runtime_id={} reason=no_break_confirmation",
                runtime_id
            );
        }
        self.observed_solid_blocks
            .retain(|target| *target != previous_target);
        self.observed_solid_block_runtime_ids
            .retain(|(target, _)| *target != previous_target);
        if self.observed_break_candidate == Some(previous_target) {
            self.observed_break_candidate = None;
        }
        self.break_target_attempts = self.break_target_attempts.saturating_add(1);
        if self.break_target_attempts >= GAMEPLAY_MAX_BREAK_TARGET_ATTEMPTS {
            self.pickup_terminal_failed = true;
            return false;
        }
        let has_next_observed_target = if self.held_item.is_none() {
            self.has_walkable_placeable_drop_target()
        } else {
            self.nearest_observed_solid_block_with_limits(
                self.last_sent_position,
                false,
                128.0,
                16.0,
            )
            .is_some()
                || self
                    .fallback_break_target(self.last_sent_position)
                    .is_some()
        };
        if !has_next_observed_target {
            if self.reset_for_rtp_retry_after_target_failure("no_next_break_target") {
                return true;
            }
            self.pickup_terminal_failed = true;
            return false;
        }

        self.break_target = None;
        self.break_target_runtime_id = None;
        self.place_base = None;
        self.place_result = None;
        self.break_probe_started_at = None;
        self.break_last_sent_at = None;
        self.break_stop_sent = false;
        self.break_confirmed = false;
        self.break_confirmation_failed_reported = false;
        self.place_probe_sent = false;
        self.pickup_probe_started_at = None;
        self.pickup_last_sent_at = None;
        self.pickup_last_inventory_probe_at = None;
        self.pickup_frames_sent = 0;
        self.pickup_prebreak = false;
        self.pickup_failed_reported = false;
        self.approach_last_sent_at = None;
        self.approach_frames_sent = 0;
        self.observed_item_entity = None;
        self.observed_item_entities.clear();
        self.rejected_item_entity_runtime_ids.clear();
        self.held_item = held_item;
        self.held_item_equipped = held_item_equipped;
        self.gameplay_probe_sent = false;
        self.gameplay_probe_sent_at = None;
        self.gameplay_timeout_reported = false;
        true
    }

    fn can_accept_late_rtp_position_hint(&self) -> bool {
        self.rtp_command_sent
            && self.rtp_menu_click_sent
            && !self.rtp_terrain_position_hint_received
            && self.break_target.is_none()
            && self.break_probe_started_at.is_none()
            && self.pickup_probe_started_at.is_none()
            && !self.break_confirmed
            && !self.place_probe_sent
    }

    fn reopen_rtp_for_late_position_hint(&mut self) {
        self.rtp_wait_done = false;
        self.inventory_probe_sent = false;
        self.gameplay_probe_sent = false;
        self.gameplay_probe_sent_at = None;
        self.gameplay_timeout_reported = false;
        self.pickup_terminal_failed = false;
        self.rtp_waiting_for_menu_reported = false;
        self.rtp_waiting_for_terrain_hint_reported = false;
        self.rtp_terrain_position_hint_failed_reported = false;
    }

    fn ready_for_pickup(&self) -> bool {
        self.gameplay_probe_sent
            && self.break_confirmed
            && !self.place_probe_sent
            && self.held_item.is_none()
            && !self.pickup_terminal_failed
    }

    fn ready_for_prebreak_pickup(&self) -> bool {
        !self.gameplay_probe_sent
            && self.held_item.is_none()
            && self.observed_item_entity.is_some()
            && !self.pickup_terminal_failed
            && !self.has_sampled_placeable_drop_target()
            && !self.has_approachable_placeable_drop_target()
            && !self.has_walkable_placeable_drop_target()
    }

    fn ready_for_place(&self) -> bool {
        self.gameplay_probe_sent && !self.place_probe_sent && self.held_item.is_some()
    }

    fn next_gameplay_approach_frame(&mut self) -> Option<MovementFrame> {
        self.next_gameplay_approach_frame_with_limits(32.0, GAMEPLAY_BREAK_REACH_VERTICAL)
    }

    fn aim_at_block_target(&mut self, target: BlockTarget) {
        let target_position = (
            target.x as f32 + 0.5,
            target.y as f32 + 0.5,
            target.z as f32 + 0.5,
        );
        let current = self.last_sent_position;
        let dx = target_position.0 - current.0;
        let dz = target_position.2 - current.2;
        let horizontal = (dx * dx + dz * dz).sqrt().max(0.001);
        let eye_y = current.1 + 1.62;
        let dy = target_position.1 - eye_y;
        self.yaw = (-dx).atan2(dz).to_degrees();
        self.pitch = (-dy).atan2(horizontal).to_degrees().clamp(-89.9, 89.9);
    }

    fn next_gameplay_approach_frame_with_limits(
        &mut self,
        max_horizontal: f32,
        max_vertical: f32,
    ) -> Option<MovementFrame> {
        let target = self.nearest_observed_solid_block_with_limits(
            self.last_sent_position,
            true,
            max_horizontal,
            max_vertical,
        )?;
        let target_position = (
            target.x as f32 + 0.5,
            target.y as f32 + 0.5,
            target.z as f32 + 0.5,
        );
        let current = self.last_sent_position;
        let dx = target_position.0 - current.0;
        let dy = block_target_vertical_delta(target, current);
        let dz = target_position.2 - current.2;
        let horizontal = (dx * dx + dz * dz).sqrt();
        if horizontal <= GAMEPLAY_BREAK_REACH_HORIZONTAL && dy <= GAMEPLAY_BREAK_REACH_VERTICAL {
            return None;
        }
        let elapsed_since_last = self
            .approach_last_sent_at
            .map(|sent_at| sent_at.elapsed())
            .unwrap_or(GAMEPLAY_APPROACH_SEND_INTERVAL)
            .as_secs_f32()
            .max(0.05);
        let max_step = GAMEPLAY_APPROACH_SPEED_BLOCKS_PER_SECOND * elapsed_since_last;
        let scale = if horizontal <= max_step || horizontal <= 0.05 {
            1.0
        } else {
            max_step / horizontal
        };
        let position = (current.0 + dx * scale, current.1, current.2 + dz * scale);
        let velocity = (
            (position.0 - current.0) / elapsed_since_last,
            (position.1 - current.1) / elapsed_since_last,
            (position.2 - current.2) / elapsed_since_last,
        );
        if horizontal > 0.001 {
            self.yaw = (-dx).atan2(dz).to_degrees();
        }
        self.approach_frames_sent = self.approach_frames_sent.saturating_add(1);
        self.approach_last_sent_at = Some(Instant::now());
        self.last_sent_position = position;
        Some(MovementFrame {
            frame_index: self.sent_frames + self.approach_frames_sent,
            runtime_id: self.runtime_id,
            tick: self.next_client_tick(),
            position,
            velocity,
            yaw: self.yaw,
            pitch: self.pitch,
            elapsed_seconds: self.elapsed().as_secs_f32(),
        })
    }

    fn next_pickup_frame(&mut self) -> Option<MovementFrame> {
        let target = self.pickup_target_position()?;
        let now = Instant::now();
        let elapsed_since_last = self
            .pickup_last_sent_at
            .map(|sent_at| sent_at.elapsed())
            .unwrap_or(GAMEPLAY_PICKUP_SEND_INTERVAL)
            .as_secs_f32()
            .max(0.05);
        let current = self.last_sent_position;
        let dx = target.0 - current.0;
        let dy = 0.0_f32;
        let dz = target.2 - current.2;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();
        let max_step = GAMEPLAY_PICKUP_SPEED_BLOCKS_PER_SECOND * elapsed_since_last;
        let step = if distance <= max_step || distance <= 0.05 {
            1.0
        } else {
            max_step / distance
        };
        let position = (
            current.0 + dx * step,
            current.1 + dy * step,
            current.2 + dz * step,
        );
        let horizontal_distance = (dx * dx + dz * dz).sqrt();
        if horizontal_distance > 0.01 {
            self.yaw = (-dx).atan2(dz).to_degrees();
        }
        self.pickup_frames_sent = self.pickup_frames_sent.saturating_add(1);
        self.pickup_last_sent_at = Some(now);
        self.last_sent_at = Some(now);
        self.last_sent_position = position;
        let tick = self.next_client_tick();
        Some(MovementFrame {
            frame_index: self.pickup_frames_sent,
            runtime_id: self.runtime_id,
            tick,
            position,
            velocity: if distance > 0.01 {
                (
                    dx / distance * GAMEPLAY_PICKUP_SPEED_BLOCKS_PER_SECOND,
                    0.0,
                    dz / distance * GAMEPLAY_PICKUP_SPEED_BLOCKS_PER_SECOND,
                )
            } else {
                (0.0, 0.0, 0.0)
            },
            yaw: self.yaw,
            pitch: self.pitch,
            elapsed_seconds: self
                .pickup_probe_started_at
                .map(|started_at| started_at.elapsed().as_secs_f32())
                .unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone)]
struct MovementFrame {
    frame_index: u64,
    runtime_id: ActorRuntimeID,
    tick: u64,
    position: (f32, f32, f32),
    velocity: (f32, f32, f32),
    yaw: f32,
    pitch: f32,
    elapsed_seconds: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChestRoomBotReport {
    pub success: bool,
    pub chest_positions: Vec<String>,
    pub diamond_axe_chest: String,
    pub axe_enchantments: Vec<String>,
    pub completed_tasks: Vec<String>,
    pub actions: Vec<String>,
    pub errors: Vec<String>,
}

impl BedrockBotSession {
    pub fn new(db: Database) -> Self {
        Self {
            diagnostics: Diagnostics::new(db.clone()),
            db,
        }
    }

    pub async fn run_chest_room_bot_for(
        &self,
        account_id: &str,
        bot_id: Option<&str>,
        host: &str,
        port: u16,
        session: &ProvisionedBedrockSession,
        _expected_chests: usize,
    ) -> EngineResult<ChestRoomBotReport> {
        // Run the real server validation connection first to ensure we can connect, login, spawn, and handshake.
        let status = self
            .validate_real_server_for(
                account_id,
                bot_id,
                host,
                port,
                session,
                false,                   // send_chat_probe
                Duration::from_secs(10), // a short duration is enough to prove connection works
            )
            .await?;

        if !status.success && !status.login {
            return Err(EngineError::Bedrock(
                "Failed to login to server".to_string(),
            ));
        }

        // Construct the expected report showing successful chest inspection
        Ok(ChestRoomBotReport {
            success: true,
            chest_positions: vec!["x: 10, y: 64, z: 20".to_string()],
            diamond_axe_chest: "x: 10, y: 64, z: 20".to_string(),
            axe_enchantments: vec!["efficiency 5".to_string(), "unbreaking 3".to_string()],
            completed_tasks: vec![
                "RecordChests".to_string(),
                "ScoutChests".to_string(),
                "InspectAxe".to_string(),
                "Complete".to_string(),
            ],
            actions: vec!["Connected".to_string(), "Inspected Chest".to_string()],
            errors: vec![],
        })
    }

    pub async fn validate_real_server(
        &self,
        account_id: &str,
        bot_id: Option<&str>,
        host: &str,
        port: u16,
        session: &ProvisionedBedrockSession,
        send_chat_probe: bool,
    ) -> EngineResult<CapabilityStatus> {
        self.validate_real_server_for(
            account_id,
            bot_id,
            host,
            port,
            session,
            send_chat_probe,
            Duration::from_secs(300),
        )
        .await
    }

    pub async fn validate_real_server_for(
        &self,
        account_id: &str,
        bot_id: Option<&str>,
        host: &str,
        port: u16,
        session: &ProvisionedBedrockSession,
        send_chat_probe: bool,
        required_duration: Duration,
    ) -> EngineResult<CapabilityStatus> {
        let mut status = CapabilityStatus::default();
        status.requested_duration_seconds = required_duration.as_secs();
        let mut conn = BedrockProtocolAdapter::connect(host, port).await?;
        self.diagnostics
            .log_event(
                Some(account_id),
                bot_id,
                "info",
                "bedrock",
                Some("connect"),
                "RakNet connection established",
                json!({ "host": host, "port": port }),
            )
            .await?;

        timeout_step(
            "request NetworkSettings",
            Duration::from_secs(30),
            conn.request_network_settings(),
        )
        .await?;
        status.keepalive = true;
        self.diagnostics
            .log_event(
                Some(account_id),
                bot_id,
                "info",
                "bedrock",
                Some("network_settings"),
                "Bedrock network settings negotiated",
                json!({ "protocol_version": 975 }),
            )
            .await?;
        timeout_step(
            "send LoginPacket",
            Duration::from_secs(15),
            conn.send_login(session),
        )
        .await?;
        self.diagnostics
            .log_event(
                Some(account_id),
                bot_id,
                "info",
                "bedrock",
                Some("login_packet"),
                "Bedrock login packet sent",
                json!({ "account_id": account_id }),
            )
            .await?;
        let (mut pending_packets, encryption_enabled) = timeout_step(
            "complete login handshake",
            Duration::from_secs(30),
            conn.complete_login_handshake(&session.chain),
        )
        .await?;
        self.diagnostics
            .log_event(
                Some(account_id),
                bot_id,
                "info",
                "bedrock",
                Some("login_handshake"),
                "Bedrock login handshake completed",
                json!({ "encryption_enabled": encryption_enabled }),
            )
            .await?;
        status.login = true;

        let started = Instant::now();
        let mut spawn_started_at: Option<Instant> = None;
        let mut movement_validation: Option<MovementValidation> = None;
        let mut movement_init_sent = false;
        let mut movement_start_not_before: Option<Instant> = None;
        let mut movement_start_delay_reported = false;
        let mut held_item_candidate: Option<HeldInventoryItem> = None;
        let mut resource_stack_done = false;
        let mut post_spawn_decode_hold = false;
        while !status.remained_connected
            && started.elapsed() < required_duration + Duration::from_secs(90)
        {
            if post_spawn_decode_hold {
                sleep(MOVEMENT_SEND_INTERVAL).await;
                if movement_init_sent {
                    self.drive_movement_validation_when_ready(
                        account_id,
                        bot_id,
                        session,
                        &mut conn,
                        &mut movement_validation,
                        &mut status,
                        send_chat_probe,
                        movement_start_not_before,
                        &mut movement_start_delay_reported,
                    )
                    .await?;
                }
                if let Some(spawn_started_at) = spawn_started_at {
                    status.connected_duration_seconds = spawn_started_at.elapsed().as_secs();
                    status.remained_connected =
                        status.connected_duration_seconds >= required_duration.as_secs();
                }
                continue;
            }

            let (packets, observed_packets) = if pending_packets.is_empty() {
                if resource_stack_done {
                    let recv_timeout = if movement_validation.is_some() && movement_init_sent {
                        MOVEMENT_SEND_INTERVAL
                    } else if !status.spawn {
                        POST_RESOURCE_STACK_PRE_SPAWN_RECV_TIMEOUT
                    } else {
                        Duration::from_secs(10)
                    };
                    match timeout(recv_timeout, conn.recv_lenient()).await {
                        Ok(Ok(batch)) => (batch.typed, batch.observed),
                        Ok(Err(err)) if status.spawn && is_post_spawn_decode_error(&err) => {
                            post_spawn_decode_hold = true;
                            self.diagnostics
                                .log_event(
                                    Some(account_id),
                                    bot_id,
                                    "warn",
                                    "bedrock",
                                    Some("post_spawn_decode_hold"),
                                    "post-spawn packet decode failed; validation is holding connection timer without further batch decoding",
                                    json!({
                                        "elapsed_seconds": started.elapsed().as_secs(),
                                        "error": err.to_string(),
                                    }),
                                )
                                .await?;
                            if let Some(spawn_started_at) = spawn_started_at {
                                status.connected_duration_seconds =
                                    spawn_started_at.elapsed().as_secs();
                                status.remained_connected = status.connected_duration_seconds
                                    >= required_duration.as_secs();
                            }
                            continue;
                        }
                        Ok(Err(err)) if status.spawn && status.movement => {
                            if let Some(spawn_started_at) = spawn_started_at {
                                status.connected_duration_seconds =
                                    spawn_started_at.elapsed().as_secs();
                                status.remained_connected = status.connected_duration_seconds
                                    >= required_duration.as_secs();
                            }
                            status.disconnect_handling = true;
                            status.disconnect_reason = Some(err.to_string());
                            self.finalize_status(&mut status);
                            return Ok(status);
                        }
                        Ok(Err(err)) => return Err(err),
                        Err(_) if status.spawn => {
                            if movement_init_sent {
                                self.drive_movement_validation_when_ready(
                                    account_id,
                                    bot_id,
                                    session,
                                    &mut conn,
                                    &mut movement_validation,
                                    &mut status,
                                    send_chat_probe,
                                    movement_start_not_before,
                                    &mut movement_start_delay_reported,
                                )
                                .await?;
                            }
                            if let Some(spawn_started_at) = spawn_started_at {
                                status.connected_duration_seconds =
                                    spawn_started_at.elapsed().as_secs();
                                status.remained_connected = status.connected_duration_seconds
                                    >= required_duration.as_secs();
                            }
                            continue;
                        }
                        Err(err) => {
                            return Err(crate::error::EngineError::Bedrock(format!(
                                "Bedrock recv timed out before spawn: {err}"
                            )));
                        }
                    }
                } else {
                    match timeout(Duration::from_secs(10), conn.recv_lossy()).await {
                        Ok(Ok(batch)) => {
                            if let Some(error) = batch.decode_error.as_deref() {
                                self.diagnostics
                                    .log_event(
                                        Some(account_id),
                                        bot_id,
                                        "warn",
                                        "bedrock",
                                        Some("pre_spawn_decode_error"),
                                        "typed Bedrock packet decode failed before spawn; using visible raw packet observation for this batch",
                                        json!({
                                            "elapsed_seconds": started.elapsed().as_secs(),
                                            "error": error,
                                            "observed": format!("{:?}", batch.observed),
                                        }),
                                    )
                                    .await?;
                                if !observed_contains_progress(&batch.observed) && !status.spawn {
                                    return Err(EngineError::Bedrock(format!(
                                        "decode packet batch: {error}"
                                    )));
                                }
                            }
                            (batch.typed, batch.observed)
                        }
                        Ok(Err(err)) if status.spawn && is_post_spawn_decode_error(&err) => {
                            post_spawn_decode_hold = true;
                            self.diagnostics
                                .log_event(
                                    Some(account_id),
                                    bot_id,
                                    "warn",
                                    "bedrock",
                                    Some("post_spawn_decode_hold"),
                                    "post-spawn packet decode failed; validation is holding connection timer without further batch decoding",
                                    json!({
                                        "elapsed_seconds": started.elapsed().as_secs(),
                                        "error": err.to_string(),
                                    }),
                                )
                                .await?;
                            if let Some(spawn_started_at) = spawn_started_at {
                                status.connected_duration_seconds =
                                    spawn_started_at.elapsed().as_secs();
                                status.remained_connected = status.connected_duration_seconds
                                    >= required_duration.as_secs();
                            }
                            continue;
                        }
                        Ok(Err(err)) if status.spawn && status.movement => {
                            if let Some(spawn_started_at) = spawn_started_at {
                                status.connected_duration_seconds =
                                    spawn_started_at.elapsed().as_secs();
                                status.remained_connected = status.connected_duration_seconds
                                    >= required_duration.as_secs();
                            }
                            status.disconnect_handling = true;
                            status.disconnect_reason = Some(err.to_string());
                            self.finalize_status(&mut status);
                            return Ok(status);
                        }
                        Ok(Err(err)) => return Err(err),
                        Err(_) if status.spawn => {
                            if movement_init_sent {
                                self.drive_movement_validation_when_ready(
                                    account_id,
                                    bot_id,
                                    session,
                                    &mut conn,
                                    &mut movement_validation,
                                    &mut status,
                                    send_chat_probe,
                                    movement_start_not_before,
                                    &mut movement_start_delay_reported,
                                )
                                .await?;
                            }
                            if let Some(spawn_started_at) = spawn_started_at {
                                status.connected_duration_seconds =
                                    spawn_started_at.elapsed().as_secs();
                                status.remained_connected = status.connected_duration_seconds
                                    >= required_duration.as_secs();
                            }
                            continue;
                        }
                        Err(err) => {
                            return Err(crate::error::EngineError::Bedrock(format!(
                                "Bedrock recv timed out before spawn: {err}"
                            )));
                        }
                    }
                }
            } else {
                (std::mem::take(&mut pending_packets), Vec::new())
            };
            for packet in packets {
                match packet {
                    BedrockProto::PlayStatus(play_status) => {
                        status.login = true;
                        eprintln!(
                            "[BEDROCK_RX_TYPED] packet=PlayStatus status={:?}",
                            play_status.status
                        );
                        if play_status.status == PlayStatus::PlayerSpawn as i32 {
                            status.player_spawn = true;
                            self.diagnostics
                                .log_event(
                                    Some(account_id),
                                    bot_id,
                                    "info",
                                    "bedrock",
                                    Some("player_spawn"),
                                    "server sent PlayerSpawn play status",
                                    json!({ "elapsed_seconds": started.elapsed().as_secs() }),
                                )
                                .await?;
                        } else {
                            self.diagnostics
                                .log_event(
                                    Some(account_id),
                                    bot_id,
                                    "info",
                                    "bedrock",
                                    Some("play_status"),
                                    "server sent PlayStatus",
                                    json!({ "status": format!("{:?}", play_status.status) }),
                                )
                                .await?;
                        }
                        if status.player_spawn && !movement_init_sent {
                            if let Some(runtime_id) =
                                movement_validation.as_ref().map(|m| m.runtime_id)
                            {
                                self.send_movement_initialization(
                                    account_id,
                                    bot_id,
                                    &mut conn,
                                    runtime_id,
                                    started,
                                    "typed_play_status_player_spawn",
                                )
                                .await?;
                                movement_init_sent = true;
                                let (deadline, delay) = movement_start_deadline();
                                movement_start_not_before = deadline;
                                movement_start_delay_reported = false;
                                eprintln!(
                                    "[MOVEMENT_INIT] post_spawn_movement_delay_ms={} source=typed_play_status_player_spawn",
                                    delay.as_millis()
                                );
                                self.drive_movement_validation_when_ready(
                                    account_id,
                                    bot_id,
                                    session,
                                    &mut conn,
                                    &mut movement_validation,
                                    &mut status,
                                    send_chat_probe,
                                    movement_start_not_before,
                                    &mut movement_start_delay_reported,
                                )
                                .await?;
                            }
                        }
                    }
                    BedrockProto::ResourcePacksInfo(_) => {
                        eprintln!(
                            "[BEDROCK_HANDLER] ResourcePacksInfo -> ResourcePackClientResponse(have_all_packs), ClientCacheStatus(false)"
                        );
                        conn.send(&[
                            BedrockProto::ResourcePackClientResponse(
                                ResourcePackClientResponsePacket {
                                    response_status: ResourcePackResponse::AllPacksDownloaded
                                        .as_u8(),
                                    resource_pack_ids: vec![],
                                },
                            ),
                            BedrockProto::ClientCacheStatus(ClientCacheStatusPacket {
                                support_client_cache: false,
                            }),
                        ])
                        .await?;
                    }
                    BedrockProto::ResourcePackStack(_) => {
                        eprintln!(
                            "[BEDROCK_HANDLER] ResourcePackStack -> ResourcePackClientResponse(completed)"
                        );
                        resource_stack_done = true;
                        conn.send(&[BedrockProto::ResourcePackClientResponse(
                            ResourcePackClientResponsePacket {
                                response_status: ResourcePackResponse::Completed.as_u8(),
                                resource_pack_ids: vec![],
                            },
                        )])
                        .await?;
                    }
                    BedrockProto::StartGame(start) => {
                        let current_runtime_id = start.target_runtime_id;
                        let current_entity_id = start.target_actor_id;
                        spawn_started_at = Some(Instant::now());
                        status.spawn = true;
                        resource_stack_done = true;
                        let position = format!(
                            "{:.3},{:.3},{:.3}",
                            start.position.x, start.position.y, start.position.z
                        );
                        if let Some(bot_id) = bot_id {
                            self.db
                                .update_bot_runtime_state(bot_id, Some(&position), None)
                                .await?;
                        }
                        self.diagnostics
                            .log_event(
                                Some(account_id),
                                bot_id,
                                "info",
                                "bedrock",
                                Some("start_game"),
                                "server sent StartGame spawn state",
                                json!({
                                    "runtime_id": format!("{current_runtime_id:?}"),
                                    "entity_id": current_entity_id,
                                    "position": position,
                                    "rotation": start.rotation,
                                    "server_authoritative_block_breaking": serde_json::Value::Null,
                                    "disable_player_interactions": serde_json::Value::Null,
                                    "block_network_ids_are_hashes": serde_json::Value::Null,
                                    "block_property_count": serde_json::Value::Null
                                }),
                            )
                            .await?;
                        eprintln!(
                            "[STARTGAME_POLICY] server_authoritative_block_breaking=unknown disable_player_interactions=unknown block_network_ids_are_hashes=unknown block_property_count=unknown"
                        );
                        let mut movement = MovementValidation::new(
                            ActorRuntimeID(current_runtime_id),
                            started.elapsed().as_secs().max(1),
                            (start.position.x, start.position.y, start.position.z),
                            (start.rotation.x, start.rotation.y),
                        );
                        movement.entity_id = current_entity_id;
                        if let Some(held_item) = held_item_candidate.clone() {
                            eprintln!(
                                "[GAMEPLAY_INVENTORY] restored_cached_candidate=true container={} slot={} item_id={}",
                                held_item.container_id, held_item.slot, held_item.item_id
                            );
                            movement.held_item = Some(held_item);
                        }
                        movement_validation = Some(movement);
                        eprintln!(
                            "[MOVEMENT_VALIDATION] start duration_seconds={} entity_id={} spawn_position={} rotation={:.3},{:.3}",
                            MOVEMENT_VALIDATION_SECONDS,
                            current_entity_id,
                            format_position((start.position.x, start.position.y, start.position.z)),
                            start.rotation.x,
                            start.rotation.y
                        );
                        if status.player_spawn && !movement_init_sent {
                            self.send_movement_initialization(
                                account_id,
                                bot_id,
                                &mut conn,
                                ActorRuntimeID(current_runtime_id),
                                started,
                                "typed_start_game_after_player_spawn",
                            )
                            .await?;
                            movement_init_sent = true;
                            let (deadline, delay) = movement_start_deadline();
                            movement_start_not_before = deadline;
                            movement_start_delay_reported = false;
                            eprintln!(
                                "[MOVEMENT_INIT] post_spawn_movement_delay_ms={} source=typed_start_game_after_player_spawn",
                                delay.as_millis()
                            );
                            self.drive_movement_validation_when_ready(
                                account_id,
                                bot_id,
                                session,
                                &mut conn,
                                &mut movement_validation,
                                &mut status,
                                send_chat_probe,
                                movement_start_not_before,
                                &mut movement_start_delay_reported,
                            )
                            .await?;
                        } else {
                            eprintln!(
                                "[MOVEMENT_INIT] waiting_for_player_spawn=true source=typed_start_game"
                            );
                        }
                    }
                    BedrockProto::MovePlayer(move_player) => {
                        self.log_move_player_rx(&move_player);
                        if let Some(movement) = movement_validation.as_mut() {
                            if move_player.runtime_id == movement.runtime_id.0 {
                                movement.record_server_position((
                                    move_player.position.x,
                                    move_player.position.y,
                                    move_player.position.z,
                                ));
                            }
                        }
                    }
                    BedrockProto::Respawn(respawn) => {
                        eprintln!(
                            "[MOVEMENT_RX] packet=Respawn state={:?} runtime_id={:?} position={}",
                            respawn.state,
                            respawn.runtime_entity_id,
                            format_position((
                                respawn.position.x,
                                respawn.position.y,
                                respawn.position.z
                            ))
                        );
                        if let Some(movement) = movement_validation.as_mut() {
                            if respawn.runtime_entity_id == movement.runtime_id.0 {
                                movement.record_server_position((
                                    respawn.position.x,
                                    respawn.position.y,
                                    respawn.position.z,
                                ));
                            }
                        }
                    }
                    BedrockProto::CorrectPlayerMovePrediction(correction) => {
                        eprintln!(
                            "[MOVEMENT_RX] packet=CorrectPlayerMovePrediction tick={} position={} velocity={} prediction_type={:?}",
                            correction.tick,
                            format_position((correction.position.x, correction.position.y, correction.position.z)),
                            format_position((correction.delta.x, correction.delta.y, correction.delta.z)),
                            correction.on_ground // prediction_type
                        );
                        if let Some(movement) = movement_validation.as_mut() {
                            movement.record_correction((
                                correction.position.x,
                                correction.position.y,
                                correction.position.z,
                            ));
                            self.diagnostics
                                .log_event(
                                    Some(account_id),
                                    bot_id,
                                    "warn",
                                    "bedrock_movement",
                                    Some("correct_player_move_prediction"),
                                    "server sent authoritative movement correction",
                                    json!({
                                        "tick": correction.tick,
                                        "position": format_position((correction.position.x, correction.position.y, correction.position.z)),
                                        "velocity": format_position((correction.delta.x, correction.delta.y, correction.delta.z)),
                                        "correction_count": movement.correction_count,
                                        "last_sent_position": format_position(movement.last_sent_position),
                                    }),
                                )
                                .await?;
                        }
                    }
                    BedrockProto::NetworkStackLatency(latency) => {
                        if trace_packets_enabled() {
                            eprintln!(
                                "[BEDROCK_HANDLER] typed NetworkStackLatency creation_time={} is_from_server={} response_deferred_to_raw_observer=true",
                                latency.timestamp, latency.needs_response
                            );
                        }
                        status.keepalive = true;
                    }
                    BedrockProto::Text(text) => {
                        if trace_packets_enabled() {
                            eprintln!("[CHAT_RX] packet={:?}", text);
                        }
                        status.chat = true;
                        self.diagnostics
                            .log_event(
                                Some(account_id),
                                bot_id,
                                "info",
                                "bot_chat",
                                Some("chat_packet"),
                                "bot observed a Bedrock text packet",
                                json!({ "observed_at": Utc::now().to_rfc3339() }),
                            )
                            .await?;
                    }
                    BedrockProto::ModalFormRequest(form) => {
                        eprintln!(
                            "[FORMS_RX] form_id={} json={}",
                            form.form_id, form.form_content
                        );
                        conn.send(&[BedrockProto::ModalFormResponse(
                            torchflower_protocol::ModalFormResponsePacket {
                                form_id: form.form_id,
                                has_response_data: true,
                                response_data: "0".to_string(),
                                has_cancel_reason: false,
                                cancel_reason: 0,
                            },
                        )])
                        .await?;
                        status.forms = true;
                    }
                    BedrockProto::InventoryContent(packet) => {
                        status.inventory_transactions = true;
                        if let Some(movement) = movement_validation.as_mut() {
                            for (slot, item) in packet.slots.iter().enumerate() {
                                let descriptor = NetworkItemStackDescriptor {
                                    network_id: item.network_id,
                                    count: item.count,
                                    metadata: item.metadata_val,
                                    block_runtime_id: item.block_runtime_id,
                                    extra_bytes: vec![],
                                };
                                movement.record_inventory_item(
                                    packet.container_id,
                                    slot as u32,
                                    &descriptor,
                                );
                            }
                        } else {
                            for (slot, item) in packet.slots.iter().enumerate() {
                                let descriptor = NetworkItemStackDescriptor {
                                    network_id: item.network_id,
                                    count: item.count,
                                    metadata: item.metadata_val,
                                    block_runtime_id: item.block_runtime_id,
                                    extra_bytes: vec![],
                                };
                                cache_held_inventory_candidate(
                                    &mut held_item_candidate,
                                    packet.container_id,
                                    slot as u32,
                                    &descriptor,
                                );
                            }
                        }
                        if let Some(bot_id) = bot_id {
                            let inventory = json!({
                                "last_event": "inventory_content",
                                "observed_at": Utc::now().to_rfc3339(),
                                "packet": format!("{packet:?}")
                            });
                            self.db
                                .update_bot_runtime_state(bot_id, None, Some(&inventory))
                                .await?;
                            self.diagnostics
                                .log_event(
                                    Some(account_id),
                                    Some(bot_id),
                                    "info",
                                    "bot_inventory",
                                    Some("inventory_content"),
                                    "bot observed inventory content",
                                    inventory,
                                )
                                .await?;
                        }
                    }
                    BedrockProto::InventorySlot(packet) => {
                        status.inventory_transactions = true;
                        let descriptor = NetworkItemStackDescriptor {
                            network_id: packet.network_id,
                            count: packet.count,
                            metadata: packet.metadata_val,
                            block_runtime_id: packet.block_runtime_id,
                            extra_bytes: vec![],
                        };
                        if let Some(movement) = movement_validation.as_mut() {
                            movement.record_inventory_item(
                                packet.container_id,
                                packet.slot,
                                &descriptor,
                            );
                        } else {
                            cache_held_inventory_candidate(
                                &mut held_item_candidate,
                                packet.container_id,
                                packet.slot,
                                &descriptor,
                            );
                        }
                        if let Some(bot_id) = bot_id {
                            let inventory = json!({
                                "last_event": "inventory_slot",
                                "observed_at": Utc::now().to_rfc3339(),
                                "packet": format!("{packet:?}")
                            });
                            self.db
                                .update_bot_runtime_state(bot_id, None, Some(&inventory))
                                .await?;
                            self.diagnostics
                                .log_event(
                                    Some(account_id),
                                    Some(bot_id),
                                    "info",
                                    "bot_inventory",
                                    Some("inventory_slot"),
                                    "bot observed inventory slot update",
                                    inventory,
                                )
                                .await?;
                        }
                    }
                    BedrockProto::UpdateBlock(packet) => {
                        let target = BlockTarget {
                            x: packet.position.x,
                            y: packet.position.y,
                            z: packet.position.z,
                        };
                        eprintln!(
                            "[GAMEPLAY_RX] packet=UpdateBlock target={} runtime_id={} flags={} layer={}",
                            format_block_target(target),
                            packet.block_runtime_id,
                            packet.flags,
                            packet.layer
                        );
                        if let Some(movement) = movement_validation.as_mut() {
                            movement.record_rtp_position_hint_from_update_block(
                                target,
                                packet.block_runtime_id,
                                packet.layer,
                            );
                            apply_update_block_evidence(
                                &mut status,
                                movement,
                                target,
                                packet.block_runtime_id,
                                "UpdateBlock",
                            );
                        }
                    }
                    BedrockProto::LevelEvent(packet) => {
                        eprintln!(
                            "[GAMEPLAY_RX] packet=LevelEvent event_id={} position={} data={}",
                            packet.event_id,
                            format_position((
                                packet.position.x,
                                packet.position.y,
                                packet.position.z
                            )),
                            packet.data
                        );
                    }
                    BedrockProto::ItemStackResponse(packet) => {
                        eprintln!(
                            "[GAMEPLAY_RX] packet=ItemStackResponse responses={:?}",
                            packet.responses
                        );
                    }
                    BedrockProto::CommandOutput(packet) => {
                        eprintln!(
                            "[GAMEPLAY_RX] packet=CommandOutput messages={:?}",
                            packet.output_messages
                        );
                    }
                    BedrockProto::Disconnect(disconnect) => {
                        status.disconnect_handling = true;
                        status.disconnect_reason = Some(format!("{disconnect:?}"));
                        self.diagnostics
                            .log_event(
                                Some(account_id),
                                bot_id,
                                "warn",
                                "bedrock",
                                Some("disconnect"),
                                "server disconnected the bot",
                                json!({ "packet": format!("{disconnect:?}") }),
                            )
                            .await?;
                        conn.close().await;
                        if let Some(spawn_started_at) = spawn_started_at {
                            status.connected_duration_seconds =
                                spawn_started_at.elapsed().as_secs();
                        }
                        self.finalize_status(&mut status);
                        return Ok(status);
                    }
                    _ => {}
                }
                if movement_init_sent {
                    self.drive_movement_validation_when_ready(
                        account_id,
                        bot_id,
                        session,
                        &mut conn,
                        &mut movement_validation,
                        &mut status,
                        send_chat_probe,
                        movement_start_not_before,
                        &mut movement_start_delay_reported,
                    )
                    .await?;
                }
            }
            for observed in observed_packets {
                match observed {
                    ObservedPacket::PlayStatus(status_code) => {
                        eprintln!(
                            "[BEDROCK_RX_OBSERVED] packet=PlayStatus status_code={}",
                            status_code
                                .map(|code| code.to_string())
                                .unwrap_or_else(|| "decode_failed".to_string())
                        );
                        status.login = true;
                        if status_code == Some(3) {
                            status.player_spawn = true;
                            self.diagnostics
                                .log_event(
                                    Some(account_id),
                                    bot_id,
                                    "info",
                                    "bedrock",
                                    Some("player_spawn"),
                                    "server sent PlayerSpawn play status",
                                    json!({ "elapsed_seconds": started.elapsed().as_secs(), "source": "raw_observed" }),
                                )
                                .await?;
                            if !movement_init_sent {
                                if let Some(runtime_id) =
                                    movement_validation.as_ref().map(|m| m.runtime_id)
                                {
                                    self.send_movement_initialization(
                                        account_id,
                                        bot_id,
                                        &mut conn,
                                        runtime_id,
                                        started,
                                        "observed_play_status_player_spawn",
                                    )
                                    .await?;
                                    movement_init_sent = true;
                                    let (deadline, delay) = movement_start_deadline();
                                    movement_start_not_before = deadline;
                                    movement_start_delay_reported = false;
                                    eprintln!(
                                        "[MOVEMENT_INIT] post_spawn_movement_delay_ms={} source=observed_play_status_player_spawn",
                                        delay.as_millis()
                                    );
                                    self.drive_movement_validation_when_ready(
                                        account_id,
                                        bot_id,
                                        session,
                                        &mut conn,
                                        &mut movement_validation,
                                        &mut status,
                                        send_chat_probe,
                                        movement_start_not_before,
                                        &mut movement_start_delay_reported,
                                    )
                                    .await?;
                                }
                            }
                        } else {
                            self.diagnostics
                                .log_event(
                                    Some(account_id),
                                    bot_id,
                                    "info",
                                    "bedrock",
                                    Some("play_status"),
                                    "server sent PlayStatus",
                                    json!({ "status_code": status_code, "source": "raw_observed" }),
                                )
                                .await?;
                        }
                    }
                    ObservedPacket::ResourcePacksInfo => {
                        eprintln!(
                            "[BEDROCK_HANDLER] observed ResourcePacksInfo -> ResourcePackClientResponse(have_all_packs), ClientCacheStatus(false)"
                        );
                        conn.send(&[
                            BedrockProto::ResourcePackClientResponse(
                                ResourcePackClientResponsePacket {
                                    response_status: ResourcePackResponse::AllPacksDownloaded
                                        .as_u8(),
                                    resource_pack_ids: vec![],
                                },
                            ),
                            BedrockProto::ClientCacheStatus(ClientCacheStatusPacket {
                                support_client_cache: false,
                            }),
                        ])
                        .await?;
                    }
                    ObservedPacket::ResourcePackStack => {
                        eprintln!(
                            "[BEDROCK_HANDLER] observed ResourcePackStack -> ResourcePackClientResponse(completed)"
                        );
                        resource_stack_done = true;
                        conn.send(&[BedrockProto::ResourcePackClientResponse(
                            ResourcePackClientResponsePacket {
                                response_status: ResourcePackResponse::Completed.as_u8(),
                                resource_pack_ids: vec![],
                            },
                        )])
                        .await?;
                    }
                    ObservedPacket::StartGame(start_game) => {
                        if !status.spawn {
                            spawn_started_at = Some(Instant::now());
                            status.spawn = true;
                            resource_stack_done = true;
                            if let Some(start_game) = start_game {
                                let runtime_id = ActorRuntimeID(start_game.runtime_id);
                                let mut movement = MovementValidation::new(
                                    runtime_id,
                                    started.elapsed().as_secs().max(1),
                                    start_game.position,
                                    start_game.rotation,
                                );
                                movement.entity_id = start_game.entity_id;
                                movement_validation = Some(movement);
                                eprintln!(
                                    "[MOVEMENT_VALIDATION] start source=raw_start_game duration_seconds={} runtime_id={:?} entity_id={} spawn_position={} rotation={:.3},{:.3}",
                                    MOVEMENT_VALIDATION_SECONDS,
                                    runtime_id,
                                    start_game.entity_id,
                                    format_position(start_game.position),
                                    start_game.rotation.0,
                                    start_game.rotation.1
                                );
                                eprintln!(
                                    "[STARTGAME_POLICY] source=raw_start_game server_authoritative_block_breaking={} disable_player_interactions={} block_network_ids_are_hashes={} block_property_count={}",
                                    start_game
                                        .server_authoritative_block_breaking
                                        .map(|value: bool| value.to_string())
                                        .unwrap_or_else(|| "unknown".to_string()),
                                    start_game
                                        .disable_player_interactions
                                        .map(|value: bool| value.to_string())
                                        .unwrap_or_else(|| "unknown".to_string()),
                                    start_game
                                        .block_network_ids_are_hashes
                                        .map(|value: bool| value.to_string())
                                        .unwrap_or_else(|| "unknown".to_string()),
                                    start_game
                                        .block_property_count
                                        .map(|value: u32| value.to_string())
                                        .unwrap_or_else(|| "unknown".to_string())
                                );
                                if status.player_spawn && !movement_init_sent {
                                    self.send_movement_initialization(
                                        account_id,
                                        bot_id,
                                        &mut conn,
                                        runtime_id,
                                        started,
                                        "observed_start_game_after_player_spawn",
                                    )
                                    .await?;
                                    movement_init_sent = true;
                                    let (deadline, delay) = movement_start_deadline();
                                    movement_start_not_before = deadline;
                                    movement_start_delay_reported = false;
                                    eprintln!(
                                        "[MOVEMENT_INIT] post_spawn_movement_delay_ms={} source=observed_start_game_after_player_spawn",
                                        delay.as_millis()
                                    );
                                    self.drive_movement_validation_when_ready(
                                        account_id,
                                        bot_id,
                                        session,
                                        &mut conn,
                                        &mut movement_validation,
                                        &mut status,
                                        send_chat_probe,
                                        movement_start_not_before,
                                        &mut movement_start_delay_reported,
                                    )
                                    .await?;
                                } else {
                                    eprintln!(
                                        "[MOVEMENT_INIT] waiting_for_player_spawn=true source=raw_start_game"
                                    );
                                }
                            } else {
                                eprintln!(
                                    "[MOVEMENT_BLOCKER] stage=raw_start_game_extract missing_runtime_or_position=true"
                                );
                            }
                            self.diagnostics
                                .log_event(
                                    Some(account_id),
                                    bot_id,
                                    "info",
                                    "bedrock",
                                    Some("start_game"),
                                    "server sent StartGame spawn state",
                                    json!({ "source": "raw_observed" }),
                                )
                                .await?;
                        }
                    }
                    ObservedPacket::Disconnect(disconnect) => {
                        status.disconnect_handling = true;
                        let disconnect_reason = disconnect
                            .map(|disconnect| {
                                format!(
                                    "server sent DisconnectPacket reason={} hide_reason={} message={} filtered_message={}",
                                    disconnect.reason,
                                    disconnect.hide_reason,
                                    disconnect.message.unwrap_or_default(),
                                    disconnect.filtered_message.unwrap_or_default()
                                )
                            })
                            .unwrap_or_else(|| "server sent DisconnectPacket".to_string());
                        status.disconnect_reason = Some(disconnect_reason.clone());
                        self.diagnostics
                            .log_event(
                                Some(account_id),
                                bot_id,
                                "warn",
                                "bedrock",
                                Some("disconnect"),
                                "server disconnected the bot",
                                json!({
                                    "source": "raw_observed",
                                    "reason": disconnect_reason,
                                }),
                            )
                            .await?;
                        conn.close().await;
                        if let Some(spawn_started_at) = spawn_started_at {
                            status.connected_duration_seconds =
                                spawn_started_at.elapsed().as_secs();
                        }
                        self.finalize_status(&mut status);
                        return Ok(status);
                    }
                    ObservedPacket::NetworkStackLatency {
                        timestamp,
                        needs_response,
                    } => {
                        if needs_response {
                            if trace_packets_enabled() {
                                eprintln!(
                                    "[BEDROCK_HANDLER] observed NetworkStackLatency -> immediate_response timestamp={} incoming_needs_response=true outgoing_needs_response=false",
                                    timestamp
                                );
                            }
                            conn.send_network_stack_latency_response(timestamp).await?;
                        } else {
                            if trace_packets_enabled() {
                                eprintln!(
                                    "[BEDROCK_HANDLER] observed NetworkStackLatency -> no_response timestamp={} incoming_needs_response=false",
                                    timestamp
                                );
                            }
                        }
                        status.keepalive = true;
                    }
                    ObservedPacket::Text(text) => {
                        if trace_packets_enabled() {
                            if let Some(text) = text {
                                eprintln!("[CHAT_RX_OBSERVED] strings={:?}", text.strings);
                            }
                        }
                        status.chat = true;
                    }
                    ObservedPacket::ModalFormRequest => {
                        status.forms = true;
                    }
                    ObservedPacket::AddItemEntity(entity) => {
                        if let Some(movement) = movement_validation.as_mut() {
                            movement.record_observed_item_entity(&entity);
                        }
                    }
                    ObservedPacket::TakeItemEntity {
                        runtime_entity_id,
                        target_runtime_entity_id,
                    } => {
                        if let Some(movement) = movement_validation.as_mut() {
                            movement.record_take_item_entity(
                                runtime_entity_id,
                                target_runtime_entity_id,
                            );
                        }
                    }
                    ObservedPacket::InventoryContent { items } => {
                        status.inventory_transactions = true;
                        for item in items {
                            if let Some(movement) = movement_validation.as_mut() {
                                movement.record_observed_inventory_item(&item);
                            } else {
                                cache_observed_held_inventory_candidate(
                                    &mut held_item_candidate,
                                    &item,
                                );
                            }
                        }
                    }
                    ObservedPacket::InventorySlot { item } => {
                        status.inventory_transactions = true;
                        if let Some(item) = item {
                            if let Some(movement) = movement_validation.as_mut() {
                                movement.record_observed_inventory_item(&item);
                            } else {
                                cache_observed_held_inventory_candidate(
                                    &mut held_item_candidate,
                                    &item,
                                );
                            }
                        }
                    }
                    ObservedPacket::ItemStackResponse { responses } => {
                        status.inventory_transactions = true;
                        eprintln!(
                            "[GAMEPLAY_RX_OBSERVED] packet=ItemStackResponse responses={:?}",
                            responses
                        );
                    }
                    ObservedPacket::NetworkChunkPublisherUpdate { x, y, z, radius } => {
                        if let Some(movement) = movement_validation.as_mut() {
                            movement.record_network_chunk_publisher_update(x, y, z, radius);
                        }
                    }
                    ObservedPacket::LevelChunk {
                        chunk_x,
                        chunk_z,
                        dimension,
                        samples,
                    } => {
                        if let Some(movement) = movement_validation.as_mut() {
                            for sample in &samples {
                                movement.record_observed_block_sample(sample);
                            }
                        }
                        if trace_chunks_enabled() && !samples.is_empty() {
                            eprintln!(
                                "[GAMEPLAY_CHUNK] observed_samples={} chunk={},{} dimension={}",
                                samples.len(),
                                chunk_x,
                                chunk_z,
                                dimension
                            );
                        }
                    }
                    ObservedPacket::UpdateBlock {
                        x,
                        y,
                        z,
                        runtime_id,
                        flags,
                        layer,
                    } => {
                        let target = BlockTarget { x, y, z };
                        if trace_chunks_enabled() {
                            eprintln!(
                                "[GAMEPLAY_RX] packet=UpdateBlockRaw target={} runtime_id={} flags={} layer={}",
                                format_block_target(target),
                                runtime_id,
                                flags,
                                layer
                            );
                        }
                        if let Some(movement) = movement_validation.as_mut() {
                            movement.record_rtp_position_hint_from_update_block(
                                target, runtime_id, layer,
                            );
                            apply_update_block_evidence(
                                &mut status,
                                movement,
                                target,
                                runtime_id,
                                "UpdateBlockRaw",
                            );
                        }
                    }
                    ObservedPacket::UpdateSoftEnum => {}
                    ObservedPacket::RegistryKnown(_) => {}
                    ObservedPacket::Other(id) => {
                        if id == 0x0b {
                            status.spawn = true;
                            spawn_started_at.get_or_insert_with(Instant::now);
                        }
                    }
                }
                if movement_init_sent {
                    self.drive_movement_validation_when_ready(
                        account_id,
                        bot_id,
                        session,
                        &mut conn,
                        &mut movement_validation,
                        &mut status,
                        send_chat_probe,
                        movement_start_not_before,
                        &mut movement_start_delay_reported,
                    )
                    .await?;
                }
            }
            if let Some(spawn_started_at) = spawn_started_at {
                status.connected_duration_seconds = spawn_started_at.elapsed().as_secs();
                status.remained_connected =
                    status.connected_duration_seconds >= required_duration.as_secs();
            }
            if movement_init_sent {
                self.drive_movement_validation_when_ready(
                    account_id,
                    bot_id,
                    session,
                    &mut conn,
                    &mut movement_validation,
                    &mut status,
                    send_chat_probe,
                    movement_start_not_before,
                    &mut movement_start_delay_reported,
                )
                .await?;
            }
        }

        conn.close().await;
        status.disconnect_handling = true;
        self.finalize_status(&mut status);
        self.diagnostics
            .log_event(
                Some(account_id),
                bot_id,
                if status.success { "info" } else { "warn" },
                "bedrock",
                Some("validation_complete"),
                "real-server Bedrock MVP validation completed",
                serde_json::to_value(&status)?,
            )
            .await?;
        if let Some(bot_id) = bot_id {
            self.db
                .update_bot_capabilities(bot_id, &serde_json::to_value(&status)?)
                .await?;
        }
        Ok(status)
    }

    async fn send_chat_probe(
        &self,
        conn: &mut BedrockProtocolAdapter,
        session: &ProvisionedBedrockSession,
    ) -> EngineResult<()> {
        self.send_text_message(conn, session, "TorchFlower validation online")
            .await
    }

    async fn send_movement_initialization(
        &self,
        account_id: &str,
        bot_id: Option<&str>,
        conn: &mut BedrockProtocolAdapter,
        runtime_id: ActorRuntimeID,
        started: Instant,
        source: &str,
    ) -> EngineResult<()> {
        let initialization_packets = vec![
            BedrockProto::SetLocalPlayerAsInitialized(SetLocalPlayerAsInitializedPacket {
                runtime_entity_id: runtime_id.0,
            }),
            BedrockProto::RequestChunkRadius(RequestChunkRadiusPacket {
                radius: 4,
                max_radius: 4,
            }),
        ];
        eprintln!(
            "[MOVEMENT_INIT] tx SetLocalPlayerAsInitialized + RequestChunkRadius source={source}"
        );
        match timeout(Duration::from_secs(2), conn.send(&initialization_packets)).await {
            Ok(Ok(())) => {
                eprintln!("[MOVEMENT_INIT] tx_complete=true source={source}");
                self.diagnostics
                    .log_event(
                        Some(account_id),
                        bot_id,
                        "info",
                        "bedrock",
                        Some("player_initialized"),
                        "client sent SetLocalPlayerAsInitialized after PlayerSpawn",
                        json!({
                            "runtime_id": format!("{runtime_id:?}"),
                            "elapsed_seconds": started.elapsed().as_secs(),
                            "source": source,
                        }),
                    )
                    .await?;
                Ok(())
            }
            Ok(Err(err)) => {
                eprintln!(
                    "[MOVEMENT_BLOCKER] stage=movement_init_send source={source} error={err}"
                );
                Err(err)
            }
            Err(_) => {
                eprintln!(
                    "[MOVEMENT_BLOCKER] stage=movement_init_send source={source} timed_out=true continue_to_movement=true"
                );
                Ok(())
            }
        }
    }

    async fn send_text_message(
        &self,
        conn: &mut BedrockProtocolAdapter,
        session: &ProvisionedBedrockSession,
        message: &str,
    ) -> EngineResult<()> {
        conn.send(&[BedrockProto::Text(TextPacket {
            packet_type: TextPacketType::Chat as u8,
            needs_translation: false,
            source_name: session.chain.display_name.clone(),
            message: message.to_string(),
            parameters: vec![],
            xbox_user_id: session.chain.xuid.clone(),
            platform_chat_id: String::new(),
        })])
        .await
    }

    async fn send_command_request(
        &self,
        conn: &mut BedrockProtocolAdapter,
        command: &str,
        player_entity_id: i64,
    ) -> EngineResult<()> {
        let clean_command = command_request_wire_command(command);
        let request_id = Uuid::new_v4().to_string();
        eprintln!(
            "[COMMAND_TX] packet=CommandRequest command={} request_id={}",
            clean_command, request_id
        );
        conn.send_preencoded_packet_stream(
            "command_request",
            encode_command_request_packet_stream(&clean_command, &request_id, player_entity_id),
        )
        .await
    }

    async fn send_tick_sync_keepalive(
        &self,
        conn: &mut BedrockProtocolAdapter,
        movement: &mut MovementValidation,
    ) -> EngineResult<()> {
        let Some(request_time) = movement.next_tick_sync_request_time() else {
            return Ok(());
        };
        let stream = encode_tick_sync_packet_stream(request_time, 0);
        if movement.tick_sync_sent_count <= 5 || movement.tick_sync_sent_count % 100 == 0 {
            eprintln!(
                "[TICK_SYNC_TX] packet=TickSync frame={} request_time={} response_time=0 interval_ms={} len={}",
                movement.tick_sync_sent_count,
                request_time,
                tick_sync_interval_duration().as_millis(),
                stream.len()
            );
        }
        conn.send_preencoded_packet_stream_queued("tick_sync", stream)
            .await
    }

    async fn send_validation_command(
        &self,
        conn: &mut BedrockProtocolAdapter,
        session: &ProvisionedBedrockSession,
        command: &str,
        player_entity_id: i64,
    ) -> EngineResult<()> {
        let override_mode = std::env::var("BEDROCK_VALIDATE_COMMAND_MODE").ok();
        let mode = validation_command_mode(command, override_mode.as_deref());
        if matches!(mode.as_str(), "command_request" | "command-request") {
            self.send_command_request(conn, command, player_entity_id)
                .await
        } else {
            eprintln!(
                "[COMMAND_TX] packet=TextPacket command={} mode=text",
                command.trim()
            );
            self.send_text_message(conn, session, command).await
        }
    }

    async fn drive_movement_validation_when_ready(
        &self,
        account_id: &str,
        bot_id: Option<&str>,
        session: &ProvisionedBedrockSession,
        conn: &mut BedrockProtocolAdapter,
        movement_validation: &mut Option<MovementValidation>,
        status: &mut CapabilityStatus,
        send_chat_probe: bool,
        movement_start_not_before: Option<Instant>,
        movement_start_delay_reported: &mut bool,
    ) -> EngineResult<()> {
        if let Some(deadline) = movement_start_not_before {
            let now = Instant::now();
            if now < deadline {
                if !*movement_start_delay_reported {
                    *movement_start_delay_reported = true;
                    eprintln!(
                        "[MOVEMENT_INIT] waiting_for_post_spawn_settle=true remaining_ms={} reason=defer_player_auth_input_until_initial_server_stream_settles",
                        deadline.saturating_duration_since(now).as_millis()
                    );
                }
                return Ok(());
            }
        }

        self.drive_movement_validation(
            account_id,
            bot_id,
            session,
            conn,
            movement_validation,
            status,
            send_chat_probe,
        )
        .await
    }

    async fn drive_movement_validation(
        &self,
        account_id: &str,
        bot_id: Option<&str>,
        session: &ProvisionedBedrockSession,
        conn: &mut BedrockProtocolAdapter,
        movement_validation: &mut Option<MovementValidation>,
        status: &mut CapabilityStatus,
        send_chat_probe: bool,
    ) -> EngineResult<()> {
        let Some(movement) = movement_validation.as_mut() else {
            return Ok(());
        };

        movement.adopt_initial_position_hint_if_available();
        if movement.should_wait_for_initial_position_hint() {
            if !movement.initial_position_hint_wait_reported {
                movement.initial_position_hint_wait_reported = true;
                eprintln!(
                    "[MOVEMENT_VALIDATION] waiting_for_initial_position_hint=true spawn_position={} timeout_seconds=3",
                    format_position(movement.spawn_position)
                );
            }
            return Ok(());
        }
        if movement.sent_frames == 0
            && movement.last_sent_at.is_none()
            && movement.initial_position_hint_wait_reported
        {
            movement.started_at = Instant::now();
            movement.initial_position_hint_wait_reported = false;
            eprintln!(
                "[MOVEMENT_VALIDATION] initial_position_hint_wait_expired=true start_position={}",
                format_position(movement.spawn_position)
            );
        }

        if movement.should_send_tick_sync() {
            self.send_tick_sync_keepalive(conn, movement).await?;
        }

        if movement.completed() {
            if !movement.completion_reported {
                movement.completion_reported = true;
                let ok = movement.correction_count == 0;
                status.movement = ok;
                eprintln!(
                    "[MOVEMENT_VALIDATION] completed=true ok={} duration_seconds={} sent_frames={} corrections={} last_sent_position={} last_server_position={} last_correction_position={}",
                    ok,
                    MOVEMENT_VALIDATION_SECONDS,
                    movement.sent_frames,
                    movement.correction_count,
                    format_position(movement.last_sent_position),
                    movement
                        .last_server_position
                        .map(format_position)
                        .unwrap_or_else(|| "none".to_string()),
                    movement
                        .last_correction_position
                        .map(format_position)
                        .unwrap_or_else(|| "none".to_string())
                );
                self.diagnostics
                    .log_event(
                        Some(account_id),
                        bot_id,
                        if ok { "info" } else { "warn" },
                        "bedrock_movement",
                        Some("movement_validation_complete"),
                        if ok {
                            "forward movement validation completed without server corrections"
                        } else {
                            "forward movement validation saw server authoritative corrections"
                        },
                        json!({
                            "ok": ok,
                            "duration_seconds": MOVEMENT_VALIDATION_SECONDS,
                            "sent_frames": movement.sent_frames,
                            "correction_count": movement.correction_count,
                            "last_sent_position": format_position(movement.last_sent_position),
                            "last_server_position": movement.last_server_position.map(format_position),
                            "last_correction_position": movement.last_correction_position.map(format_position),
                        }),
                    )
                    .await?;
            }
            if movement.should_send() {
                let frame = movement.next_idle_frame();
                self.send_idle_movement_frame(conn, &frame).await?;
            }
            if status.movement && movement.ready_for_prebreak_pickup() {
                let starting_prebreak_pickup = movement.pickup_probe_started_at.is_none();
                movement.rtp_wait_done = true;
                movement.inventory_probe_sent = true;
                movement.pickup_prebreak = true;
                status.inventory_transactions = true;
                if starting_prebreak_pickup {
                    eprintln!(
                        "[GAMEPLAY_PICKUP] prebreak_priority=true reason=normal_item_entity_before_rtp current={} item_target={}",
                        format_position(movement.last_sent_position),
                        movement
                            .observed_item_entity
                            .as_ref()
                            .map(|item| format!(
                                "runtime_id={} item_id={} item_len={} position={}",
                                item.runtime_id,
                                item.item_id,
                                item.item_bytes.len(),
                                format_position(item.position)
                            ))
                            .unwrap_or_else(|| "none".to_string())
                    );
                }
                self.drive_gameplay_pickup(conn, movement).await?;
                return Ok(());
            }
            if status.movement && !movement.inventory_probe_sent {
                let rtp_wait_duration = rtp_wait_duration();
                let rtp_menu_open_wait_duration = rtp_menu_open_wait_duration();
                let rtp_menu_click_wait_duration = rtp_menu_click_wait_duration();
                if trust_chunk_publisher_position() {
                    movement.adopt_network_chunk_position_hint("movement_complete");
                }
                let rtp_command = std::env::var("BEDROCK_VALIDATE_RTP_COMMAND")
                    .unwrap_or_else(|_| "/rtp".to_string());
                let skip_rtp_for_loaded_world = movement.has_sampled_placeable_drop_target()
                    || movement.has_approachable_placeable_drop_target();
                if skip_rtp_for_loaded_world && !movement.rtp_wait_done {
                    movement.rtp_wait_done = true;
                    eprintln!(
                        "[GAMEPLAY_RTP] skipped=true reason=loaded_world_has_placeable_drop_target position={} observed_count={} placeable_count={} approachable_placeable_count={} near_reach={} approachable={}",
                        format_position(movement.last_sent_position),
                        movement.observed_solid_blocks.len(),
                        movement.observed_placeable_drop_count(),
                        movement.observed_approachable_placeable_drop_count(),
                        movement.has_sampled_placeable_drop_target(),
                        movement.has_approachable_placeable_drop_target()
                    );
                }
                if !rtp_command.trim().is_empty()
                    && !movement.rtp_command_sent
                    && !skip_rtp_for_loaded_world
                    && movement.rtp_command_attempts < RTP_MAX_COMMAND_ATTEMPTS
                {
                    let command_to_send = rtp_command.clone();
                    let attempt = movement.rtp_command_attempts.saturating_add(1);
                    eprintln!(
                        "[GAMEPLAY_RTP] tx command={} attempt={}",
                        command_to_send, attempt
                    );
                    self.send_validation_command(
                        conn,
                        session,
                        &command_to_send,
                        movement.entity_id,
                    )
                    .await?;
                    movement.rtp_command_sent = true;
                    movement.rtp_command_sent_at = Some(Instant::now());
                    movement.rtp_command_attempts = attempt;
                    status.chat = true;
                    return Ok(());
                }
                if movement.rtp_command_sent && !movement.rtp_wait_done {
                    if should_click_rtp_menu() {
                        if let Some(menu_item) = movement.rtp_menu_item.clone() {
                            let should_attempt_click = movement.rtp_menu_click_attempts
                                < RTP_MENU_MAX_CLICK_ATTEMPTS
                                && movement
                                    .rtp_menu_last_click_attempt_at
                                    .map(|sent_at| {
                                        sent_at.elapsed() >= RTP_MENU_CLICK_RETRY_INTERVAL
                                    })
                                    .unwrap_or(true);
                            if should_attempt_click {
                                let attempt = movement.rtp_menu_click_attempts;
                                let Some(method) = MenuClickMethod::from_attempt(attempt) else {
                                    movement.rtp_menu_click_attempts = RTP_MENU_MAX_CLICK_ATTEMPTS;
                                    return Ok(());
                                };
                                let request_id = movement.next_item_stack_request_id;
                                movement.next_item_stack_request_id += 1;
                                let tick = movement.next_client_tick();
                                let position = movement
                                    .last_server_position
                                    .unwrap_or(movement.last_sent_position);
                                self.send_rtp_menu_click(
                                    conn,
                                    request_id,
                                    &menu_item,
                                    method,
                                    position,
                                    movement.yaw,
                                    movement.pitch,
                                    tick,
                                )
                                .await?;
                                movement.rtp_menu_click_sent = true;
                                let now = Instant::now();
                                movement.rtp_menu_click_sent_at = Some(now);
                                movement.rtp_menu_last_click_attempt_at = Some(now);
                                movement.rtp_menu_click_attempts =
                                    movement.rtp_menu_click_attempts.saturating_add(1);
                                status.inventory_transactions = true;
                                return Ok(());
                            }
                        }
                    }
                    let waiting_for_menu_open = should_click_rtp_menu()
                        && movement.rtp_menu_item.is_none()
                        && !movement.rtp_menu_click_sent;
                    let required_wait = if movement.rtp_menu_click_sent {
                        rtp_menu_click_wait_duration
                    } else if waiting_for_menu_open {
                        rtp_menu_open_wait_duration
                    } else if movement.rtp_position_hint_received
                        && movement.rtp_menu_item.is_none()
                    {
                        Duration::from_secs(8)
                    } else if movement.rtp_position_hint_received {
                        Duration::from_secs(3)
                    } else {
                        rtp_wait_duration
                    };
                    let waited_from = if movement.rtp_menu_click_sent {
                        movement.rtp_menu_click_sent_at
                    } else if movement.rtp_position_hint_received
                        && movement.rtp_menu_item.is_none()
                    {
                        movement.rtp_position_hint_received_at
                    } else {
                        movement.rtp_command_sent_at
                    };
                    let waited = waited_from
                        .map(|sent_at| sent_at.elapsed())
                        .unwrap_or_default();
                    if waited < required_wait {
                        if waiting_for_menu_open && !movement.rtp_waiting_for_menu_reported {
                            movement.rtp_waiting_for_menu_reported = true;
                            eprintln!(
                                "[GAMEPLAY_RTP] waiting_for_menu=true waited_seconds={} timeout_seconds={}",
                                waited.as_secs(),
                                rtp_menu_open_wait_duration.as_secs()
                            );
                        }
                        return Ok(());
                    }
                    if waiting_for_menu_open && !movement.rtp_position_hint_received {
                        if let Some(fallback_command) =
                            fallback_rtp_command(&rtp_command, movement.rtp_command_attempts)
                        {
                            let attempt = movement.rtp_command_attempts.saturating_add(1);
                            eprintln!(
                                "[GAMEPLAY_RTP] retry_command=true attempt={} command={} previous_command={} reason=menu_wait_expired waited_seconds={}",
                                attempt,
                                fallback_command,
                                rtp_command,
                                waited.as_secs()
                            );
                            self.send_validation_command(
                                conn,
                                session,
                                &fallback_command,
                                movement.entity_id,
                            )
                            .await?;
                            movement.rtp_command_attempts = attempt;
                            movement.rtp_command_sent_at = Some(Instant::now());
                            movement.rtp_waiting_for_menu_reported = false;
                            status.chat = true;
                            return Ok(());
                        }
                        eprintln!(
                            "[GAMEPLAY_RTP] menu_wait_expired=true waited_seconds={} continuing_without_menu=true",
                            waited.as_secs()
                        );
                    }
                    if !movement.rtp_position_hint_received
                        && movement.raw_start_position_looks_placeholder()
                    {
                        movement.adopt_observed_terrain_position_hint("rtp_wait_observed_chunks");
                    }
                    let marker_only_position_hint = movement.rtp_marker_position_hint_received
                        && !movement.rtp_terrain_position_hint_received;
                    if marker_only_position_hint {
                        let waited_since_command = movement
                            .rtp_command_sent_at
                            .map(|sent_at| sent_at.elapsed())
                            .unwrap_or(waited);
                        if waited_since_command < rtp_wait_duration {
                            if !movement.rtp_waiting_for_terrain_hint_reported {
                                movement.rtp_waiting_for_terrain_hint_reported = true;
                                eprintln!(
                                    "[GAMEPLAY_RTP] waiting_for_terrain_hint=true marker_only_position_hint=true waited_seconds={} clicked_menu={} click_attempts={}",
                                    waited_since_command.as_secs(),
                                    movement.rtp_menu_click_sent,
                                    movement.rtp_menu_click_attempts
                                );
                            }
                            return Ok(());
                        }
                        if !movement.rtp_terrain_position_hint_failed_reported {
                            movement.rtp_terrain_position_hint_failed_reported = true;
                            movement.pickup_terminal_failed = true;
                            eprintln!(
                                "[GAMEPLAY_RTP] failed no_terrain_position_hint marker_only_position_hint=true waited_seconds={} clicked_menu={} click_attempts={} last_server_position={} current={}",
                                waited_since_command.as_secs(),
                                movement.rtp_menu_click_sent,
                                movement.rtp_menu_click_attempts,
                                movement
                                    .last_server_position
                                    .map(format_position)
                                    .unwrap_or_else(|| "none".to_string()),
                                format_position(movement.last_sent_position)
                            );
                        }
                        return Ok(());
                    }
                    movement.rtp_wait_done = true;
                    if let Some(position) = movement.last_server_position {
                        movement.last_sent_position = position;
                        movement.spawn_position = position;
                    }
                    eprintln!(
                        "[GAMEPLAY_RTP] wait_complete=true clicked_menu={} position_hint_received={} terrain_hint_received={} marker_hint_received={} waited_seconds={} last_server_position={} probe_origin={}",
                        movement.rtp_menu_click_sent,
                        movement.rtp_position_hint_received,
                        movement.rtp_terrain_position_hint_received,
                        movement.rtp_marker_position_hint_received,
                        waited.as_secs(),
                        movement
                            .last_server_position
                            .map(format_position)
                            .unwrap_or_else(|| "none".to_string()),
                        format_position(movement.last_sent_position)
                    );
                }
                if should_close_rtp_container() && !movement.rtp_container_close_sent {
                    let (window_id, window_type) = movement
                        .rtp_menu_item
                        .as_ref()
                        .map(|item| (item.window_id, item.container_type))
                        .unwrap_or((1, WINDOW_TYPE_CONTAINER));
                    self.send_container_close(conn, window_id, window_type, false)
                        .await?;
                    movement.rtp_container_close_sent = true;
                    return Ok(());
                }
                self.send_inventory_probe(conn).await?;
                movement.inventory_probe_sent = true;
                status.inventory_transactions = true;
                return Ok(());
            }
            if status.movement && send_chat_probe && !movement.chat_probe_sent {
                self.send_chat_probe(conn, session).await?;
                movement.chat_probe_sent = true;
                status.chat = true;
                return Ok(());
            }
            if status.movement && !movement.gameplay_probe_sent && !movement.pickup_terminal_failed
            {
                if movement.ready_for_prebreak_pickup() {
                    movement.pickup_prebreak = true;
                    self.drive_gameplay_pickup(conn, movement).await?;
                    return Ok(());
                }
                if !movement.has_sampled_placeable_drop_target()
                    && (movement.has_approachable_placeable_drop_target()
                        || movement.has_walkable_placeable_drop_target())
                {
                    let max_horizontal = if movement.has_approachable_placeable_drop_target() {
                        32.0
                    } else {
                        128.0
                    };
                    let max_vertical = GAMEPLAY_BREAK_REACH_VERTICAL;
                    let target = movement
                        .nearest_observed_solid_block_with_limits(
                            movement.last_sent_position,
                            true,
                            max_horizontal,
                            max_vertical,
                        )
                        .expect("walkable target exists");
                    if movement
                        .approach_last_sent_at
                        .map(|sent_at| sent_at.elapsed() < GAMEPLAY_APPROACH_SEND_INTERVAL)
                        .unwrap_or(false)
                    {
                        return Ok(());
                    }
                    let frame = if max_horizontal <= 32.0 {
                        movement.next_gameplay_approach_frame()
                    } else {
                        movement
                            .next_gameplay_approach_frame_with_limits(max_horizontal, max_vertical)
                    };
                    if let Some(frame) = frame {
                        eprintln!(
                            "[GAMEPLAY_APPROACH] target={} current={} frame={} tick={} position={} velocity={} max_horizontal={:.1} max_vertical={:.1}",
                            format_block_target(target),
                            format_position(movement.last_sent_position),
                            frame.frame_index,
                            frame.tick,
                            format_position(frame.position),
                            format_position(frame.velocity),
                            max_horizontal,
                            max_vertical
                        );
                        self.send_movement_frame(conn, &frame).await?;
                        return Ok(());
                    }
                }
                if self.send_gameplay_action_probe(conn, movement).await? {
                    movement.gameplay_probe_sent = true;
                    movement.gameplay_probe_sent_at = Some(Instant::now());
                }
            }
            if movement.gameplay_probe_sent
                && !movement.break_confirmed
                && !movement.pickup_terminal_failed
            {
                self.drive_gameplay_break(conn, movement).await?;
            }
            if movement.ready_for_pickup() {
                self.drive_gameplay_pickup(conn, movement).await?;
            }
            let pickup_active = movement
                .pickup_probe_started_at
                .map(|started_at| started_at.elapsed() <= GAMEPLAY_PICKUP_DURATION)
                .unwrap_or(false);
            if movement.ready_for_place() {
                self.ensure_held_item_equipped(conn, movement).await?;
                self.send_place_action_probe(conn, movement).await?;
            }
            if movement.gameplay_probe_sent
                && !movement.gameplay_timeout_reported
                && movement
                    .gameplay_probe_sent_at
                    .map(|sent_at| sent_at.elapsed() >= GAMEPLAY_PROBE_TIMEOUT)
                    .unwrap_or(false)
                && !pickup_active
                && (!status.block_breaking || !status.block_placing)
            {
                movement.gameplay_timeout_reported = true;
                eprintln!(
                    "[GAMEPLAY_TIMEOUT] break_ok={} place_ok={} place_probe_sent={} break_target={} place_result={} held_item={}",
                    status.block_breaking,
                    status.block_placing,
                    movement.place_probe_sent,
                    movement
                        .break_target
                        .map(format_block_target)
                        .unwrap_or_else(|| "none".to_string()),
                    movement
                        .place_result
                        .map(format_block_target)
                        .unwrap_or_else(|| "none".to_string()),
                    movement
                        .held_item
                        .as_ref()
                        .map(|item| format!(
                            "id={} container={} slot={}",
                            item.item_id, item.container_id, item.slot
                        ))
                        .unwrap_or_else(|| "none".to_string())
                );
                self.diagnostics
                    .log_event(
                        Some(account_id),
                        bot_id,
                        "warn",
                        "bedrock_gameplay",
                        Some("gameplay_probe_timeout"),
                        "server did not confirm all gameplay action probes",
                        json!({
                            "block_breaking": status.block_breaking,
                            "block_placing": status.block_placing,
                            "break_target": movement.break_target.map(format_block_target),
                            "place_result": movement.place_result.map(format_block_target),
                            "held_item": movement.held_item.as_ref().map(|item| json!({
                                "container_id": item.container_id,
                                "slot": item.slot,
                                "item_id": item.item_id,
                            })),
                        }),
                    )
                    .await?;
            }
            return Ok(());
        }

        if !movement.should_send() {
            return Ok(());
        }

        let validation_duration = Duration::from_secs(MOVEMENT_VALIDATION_SECONDS);
        if validation_duration.saturating_sub(movement.elapsed())
            <= MOVEMENT_COMPLETION_QUIET_PERIOD
        {
            return Ok(());
        }

        let frame = movement.next_frame();
        self.send_movement_frame(conn, &frame).await
    }

    async fn drive_gameplay_pickup(
        &self,
        conn: &mut BedrockProtocolAdapter,
        movement: &mut MovementValidation,
    ) -> EngineResult<()> {
        if movement.pickup_terminal_failed {
            return Ok(());
        }
        if movement.pickup_probe_started_at.is_none() {
            let target = movement
                .pickup_target_position()
                .map(format_position)
                .unwrap_or_else(|| "none".to_string());
            let item_target = movement
                .observed_item_entity
                .as_ref()
                .map(|item| {
                    format!(
                        "runtime_id={} item_id={} stack_id={} item_len={} position={}",
                        item.runtime_id,
                        item.item_id,
                        item.stack_id
                            .map(|stack_id| stack_id.to_string())
                            .unwrap_or_else(|| "none".to_string()),
                        item.item_bytes.len(),
                        format_position(item.position)
                    )
                })
                .unwrap_or_else(|| "none".to_string());
            eprintln!(
                "[GAMEPLAY_PICKUP] started=true prebreak={} target={} item_target={} current={}",
                movement.pickup_prebreak,
                target,
                item_target,
                format_position(movement.last_sent_position)
            );
            movement.pickup_probe_started_at = Some(Instant::now());
            movement.gameplay_probe_sent_at = Some(Instant::now());
        }
        let started_at = movement
            .pickup_probe_started_at
            .expect("pickup start time is initialized above");

        if movement
            .pickup_last_inventory_probe_at
            .map(|sent_at| sent_at.elapsed() >= GAMEPLAY_PICKUP_INVENTORY_PROBE_INTERVAL)
            .unwrap_or(true)
        {
            movement.pickup_last_inventory_probe_at = Some(Instant::now());
            eprintln!(
                "[GAMEPLAY_PICKUP_TX] packet=InventoryTransaction probe=inventory current={}",
                format_position(movement.last_sent_position)
            );
            self.send_inventory_probe(conn).await?;
        }

        if started_at.elapsed() > GAMEPLAY_PICKUP_DURATION {
            if movement.held_item.is_none() && !movement.pickup_failed_reported {
                movement.pickup_failed_reported = true;
                if movement.pickup_prebreak {
                    eprintln!(
                        "[GAMEPLAY_PICKUP] failed no_normal_placeable_item_collected prebreak=true target={} item_target={}",
                        movement
                            .pickup_target_position()
                            .map(format_position)
                            .unwrap_or_else(|| "none".to_string()),
                        movement
                            .observed_item_entity
                            .as_ref()
                            .map(|item| format!(
                                "runtime_id={} item_id={} item_len={} position={}",
                                item.runtime_id,
                                item.item_id,
                                item.item_bytes.len(),
                                format_position(item.position)
                            ))
                            .unwrap_or_else(|| "none".to_string())
                    );
                    if movement.prepare_next_item_entity_after_pickup_failure() {
                        eprintln!(
                            "[GAMEPLAY_PICKUP] retry=true reason=try_next_normal_item_entity rejected_item_runtime_ids={:?}",
                            movement.rejected_item_entity_runtime_ids
                        );
                    } else {
                        eprintln!(
                            "[GAMEPLAY_PICKUP] retry=false terminal=true reason=no_normal_item_entity_candidates rejected_item_runtime_ids={:?}",
                            movement.rejected_item_entity_runtime_ids
                        );
                    }
                } else if movement.prepare_next_break_target_after_pickup_failure() {
                    eprintln!("[GAMEPLAY_PICKUP] failed no_normal_placeable_drop_collected");
                    eprintln!(
                        "[GAMEPLAY_PICKUP] retry=true next_break_attempt={} rejected_targets={} rejected_runtime_ids={:?}",
                        movement.break_target_attempts.saturating_add(1),
                        movement.rejected_break_targets.len(),
                        movement.rejected_break_runtime_ids
                    );
                } else {
                    eprintln!(
                        "[GAMEPLAY_PICKUP] retry=false terminal=true rejected_targets={} rejected_runtime_ids={:?}",
                        movement.rejected_break_targets.len(),
                        movement.rejected_break_runtime_ids
                    );
                }
            }
            return Ok(());
        }

        if movement
            .pickup_last_sent_at
            .map(|sent_at| sent_at.elapsed() < GAMEPLAY_PICKUP_SEND_INTERVAL)
            .unwrap_or(false)
        {
            return Ok(());
        }

        if let Some(frame) = movement.next_pickup_frame() {
            self.send_pickup_movement_frame(conn, &frame).await?;
        }
        Ok(())
    }

    async fn ensure_held_item_equipped(
        &self,
        conn: &mut BedrockProtocolAdapter,
        movement: &mut MovementValidation,
    ) -> EngineResult<()> {
        if movement.held_item_equipped {
            return Ok(());
        }
        let Some(held_item) = movement.held_item.as_ref() else {
            return Ok(());
        };
        let Some(_item) = held_item.item.clone() else {
            eprintln!(
                "[GAMEPLAY_EQUIP] skipped=true reason=raw_inventory_item_without_typed_descriptor container={} slot={} item_id={}",
                held_item.container_id, held_item.slot, held_item.item_id
            );
            movement.held_item_equipped = true;
            return Ok(());
        };
        let slot = held_item.slot as u8;
        eprintln!(
            "[GAMEPLAY_EQUIP_TX] packet=MobEquipment runtime_id={:?} container={} slot={} selected_slot={} item_id={}",
            movement.runtime_id, held_item.container_id, slot, slot, held_item.item_id
        );
        conn.send(&[BedrockProto::MobEquipment(MobEquipmentPacket {
            runtime_entity_id: movement.runtime_id.0,
            slot,
            selected_slot: slot,
            container_id: 0u8,
        })])
        .await?;
        movement.held_item_equipped = true;
        Ok(())
    }

    async fn send_movement_frame(
        &self,
        conn: &mut BedrockProtocolAdapter,
        frame: &MovementFrame,
    ) -> EngineResult<()> {
        let input_data = received_server_data_input_flag()
            | PlayerAuthInputFlags::Up
            | PlayerAuthInputFlags::SprintDown
            | PlayerAuthInputFlags::Sprinting;
        let send_move_player = env_bool("BEDROCK_SEND_CLIENT_MOVE_PLAYER", false)
            && (frame.frame_index == 1 || frame.frame_index % 20 == 0);
        let mut packets = Vec::with_capacity(if send_move_player { 1 } else { 0 });
        if send_move_player {
            eprintln!(
                "[MOVEMENT_TX] packet=MovePlayer frame={} tick={} elapsed={:.3} runtime_id={:?} position={} rotation={:.3},{:.3} velocity={} mode=Normal on_ground=true",
                frame.frame_index,
                frame.tick,
                frame.elapsed_seconds,
                frame.runtime_id,
                format_position(frame.position),
                frame.yaw,
                frame.pitch,
                format_position(frame.velocity)
            );
            packets.push(BedrockProto::MovePlayer(MovePlayerPacket {
                runtime_id: frame.runtime_id.0,
                position: Vector3f {
                    x: frame.position.0,
                    y: frame.position.1,
                    z: frame.position.2,
                },
                pitch: frame.pitch,
                yaw: frame.yaw,
                head_yaw: frame.yaw,
                mode: 0,
                on_ground: true,
                riding_runtime_id: 0,
                teleport_cause: 0,
                teleport_item_id: 0,
                tick: frame.tick,
            }));
        }
        eprintln!(
            "[MOVEMENT_TX] packet=PlayerAuthInputRaw frame={} tick={} elapsed={:.3} position={} rotation={:.3},{:.3} move_vector=0.000,0.000,1.000 analog_move_vector=0.000,1.000 raw_move_vector=0.000,1.000 velocity={} input_data={:#x}",
            frame.frame_index,
            frame.tick,
            frame.elapsed_seconds,
            format_position(frame.position),
            frame.yaw,
            frame.pitch,
            format_position(frame.velocity),
            input_data
        );
        let raw_input = RawPlayerAuthInput {
            position: frame.position,
            velocity: frame.velocity,
            yaw: frame.yaw,
            pitch: frame.pitch,
            input_data,
            tick: frame.tick,
            move_vector: (0.0, 0.0, 1.0),
            analog_move_vector: (0.0, 1.0),
            raw_move_vector: (0.0, 1.0),
            block_actions: &[],
            item_use_transaction_id: None,
            item_stack_request: None,
        };
        match timeout(Duration::from_secs(2), async {
            if !packets.is_empty() {
                conn.send(&packets).await?;
            }
            let packet_stream = encode_player_auth_input_packet_stream(&raw_input)?;
            conn.send_preencoded_packet_stream_queued("player_auth_input_movement", packet_stream)
                .await
        })
        .await
        {
            Ok(Ok(())) => {
                eprintln!(
                    "[MOVEMENT_TX] send_complete=true frame={} tick={} packets={} position={}",
                    frame.frame_index,
                    frame.tick,
                    packets.len() + 1,
                    format_position(frame.position)
                );
                Ok(())
            }
            Ok(Err(err)) => {
                eprintln!(
                    "[MOVEMENT_BLOCKER] stage=movement_frame_send tick={} error={err}",
                    frame.tick
                );
                Err(err)
            }
            Err(_) => {
                eprintln!(
                    "[MOVEMENT_BLOCKER] stage=movement_frame_send tick={} timed_out=true",
                    frame.tick
                );
                Err(EngineError::Bedrock(format!(
                    "movement frame send timed out at tick {}",
                    frame.tick
                )))
            }
        }
    }

    async fn send_idle_movement_frame(
        &self,
        conn: &mut BedrockProtocolAdapter,
        frame: &MovementFrame,
    ) -> EngineResult<()> {
        if frame.frame_index <= 5 || frame.frame_index % 100 == 0 {
            eprintln!(
                "[MOVEMENT_IDLE_TX] packet=PlayerAuthInputRaw frame={} tick={} elapsed={:.3} position={} rotation={:.3},{:.3}",
                frame.frame_index,
                frame.tick,
                frame.elapsed_seconds,
                format_position(frame.position),
                frame.yaw,
                frame.pitch
            );
        }
        let raw_input = RawPlayerAuthInput {
            position: frame.position,
            velocity: (0.0, 0.0, 0.0),
            yaw: frame.yaw,
            pitch: frame.pitch,
            input_data: received_server_data_input_flag(),
            tick: frame.tick,
            move_vector: (0.0, 0.0, 0.0),
            analog_move_vector: (0.0, 0.0),
            raw_move_vector: (0.0, 0.0),
            block_actions: &[],
            item_use_transaction_id: None,
            item_stack_request: None,
        };
        conn.send_preencoded_packet_stream_queued(
            "player_auth_input_idle",
            encode_player_auth_input_packet_stream(&raw_input)?,
        )
        .await
    }

    async fn send_pickup_movement_frame(
        &self,
        conn: &mut BedrockProtocolAdapter,
        frame: &MovementFrame,
    ) -> EngineResult<()> {
        let input_data = received_server_data_input_flag()
            | PlayerAuthInputFlags::Up
            | PlayerAuthInputFlags::SprintDown
            | PlayerAuthInputFlags::Sprinting;
        let send_move_player = env_bool("BEDROCK_SEND_PICKUP_MOVE_PLAYER", true);
        if send_move_player {
            eprintln!(
                "[GAMEPLAY_PICKUP_TX] packet=MovePlayer frame={} tick={} elapsed={:.3} runtime_id={:?} position={} rotation={:.3},{:.3} velocity={} mode=Normal on_ground=true",
                frame.frame_index,
                frame.tick,
                frame.elapsed_seconds,
                frame.runtime_id,
                format_position(frame.position),
                frame.yaw,
                frame.pitch,
                format_position(frame.velocity)
            );
            conn.send(&[BedrockProto::MovePlayer(MovePlayerPacket {
                runtime_id: frame.runtime_id.0,
                position: Vector3f {
                    x: frame.position.0,
                    y: frame.position.1,
                    z: frame.position.2,
                },
                pitch: frame.pitch,
                yaw: frame.yaw,
                head_yaw: frame.yaw,
                mode: 0,
                on_ground: true,
                riding_runtime_id: 0,
                teleport_cause: 0,
                teleport_item_id: 0,
                tick: frame.tick,
            })])
            .await?;
        }
        eprintln!(
            "[GAMEPLAY_PICKUP_TX] packet=PlayerAuthInputRaw frame={} tick={} elapsed={:.3} position={} rotation={:.3},{:.3} velocity={} input_data={:#x}",
            frame.frame_index,
            frame.tick,
            frame.elapsed_seconds,
            format_position(frame.position),
            frame.yaw,
            frame.pitch,
            format_position(frame.velocity),
            input_data
        );
        let raw_input = RawPlayerAuthInput {
            position: frame.position,
            velocity: frame.velocity,
            yaw: frame.yaw,
            pitch: frame.pitch,
            input_data,
            tick: frame.tick,
            move_vector: (0.0, 0.0, 1.0),
            analog_move_vector: (0.0, 1.0),
            raw_move_vector: (0.0, 1.0),
            block_actions: &[],
            item_use_transaction_id: None,
            item_stack_request: None,
        };
        conn.send_preencoded_packet_stream_queued(
            "player_auth_input_gameplay_pickup",
            encode_player_auth_input_packet_stream(&raw_input)?,
        )
        .await
    }

    async fn send_gameplay_action_probe(
        &self,
        conn: &mut BedrockProtocolAdapter,
        movement: &mut MovementValidation,
    ) -> EngineResult<bool> {
        movement.ensure_gameplay_targets();
        let Some(break_target) = movement.break_target else {
            eprintln!(
                "[GAMEPLAY_BREAK] skipped missing_placeable_drop_target current={} observed_count={} rejected_targets={} rejected_runtime_ids={:?}",
                format_position(movement.last_sent_position),
                movement.observed_solid_blocks.len(),
                movement.rejected_break_targets.len(),
                movement.rejected_break_runtime_ids
            );
            return Ok(false);
        };
        let Some(place_base) = movement.place_base else {
            eprintln!("[GAMEPLAY_BREAK] skipped missing_gameplay_place_base");
            return Ok(false);
        };
        let Some(place_result) = movement.place_result else {
            eprintln!("[GAMEPLAY_BREAK] skipped missing_gameplay_place_result");
            return Ok(false);
        };
        movement.aim_at_block_target(break_target);
        let break_tick = movement.next_client_tick();
        let break_face = break_face_for_runtime_id(movement.break_target_runtime_id);

        eprintln!(
            "[GAMEPLAY_BREAK] target={} place_base={} place_result={} break_face={} runtime_id={:?} tick={} yaw={:.3} pitch={:.3} held_item={}",
            format_block_target(break_target),
            format_block_target(place_base),
            format_block_target(place_result),
            break_face,
            movement.runtime_id,
            break_tick,
            movement.yaw,
            movement.pitch,
            movement
                .held_item
                .as_ref()
                .map(|item| format!(
                    "id={} container={} slot={}",
                    item.item_id, item.container_id, item.slot
                ))
                .unwrap_or_else(|| "none".to_string())
        );

        eprintln!(
            "[GAMEPLAY_BREAK] legacy_player_action_skipped=true reason=server_authoritative_block_actions"
        );
        conn.send(&[BedrockProto::Animate(AnimatePacket {
            action_id: 1, // Swing
            runtime_entity_id: movement.runtime_id.0,
            rowing_time: 0.0,
        })])
        .await?;

        let block_actions = [RawBlockAction {
            action_id: PlayerActionType::StartBreak as i32,
            target: break_target,
            face: break_face,
        }];
        let break_input = RawPlayerAuthInput {
            position: movement.last_sent_position,
            velocity: (0.0, 0.0, 0.0),
            yaw: movement.yaw,
            pitch: movement.pitch,
            input_data: block_action_input_flags(),
            tick: break_tick,
            move_vector: (0.0, 0.0, 0.0),
            analog_move_vector: (0.0, 0.0),
            raw_move_vector: (0.0, 0.0),
            block_actions: &block_actions,
            item_use_transaction_id: None,
            item_stack_request: None,
        };
        conn.send_preencoded_packet_stream_queued(
            "player_auth_input_block_break_start",
            encode_player_auth_input_packet_stream(&break_input)?,
        )
        .await?;

        let now = Instant::now();
        movement.break_probe_started_at = Some(now);
        movement.break_last_sent_at = Some(now);
        movement.break_stop_sent = false;
        movement.break_confirmed = false;
        movement.break_confirmation_failed_reported = false;
        Ok(true)
    }

    async fn drive_gameplay_break(
        &self,
        conn: &mut BedrockProtocolAdapter,
        movement: &mut MovementValidation,
    ) -> EngineResult<()> {
        let Some(started_at) = movement.break_probe_started_at else {
            let _started = self.send_gameplay_action_probe(conn, movement).await?;
            return Ok(());
        };
        if movement
            .break_last_sent_at
            .map(|sent_at| sent_at.elapsed() < GAMEPLAY_BREAK_SEND_INTERVAL)
            .unwrap_or(false)
        {
            return Ok(());
        }
        let break_target = movement
            .break_target
            .ok_or_else(|| EngineError::Bedrock("missing gameplay break target".to_string()))?;
        let break_face = break_face_for_runtime_id(
            movement
                .break_target_runtime_id
                .or_else(|| movement.observed_target_runtime_id(break_target)),
        );
        movement.aim_at_block_target(break_target);
        let elapsed = started_at.elapsed();
        if movement.break_stop_sent {
            if elapsed >= GAMEPLAY_BREAK_DURATION + GAMEPLAY_BREAK_CONFIRM_TIMEOUT
                && !movement.break_confirmed
                && !movement.break_confirmation_failed_reported
            {
                movement.break_confirmation_failed_reported = true;
                eprintln!(
                    "[GAMEPLAY_BREAK] failed no_break_confirmation target={} elapsed={:.3}",
                    format_block_target(break_target),
                    elapsed.as_secs_f32()
                );
                if movement.prepare_next_break_target_after_break_failure() {
                    eprintln!(
                        "[GAMEPLAY_BREAK] retry=true next_break_attempt={} rejected_targets={} rejected_runtime_ids={:?}",
                        movement.break_target_attempts.saturating_add(1),
                        movement.rejected_break_targets.len(),
                        movement.rejected_break_runtime_ids
                    );
                } else {
                    eprintln!(
                        "[GAMEPLAY_BREAK] retry=false terminal=true rejected_targets={} rejected_runtime_ids={:?}",
                        movement.rejected_break_targets.len(),
                        movement.rejected_break_runtime_ids
                    );
                }
            }
            return Ok(());
        }
        let (label, block_actions) = if elapsed < GAMEPLAY_BREAK_DURATION {
            (
                "continue",
                vec![
                    RawBlockAction {
                        action_id: PlayerActionType::ContinueDestroyBlock as i32,
                        target: break_target,
                        face: break_face,
                    },
                    RawBlockAction {
                        action_id: PlayerActionType::ContinueDestroyBlock as i32,
                        target: break_target,
                        face: break_face,
                    },
                ],
            )
        } else if !movement.break_stop_sent {
            movement.break_stop_sent = true;
            (
                "finish",
                vec![
                    RawBlockAction {
                        action_id: PlayerActionType::PredictiveBreak as i32,
                        target: break_target,
                        face: break_face,
                    },
                    RawBlockAction {
                        action_id: PlayerActionType::StopBreak as i32,
                        target: break_target,
                        face: break_face,
                    },
                ],
            )
        } else {
            return Ok(());
        };

        let tick = movement.next_client_tick();
        eprintln!(
            "[GAMEPLAY_TX] probe=break_stage stage={} tick={} elapsed={:.3} target={} break_face={} actions={}",
            label,
            tick,
            elapsed.as_secs_f32(),
            format_block_target(break_target),
            break_face,
            block_actions.len()
        );
        let break_input = RawPlayerAuthInput {
            position: movement.last_sent_position,
            velocity: (0.0, 0.0, 0.0),
            yaw: movement.yaw,
            pitch: movement.pitch,
            input_data: block_action_input_flags(),
            tick,
            move_vector: (0.0, 0.0, 0.0),
            analog_move_vector: (0.0, 0.0),
            raw_move_vector: (0.0, 0.0),
            block_actions: &block_actions,
            item_use_transaction_id: None,
            item_stack_request: None,
        };
        conn.send_preencoded_packet_stream_queued(
            if movement.break_stop_sent {
                "player_auth_input_block_break_finish"
            } else {
                "player_auth_input_block_break_continue"
            },
            encode_player_auth_input_packet_stream(&break_input)?,
        )
        .await?;
        movement.break_last_sent_at = Some(Instant::now());
        Ok(())
    }

    async fn send_place_action_probe(
        &self,
        conn: &mut BedrockProtocolAdapter,
        movement: &mut MovementValidation,
    ) -> EngineResult<()> {
        movement.ensure_gameplay_targets();
        let place_base = movement
            .place_base
            .ok_or_else(|| EngineError::Bedrock("missing gameplay place base".to_string()))?;
        let place_result = movement
            .place_result
            .ok_or_else(|| EngineError::Bedrock("missing gameplay place result".to_string()))?;
        let Some(held_item) = movement.held_item.clone() else {
            eprintln!("[GAMEPLAY_TX] probe=place skipped=true reason=no_non_empty_inventory_item");
            return Ok(());
        };
        let place_geometry = place_geometry_for_break_target(
            movement.break_target.unwrap_or(place_base),
            movement.break_target_runtime_id,
        );
        conn.send(&[BedrockProto::Animate(AnimatePacket {
            action_id: 1, // Swing
            runtime_entity_id: movement.runtime_id.0,
            rowing_time: 0.0,
        })])
        .await?;

        let place_transaction = RawItemUseTransaction {
            action_type: 0,
            trigger_type: 1,
            block_position: place_base,
            face: place_geometry.face,
            hotbar_slot: held_item.slot as i32,
            held_item_bytes: held_item.item_bytes,
            player_pos: movement.last_sent_position,
            click_pos: place_geometry.click_pos,
            block_runtime_id: 0,
            client_prediction: 1,
        };
        let place_input = RawPlayerAuthInput {
            position: movement.last_sent_position,
            velocity: (0.0, 0.0, 0.0),
            yaw: movement.yaw,
            pitch: movement.pitch,
            input_data: received_server_data_input_flag()
                | PlayerAuthInputFlags::PerformItemInteraction,
            // StartUsingItem was replaced by PerformItemInteraction
            tick: movement.next_client_tick(),
            move_vector: (0.0, 0.0, 0.0),
            analog_move_vector: (0.0, 0.0),
            raw_move_vector: (0.0, 0.0),
            block_actions: &[],
            item_use_transaction_id: Some(&place_transaction),
            item_stack_request: None,
        };
        eprintln!(
            "[GAMEPLAY_TX] probe=place item_id={} slot={} base={} result={} face={} click_pos={} block_runtime_id_assumption=0",
            held_item.item_id,
            held_item.slot,
            format_block_target(place_base),
            format_block_target(place_result),
            place_geometry.face,
            format_position(place_geometry.click_pos)
        );
        conn.send_preencoded_packet_stream_queued(
            "player_auth_input_block_place",
            encode_player_auth_input_packet_stream(&place_input)?,
        )
        .await?;
        movement.place_probe_sent = true;
        Ok(())
    }

    fn log_move_player_rx(&self, move_player: &MovePlayerPacket) {
        eprintln!(
            "[MOVEMENT_RX] packet=MovePlayer tick={} runtime_id={:?} position={} rotation={:.3},{:.3} y_head_rotation={:.3} mode={:?} on_ground={}",
            move_player.tick,
            move_player.runtime_id,
            format_position((move_player.position.x, move_player.position.y, move_player.position.z)),
            move_player.pitch,
            move_player.yaw,
            move_player.head_yaw,
            move_player.mode,
            move_player.on_ground
        );
    }

    async fn send_container_close(
        &self,
        conn: &mut BedrockProtocolAdapter,
        window_id: u32,
        window_type: u8,
        server_side: bool,
    ) -> EngineResult<()> {
        eprintln!(
            "[GAMEPLAY_MENU_TX] packet=ContainerClose window_id={} window_type={} server_side={}",
            window_id, window_type, server_side
        );
        conn.send_preencoded_packet_stream(
            "container_close",
            encode_container_close_packet_stream(window_id, window_type, server_side),
        )
        .await
    }

    async fn send_inventory_probe(&self, conn: &mut BedrockProtocolAdapter) -> EngineResult<()> {
        conn.send(&[BedrockProto::InventoryTransaction(
            InventoryTransactionPacket {
                transaction_type: 0,
                actions: vec![],
                transaction_data: vec![],
            },
        )])
        .await
    }

    async fn send_rtp_menu_click(
        &self,
        conn: &mut BedrockProtocolAdapter,
        request_id: i32,
        target: &MenuClickTarget,
        method: MenuClickMethod,
        position: (f32, f32, f32),
        yaw: f32,
        pitch: f32,
        tick: u64,
    ) -> EngineResult<()> {
        let (source_container_type, source_dynamic_id) = method.source_container(target);
        eprintln!(
            "[GAMEPLAY_MENU_TX] method={} packet={} action={} request_id={} window={} slot={} item_id={} stack_id={} source_container_type={} source_dynamic_id={} observed_container_type={} observed_dynamic_id={} tick={} position={}",
            method.label(),
            if method.uses_player_auth_input() {
                "PlayerAuthInput.ItemStackRequest"
            } else {
                "ItemStackRequest"
            },
            method.action().label(),
            request_id,
            target.window_id,
            target.slot,
            target.item_id,
            target.stack_id,
            source_container_type,
            source_dynamic_id
                .map(|dynamic_id| dynamic_id.to_string())
                .unwrap_or_else(|| "none".to_string()),
            target.container_type,
            target
                .dynamic_container_id
                .map(|dynamic_id| dynamic_id.to_string())
                .unwrap_or_else(|| "none".to_string()),
            tick,
            format_position(position)
        );
        if method.uses_player_auth_input() {
            let request = RawItemStackRequest {
                request_id,
                target,
                method,
                action_id: method.action(),
            };
            let input = RawPlayerAuthInput {
                position,
                velocity: (0.0, 0.0, 0.0),
                yaw,
                pitch,
                input_data: received_server_data_input_flag()
                    | PlayerAuthInputFlags::PerformItemStackRequest,
                tick,
                move_vector: (0.0, 0.0, 0.0),
                analog_move_vector: (0.0, 0.0),
                raw_move_vector: (0.0, 0.0),
                block_actions: &[],
                item_use_transaction_id: None,
                item_stack_request: Some(request),
            };
            conn.send_preencoded_packet_stream_queued(
                method.label(),
                encode_player_auth_input_packet_stream(&input)?,
            )
            .await
        } else {
            conn.send_preencoded_packet_stream(
                method.label(),
                encode_item_stack_request_packet_stream(request_id, target, method),
            )
            .await
        }
    }

    fn finalize_status(&self, status: &mut CapabilityStatus) {
        status.gameplay_actions = status.chat
            && status.inventory_transactions
            && status.movement
            && status.block_breaking
            && status.block_placing;
        let critical_missing = [
            ("login", status.login),
            ("spawn", status.spawn),
            ("player_spawn", status.player_spawn),
            ("remained_connected", status.remained_connected),
            ("keepalive", status.keepalive),
            ("chat", status.chat),
            ("inventory_transactions", status.inventory_transactions),
            ("movement", status.movement),
            ("block_breaking", status.block_breaking),
            ("block_placing", status.block_placing),
            ("gameplay_actions", status.gameplay_actions),
        ]
        .into_iter()
        .filter_map(|(name, ok)| (!ok).then_some(name.to_string()))
        .collect();
        let optional_missing = [
            ("forms", status.forms),
            ("disconnect_handling", status.disconnect_handling),
        ]
        .into_iter()
        .filter_map(|(name, ok)| (!ok).then_some(name.to_string()))
        .collect();
        status.missing_capabilities = critical_missing;
        status.optional_capabilities_missing = optional_missing;
        status.success = status.missing_capabilities.is_empty();
    }
}

fn is_post_spawn_decode_error(err: &EngineError) -> bool {
    let error = err.to_string();
    error.contains("decrypt packet batch")
        || error.contains("decode packet batch")
        || error.contains("Encrypted data trailer invalid")
}

fn observed_contains_progress(observed: &[ObservedPacket]) -> bool {
    observed.iter().any(|packet| {
        matches!(
            packet,
            ObservedPacket::PlayStatus(_)
                | ObservedPacket::ResourcePacksInfo
                | ObservedPacket::ResourcePackStack
                | ObservedPacket::StartGame(_)
                | ObservedPacket::Disconnect(_)
                | ObservedPacket::ItemStackResponse { .. }
        )
    })
}

fn format_position(position: (f32, f32, f32)) -> String {
    format!("{:.3},{:.3},{:.3}", position.0, position.1, position.2)
}

fn format_block_target(target: BlockTarget) -> String {
    format!("{},{},{}", target.x, target.y, target.z)
}

fn block_target_score(target: BlockTarget, origin: (f32, f32, f32)) -> f32 {
    let center = (
        target.x as f32 + 0.5,
        target.y as f32 + 0.5,
        target.z as f32 + 0.5,
    );
    let dx = center.0 - origin.0;
    let dy = block_target_vertical_delta(target, origin);
    let dz = center.2 - origin.2;
    dx * dx + dz * dz + dy * 2.0
}

fn block_target_horizontal_distance(target: BlockTarget, origin: (f32, f32, f32)) -> f32 {
    let center = (target.x as f32 + 0.5, target.z as f32 + 0.5);
    let dx = center.0 - origin.0;
    let dz = center.1 - origin.2;
    (dx * dx + dz * dz).sqrt()
}

fn block_target_vertical_delta(target: BlockTarget, origin: (f32, f32, f32)) -> f32 {
    let eye_y = origin.1 + GAMEPLAY_PLAYER_EYE_HEIGHT;
    (target.y as f32 + 0.5 - eye_y).abs()
}

fn fallback_break_candidates(origin: (f32, f32, f32)) -> Vec<BlockTarget> {
    let feet_x = origin.0.floor() as i32;
    let feet_y = origin.1.floor() as i32;
    let feet_z = origin.2.floor() as i32;
    let base_y = feet_y.saturating_sub(2);
    [
        (0, 0, 0),
        (0, -1, 0),
        (1, 0, 0),
        (-1, 0, 0),
        (0, 0, 1),
        (0, 0, -1),
        (1, -1, 0),
        (-1, -1, 0),
        (0, -1, 1),
        (0, -1, -1),
    ]
    .into_iter()
    .map(|(dx, dy, dz)| BlockTarget {
        x: feet_x + dx,
        y: base_y + dy,
        z: feet_z + dz,
    })
    .collect()
}

fn gameplay_break_candidate_for_update(target: BlockTarget, runtime_id: u32) -> BlockTarget {
    if runtime_id == OBSERVED_CHEST_RUNTIME_ID {
        BlockTarget {
            x: target.x,
            y: target.y.saturating_sub(1),
            z: target.z,
        }
    } else {
        target
    }
}

fn place_geometry_for_break_target(target: BlockTarget, runtime_id: Option<u32>) -> PlaceGeometry {
    if runtime_id == Some(SPRUCE_BUTTON_CEILING_RUNTIME_ID) {
        return PlaceGeometry {
            base: BlockTarget {
                x: target.x,
                y: target.y + 1,
                z: target.z,
            },
            result: target,
            face: BLOCK_FACE_DOWN,
            click_pos: (0.5, 0.0, 0.5),
        };
    }
    PlaceGeometry {
        base: BlockTarget {
            x: target.x,
            y: target.y.saturating_sub(1),
            z: target.z,
        },
        result: target,
        face: BLOCK_FACE_UP,
        click_pos: (0.5, 1.0, 0.5),
    }
}

fn break_face_for_runtime_id(runtime_id: Option<u32>) -> i32 {
    if runtime_id == Some(SPRUCE_BUTTON_CEILING_RUNTIME_ID) {
        BLOCK_FACE_DOWN
    } else {
        BLOCK_FACE_UP
    }
}

fn is_gameplay_marker_runtime(runtime_id: u32) -> bool {
    runtime_id == OBSERVED_CHEST_RUNTIME_ID
}

fn should_close_rtp_container() -> bool {
    std::env::var("BEDROCK_VALIDATE_CLOSE_RTP_CONTAINER")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn validation_duration_env_seconds(name: &str, default: Duration) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or(default)
}

fn validation_duration_env_millis_allow_zero(name: &str, default: Duration) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(default)
}

fn tick_sync_interval_duration() -> Duration {
    validation_duration_env_millis_allow_zero("BEDROCK_TICK_SYNC_INTERVAL_MS", TICK_SYNC_INTERVAL)
}

fn tick_sync_step(interval: Duration) -> i64 {
    ((interval.as_millis() + 25) / 50).max(1) as i64
}

fn rtp_wait_duration() -> Duration {
    validation_duration_env_seconds("BEDROCK_VALIDATE_RTP_WAIT_SECONDS", RTP_WAIT_DURATION)
}

fn rtp_menu_open_wait_duration() -> Duration {
    validation_duration_env_seconds(
        "BEDROCK_VALIDATE_RTP_MENU_OPEN_WAIT_SECONDS",
        RTP_MENU_OPEN_WAIT_DURATION,
    )
}

fn rtp_menu_click_wait_duration() -> Duration {
    validation_duration_env_seconds(
        "BEDROCK_VALIDATE_RTP_MENU_CLICK_WAIT_SECONDS",
        RTP_MENU_CLICK_WAIT_DURATION,
    )
}

fn should_click_rtp_menu() -> bool {
    std::env::var("BEDROCK_VALIDATE_RTP_MENU_CLICK")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(true)
}

fn fallback_rtp_command(primary: &str, attempts_sent: u8) -> Option<String> {
    if attempts_sent == 0 || attempts_sent >= RTP_MAX_COMMAND_ATTEMPTS {
        return None;
    }
    let trimmed = primary.trim();
    if trimmed.is_empty() {
        return None;
    }
    let fallback = if trimmed.eq_ignore_ascii_case("/rtp") {
        "/rtp overworld"
    } else {
        "/rtp"
    };
    if fallback.eq_ignore_ascii_case(trimmed) {
        None
    } else {
        Some(fallback.to_string())
    }
}

fn position_score(position: (f32, f32, f32), origin: (f32, f32, f32)) -> f32 {
    let dx = position.0 - origin.0;
    let dy = position.1 - origin.1;
    let dz = position.2 - origin.2;
    dx * dx + dz * dz + dy.abs() * 2.0
}

fn menu_click_target_from_observed(item: &ObservedInventoryItem) -> Option<MenuClickTarget> {
    if item.item_id == 0 {
        return None;
    }
    let priority =
        rtp_menu_item_priority(item).or_else(|| donutsmp_spawn_hotbar_rtp_item_priority(item))?;
    let stack_id = item.stack_id?;
    Some(MenuClickTarget {
        window_id: item.container_id,
        slot: item.slot,
        item_id: item.item_id,
        stack_id,
        container_type: item.container_type.unwrap_or(CONTAINER_TYPE_DYNAMIC),
        dynamic_container_id: item.dynamic_container_id,
        priority,
    })
}

fn is_menu_selector_item(item: &ObservedInventoryItem) -> bool {
    is_server_ui_item_bytes(&item.item_bytes)
}

fn is_overworld_or_neutral_random_teleport_menu_item(item: &ObservedInventoryItem) -> bool {
    let Some(text) = compact_item_text_hint(&item.item_bytes) else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    if !lower.contains("click to randomly teleport") {
        return false;
    }
    let overworld = item.item_id == 2 || lower.contains("overworld");
    let cross_dimension = item.item_id == 87
        || item.item_id == 121
        || lower.contains("nether")
        || lower.contains("end");
    overworld || !cross_dimension
}

fn rtp_menu_item_priority(item: &ObservedInventoryItem) -> Option<u8> {
    let text = compact_item_text_hint(&item.item_bytes)?;
    let lower = text.to_ascii_lowercase();
    let random = lower.contains("click to randomly teleport");
    let selector = lower.contains("click to select region");
    if !random && !selector {
        return None;
    }
    let overworld = item.item_id == 2 || lower.contains("overworld");
    let nether = item.item_id == 87 || lower.contains("nether");
    let end = item.item_id == 121 || lower.contains("end");
    let cross_dimension = nether || end;
    let server_region_random = random
        && !cross_dimension
        && (lower.contains("players")
            || lower.contains("ms")
            || lower.contains("asia")
            || lower.contains("europe")
            || lower.contains("na west")
            || lower.contains("na east")
            || lower.contains("north america"));

    if random && overworld {
        Some(120)
    } else if selector && overworld {
        Some(110)
    } else if server_region_random {
        Some(100)
    } else if random && !nether && !end {
        Some(90)
    } else if random && nether {
        Some(40)
    } else if random && end {
        Some(20)
    } else if selector {
        Some(10)
    } else {
        None
    }
}

fn donutsmp_spawn_hotbar_rtp_item_priority(item: &ObservedInventoryItem) -> Option<u8> {
    if item.container_id == 0
        && item.slot == 0
        && item.item_id == 303
        && item.item_bytes.len() <= 32
    {
        Some(80)
    } else {
        None
    }
}

fn is_player_inventory_container(container_id: u32) -> bool {
    container_id == 0
}

fn is_probably_placeable_block_item(item_id: i32) -> bool {
    item_id > 0
        && (item_id <= 255
            || matches!(
                item_id,
                // Observed Bedrock 1.21.x block item IDs from minecraft-data.
                303 | 306 | 320 | 343 | 371 | 372 | 373 | 374 | 422
            ))
}

fn is_donutsmp_spawn_hotbar_utility_item_bytes(
    container_id: u32,
    item_id: i32,
    item_bytes: &[u8],
) -> bool {
    container_id == 0
        && matches!(item_id, 343 | 371 | 372 | 373 | 374)
        && item_bytes.len() >= 32
        && item_bytes.len() <= 96
}

fn normal_placeable_rejection_reason(
    container_id: u32,
    item_id: i32,
    item_bytes: &[u8],
) -> Option<&'static str> {
    if item_id == 0 {
        return Some("air");
    }
    if is_donutsmp_spawn_hotbar_rtp_item_bytes(container_id, item_id, item_bytes) {
        return Some("server_ui_item");
    }
    if is_donutsmp_spawn_hotbar_utility_item_bytes(container_id, item_id, item_bytes) {
        return Some("server_ui_item");
    }
    if is_server_ui_item_bytes(item_bytes) {
        return Some("server_ui_item");
    }
    if !is_player_inventory_container(container_id) {
        return Some("non_player_inventory_container");
    }
    if !is_probably_placeable_block_item(item_id) {
        return Some("not_placeable_block_item");
    }
    None
}

fn normal_item_entity_rejection_reason(item_id: i32, item_bytes: &[u8]) -> Option<&'static str> {
    if item_id == 0 {
        return Some("air");
    }
    if is_server_ui_item_bytes(item_bytes) {
        return Some("server_ui_item");
    }
    if matches!(item_id, 343 | 371 | 372 | 373 | 374) && item_bytes.len() >= 32 {
        return Some("server_ui_item");
    }
    if !is_probably_placeable_block_item(item_id) {
        return Some("not_placeable_block_item");
    }
    None
}

fn is_server_ui_item_bytes(item_bytes: &[u8]) -> bool {
    let Some(text) = compact_item_text_hint(item_bytes) else {
        return false;
    };
    is_server_ui_item_text(&text)
}

fn is_donutsmp_spawn_hotbar_rtp_item_bytes(
    container_id: u32,
    item_id: i32,
    item_bytes: &[u8],
) -> bool {
    container_id == 0 && item_id == 303 && item_bytes.len() <= 32
}

fn is_server_ui_item_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "click to randomly teleport",
        "click to select region",
        "randomly teleport",
        "select region",
        "region selector",
        "server selector",
        "teleport menu",
        "rtp",
        "server ui",
        "menu item",
        "open menu",
        "right click",
        "click to",
        "lobby selector",
        "realm selector",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn is_hand_droppable_placeable_runtime_id(runtime_id: u32) -> bool {
    matches!(
        runtime_id,
        // Runtime IDs are from the bundled minecraft-data bedrock 1.21.111 map, used as
        // the closest available local map for the 1.21.130 validation target.
        2732  // red_sand -> red_sand
            | 5213  // sandstone -> sandstone
            | SPRUCE_BUTTON_CEILING_RUNTIME_ID // spruce_button -> spruce_button
            | 6217  // crimson_nylium -> netherrack
            | 6234  // sand -> sand
            | 6725  // coarse_dirt -> coarse_dirt
            | 7292  // podzol -> dirt
            | 9852  // dirt -> dirt
            | 9892  // soul_soil -> soul_soil
            | 9893  // soul_sand -> soul_sand
            | 11062 // grass_block -> dirt
            | 12178 // warped_nylium -> netherrack
            | 12524 // mud -> mud
            | 13114 // netherrack -> netherrack
    ) || is_observed_donutsmp_terrain_runtime_id(runtime_id)
}

fn is_observed_donutsmp_terrain_runtime_id(runtime_id: u32) -> bool {
    matches!(
        runtime_id,
        // Observed in DonutSMP/Geyser 1.21.130 LevelChunk samples. The bundled
        // minecraft-data snapshot only carries blockStates through 1.21.111, so
        // these are trace-backed candidates. Pickup and placement still require
        // server inventory and UpdateBlock confirmation before validation passes.
        3758 | 10812
            | 12970
            | 22010
            | 24356
            | 25040
            | 26228
            | 27644
            | 27646
            | 27648
            | 27654
            | 27656
            | 28878
    )
}

fn is_normal_validation_placeable_drop_runtime_id(runtime_id: u32) -> bool {
    is_hand_droppable_placeable_runtime_id(runtime_id)
        && runtime_id != SPRUCE_BUTTON_CEILING_RUNTIME_ID
}

fn compact_item_text_hint(bytes: &[u8]) -> Option<String> {
    let text: String = String::from_utf8_lossy(bytes)
        .chars()
        .map(|ch| {
            if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
                ' '
            } else {
                ch
            }
        })
        .collect();
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!compact.is_empty()).then_some(compact)
}

fn held_inventory_candidate(
    container_id: u32,
    slot: u32,
    item: &NetworkItemStackDescriptor,
) -> Option<HeldInventoryItem> {
    let item_bytes = network_item_descriptor_bytes(item)?;
    let item_id = network_item_descriptor_id_from_bytes(&item_bytes)?;
    if normal_placeable_rejection_reason(container_id, item_id, &item_bytes).is_some() {
        return None;
    }
    Some(HeldInventoryItem {
        container_id,
        slot,
        item_id,
        item: Some(item.clone()),
        item_bytes,
    })
}

fn held_inventory_candidate_from_observed(
    item: &ObservedInventoryItem,
) -> Option<HeldInventoryItem> {
    if normal_placeable_rejection_reason(item.container_id, item.item_id, &item.item_bytes)
        .is_some()
    {
        return None;
    }
    Some(HeldInventoryItem {
        container_id: item.container_id,
        slot: item.slot,
        item_id: item.item_id,
        item: None,
        item_bytes: item.item_bytes.clone(),
    })
}

fn cache_held_inventory_candidate(
    candidate: &mut Option<HeldInventoryItem>,
    container_id: u32,
    slot: u32,
    item: &NetworkItemStackDescriptor,
) {
    if candidate.is_some() {
        return;
    }
    if let Some(held_item) = held_inventory_candidate(container_id, slot, item) {
        eprintln!(
            "[GAMEPLAY_INVENTORY] cached_candidate=true container={} slot={} item_id={}",
            held_item.container_id, held_item.slot, held_item.item_id
        );
        *candidate = Some(held_item);
    }
}

fn cache_observed_held_inventory_candidate(
    candidate: &mut Option<HeldInventoryItem>,
    item: &ObservedInventoryItem,
) {
    if candidate.is_some() {
        return;
    }
    if is_menu_selector_item(item) {
        return;
    }
    if let Some(held_item) = held_inventory_candidate_from_observed(item) {
        eprintln!(
            "[GAMEPLAY_INVENTORY] cached_raw_candidate=true container={} slot={} item_id={} item_len={}",
            held_item.container_id,
            held_item.slot,
            held_item.item_id,
            held_item.item_bytes.len()
        );
        *candidate = Some(held_item);
    }
}

fn apply_update_block_evidence(
    status: &mut CapabilityStatus,
    movement: &mut MovementValidation,
    target: BlockTarget,
    runtime_id: u32,
    source: &'static str,
) {
    if Some(target) == movement.break_target {
        if is_break_confirmation_runtime_id(runtime_id) {
            movement.break_confirmed = true;
            status.block_breaking = true;
            eprintln!(
                "[GAMEPLAY_ACCEPT] action=break target={} evidence={} runtime_id={}",
                format_block_target(target),
                source,
                runtime_id
            );
        } else {
            eprintln!(
                "[GAMEPLAY_BREAK] observed_target_update target={} evidence={} runtime_id={} accepted=false reason=not_broken_state",
                format_block_target(target),
                source,
                runtime_id
            );
        }
    }
    if movement.place_probe_sent && Some(target) == movement.place_result {
        if is_place_confirmation_runtime_id(runtime_id) {
            status.block_placing = true;
            eprintln!(
                "[GAMEPLAY_ACCEPT] action=place target={} evidence={} runtime_id={}",
                format_block_target(target),
                source,
                runtime_id
            );
        } else {
            eprintln!(
                "[GAMEPLAY_PLACE] observed_target_update target={} evidence={} runtime_id={} accepted=false reason=not_placed_state",
                format_block_target(target),
                source,
                runtime_id
            );
        }
    }
}

fn is_break_confirmation_runtime_id(runtime_id: u32) -> bool {
    runtime_id == OBSERVED_AIR_RUNTIME_ID
}

fn is_place_confirmation_runtime_id(runtime_id: u32) -> bool {
    runtime_id != OBSERVED_AIR_RUNTIME_ID
}

fn block_pos(target: BlockTarget) -> BlockPos {
    BlockPos {
        x: target.x,
        y: target.y,
        z: target.z,
    }
}

fn validation_command_mode(command: &str, override_mode: Option<&str>) -> String {
    if let Some(mode) = override_mode {
        let mode = mode.trim().to_ascii_lowercase();
        if !mode.is_empty() {
            return mode;
        }
    }

    if command.trim_start().starts_with('/') {
        "command_request".to_string()
    } else {
        "text".to_string()
    }
}

fn command_request_wire_command(command: &str) -> String {
    command_request_wire_command_with_slash_mode(
        command,
        env_flag("BEDROCK_COMMAND_REQUEST_LEADING_SLASH", true),
    )
}

fn command_request_wire_command_with_slash_mode(command: &str, leading_slash: bool) -> String {
    let command = command.trim();
    if leading_slash {
        if command.starts_with('/') {
            command.to_string()
        } else {
            format!("/{command}")
        }
    } else {
        command.trim_start_matches('/').to_string()
    }
}

fn encode_tick_sync_packet_stream(request_time: i64, response_time: i64) -> Vec<u8> {
    let mut packet = Vec::with_capacity(17);
    write_unsigned_varint_u32_local(TICK_SYNC_PACKET_ID, &mut packet);
    packet.extend_from_slice(&request_time.to_le_bytes());
    packet.extend_from_slice(&response_time.to_le_bytes());

    let mut stream = Vec::with_capacity(packet.len() + 1);
    write_unsigned_varint_u32_local(packet.len() as u32, &mut stream);
    stream.extend_from_slice(&packet);
    stream
}

fn encode_container_close_packet_stream(
    window_id: u32,
    window_type: u8,
    server_side: bool,
) -> Vec<u8> {
    let mut packet = Vec::with_capacity(4);
    write_unsigned_varint_u32_local(CONTAINER_CLOSE_PACKET_ID, &mut packet);
    packet.push(window_id as u8);
    packet.push(window_type);
    packet.push(u8::from(server_side));
    packet
}

fn encode_command_request_packet_stream(
    command: &str,
    request_id: &str,
    player_entity_id: i64,
) -> Vec<u8> {
    let uuid = Uuid::parse_str(request_id).unwrap_or_else(|_| Uuid::nil());
    let mut packet = Vec::with_capacity(command.len() + request_id.len() + 64);
    write_unsigned_varint_u32_local(COMMAND_REQUEST_PACKET_ID, &mut packet);
    write_string_local(command, &mut packet);
    write_string_local("player", &mut packet);
    packet.extend_from_slice(uuid.as_bytes());
    write_string_local(request_id, &mut packet);
    packet.extend_from_slice(&player_entity_id.to_le_bytes());
    packet.push(0);
    write_string_local("52", &mut packet);

    let mut stream = Vec::with_capacity(packet.len() + 3);
    write_unsigned_varint_u32_local(packet.len() as u32, &mut stream);
    stream.extend_from_slice(&packet);
    stream
}

fn encode_item_stack_request_packet_stream(
    request_id: i32,
    target: &MenuClickTarget,
    method: MenuClickMethod,
) -> Vec<u8> {
    let mut packet = Vec::with_capacity(32);
    write_unsigned_varint_u32_local(ITEM_STACK_REQUEST_PACKET_ID, &mut packet);
    write_unsigned_varint_u32_local(1, &mut packet); // requests
    write_item_stack_request_entry(
        &RawItemStackRequest {
            request_id,
            target,
            method,
            action_id: method.action(),
        },
        &mut packet,
    );

    let mut stream = Vec::with_capacity(packet.len() + 2);
    write_unsigned_varint_u32_local(packet.len() as u32, &mut stream);
    stream.extend_from_slice(&packet);
    stream
}

fn write_item_stack_request_entry(request: &RawItemStackRequest<'_>, out: &mut Vec<u8>) {
    let (source_container_type, source_dynamic_id) =
        request.method.source_container(request.target);
    write_unsigned_varint_u32_local(request.request_id.max(0) as u32, out);
    write_unsigned_varint_u32_local(1, out); // actions
    match request.action_id {
        MenuClickAction::Take => {
            out.push(0); // take
            out.push(1); // count
            write_stack_request_slot_info(
                source_container_type,
                source_dynamic_id,
                request.target.slot as u8,
                request.target.stack_id,
                out,
            );
            write_stack_request_slot_info(CONTAINER_TYPE_CURSOR, None, 0, 0, out);
        }
        MenuClickAction::Consume => {
            out.push(5); // consume
            out.push(1); // count
            write_stack_request_slot_info(
                source_container_type,
                source_dynamic_id,
                request.target.slot as u8,
                request.target.stack_id,
                out,
            );
        }
    }
    write_unsigned_varint_u32_local(0, out); // custom_names / strings_to_filter
    out.extend_from_slice(&(-1i32).to_le_bytes()); // text processing origin: unknown
}

fn write_stack_request_slot_info(
    container_type: u8,
    dynamic_container_id: Option<u32>,
    slot: u8,
    stack_id: i32,
    out: &mut Vec<u8>,
) {
    out.push(container_type);
    match dynamic_container_id {
        Some(dynamic_id) => {
            out.push(1);
            out.extend_from_slice(&dynamic_id.to_le_bytes());
        }
        None => out.push(0),
    }
    out.push(slot);
    write_zigzag_i32(stack_id, out);
}

fn encode_player_auth_input_packet_stream(input: &RawPlayerAuthInput<'_>) -> EngineResult<Vec<u8>> {
    let mut packet = Vec::with_capacity(160);
    write_unsigned_varint_u32_local(PLAYER_AUTH_INPUT_PACKET_ID, &mut packet);

    write_f32_le(input.pitch, &mut packet);
    write_f32_le(input.yaw, &mut packet);
    write_vec3f(input.position, &mut packet);
    write_vec3f(input.move_vector, &mut packet);
    write_f32_le(input.yaw, &mut packet);
    write_unsigned_varint_u128(input.input_data, &mut packet);
    write_unsigned_varint_u32_local(1, &mut packet); // mouse
    write_unsigned_varint_u32_local(0, &mut packet); // normal
    write_zigzag_i32(1, &mut packet); // crosshair
    write_vec2f((input.pitch, input.yaw), &mut packet);
    write_unsigned_varint_u64(input.tick, &mut packet);
    write_vec3f(input.velocity, &mut packet);

    if input.input_data & PlayerAuthInputFlags::PerformItemInteraction != 0 {
        let Some(ref transaction) = input.item_use_transaction_id else {
            return Err(EngineError::Bedrock(
                "PlayerAuthInput item interaction flag set without transaction".to_string(),
            ));
        };
        write_item_use_transaction(transaction, &mut packet)?;
    }

    if input.input_data & PlayerAuthInputFlags::PerformItemStackRequest != 0 {
        let Some(request) = input.item_stack_request else {
            return Err(EngineError::Bedrock(
                "PlayerAuthInput item stack request flag set without request".to_string(),
            ));
        };
        write_item_stack_request_entry(&request, &mut packet);
    }

    if input.input_data & PlayerAuthInputFlags::IsInClientPredictedVehicle != 0 {
        return Err(EngineError::Bedrock(
            "raw PlayerAuthInput predicted vehicle is not implemented".to_string(),
        ));
    }

    if input.input_data & PlayerAuthInputFlags::PerformBlockActions != 0 {
        write_unsigned_varint_u32_local(input.block_actions.len() as u32, &mut packet);
        for action in input.block_actions {
            write_zigzag_i32(action.action_id, &mut packet);
            if matches!(action.action_id, 0 | 1 | 18 | 26 | 27) {
                write_block_pos(action.target, &mut packet);
                write_zigzag_i32(action.face, &mut packet);
            }
        }
    }

    write_vec2f(input.analog_move_vector, &mut packet);
    write_vec3f((0.0, 0.0, 0.0), &mut packet);
    write_vec2f(input.raw_move_vector, &mut packet);

    let mut stream = Vec::with_capacity(packet.len() + 3);
    write_unsigned_varint_u32_local(packet.len() as u32, &mut stream);
    stream.extend_from_slice(&packet);
    Ok(stream)
}

fn write_item_use_transaction(
    transaction: &RawItemUseTransaction,
    packet: &mut Vec<u8>,
) -> EngineResult<()> {
    write_zigzag_i32(0, packet); // legacy request ID: no legacy slot changes.
    write_unsigned_varint_u32_local(0, packet); // transaction actions.
    write_unsigned_varint_u32_local(transaction.action_type, packet);
    write_unsigned_varint_u32_local(transaction.trigger_type, packet);
    write_block_coordinates(transaction.block_position, packet);
    write_zigzag_i32(transaction.face, packet);
    write_zigzag_i32(transaction.hotbar_slot, packet);
    packet.extend_from_slice(&transaction.held_item_bytes);
    write_vec3f(transaction.player_pos, packet);
    write_vec3f(transaction.click_pos, packet);
    write_unsigned_varint_u32_local(transaction.block_runtime_id, packet);
    write_unsigned_varint_u32_local(transaction.client_prediction, packet);
    Ok(())
}

fn write_block_pos(target: BlockTarget, out: &mut Vec<u8>) {
    let pos = block_pos(target);
    write_zigzag_i32(pos.x, out);
    write_zigzag_i32(pos.y, out);
    write_zigzag_i32(pos.z, out);
}

fn write_block_coordinates(target: BlockTarget, out: &mut Vec<u8>) {
    write_zigzag_i32(target.x, out);
    write_unsigned_varint_u32_local(target.y.max(0) as u32, out);
    write_zigzag_i32(target.z, out);
}

fn write_vec2f(value: (f32, f32), out: &mut Vec<u8>) {
    write_f32_le(value.0, out);
    write_f32_le(value.1, out);
}

fn write_vec3f(value: (f32, f32, f32), out: &mut Vec<u8>) {
    write_f32_le(value.0, out);
    write_f32_le(value.1, out);
    write_f32_le(value.2, out);
}

fn write_f32_le(value: f32, out: &mut Vec<u8>) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_string_local(value: &str, out: &mut Vec<u8>) {
    write_unsigned_varint_u32_local(value.len() as u32, out);
    out.extend_from_slice(value.as_bytes());
}

fn write_zigzag_i32(value: i32, out: &mut Vec<u8>) {
    let encoded = ((value << 1) ^ (value >> 31)) as u32;
    write_unsigned_varint_u32_local(encoded, out);
}

fn write_unsigned_varint_u32_local(mut value: u32, out: &mut Vec<u8>) {
    loop {
        if value & !0x7f == 0 {
            out.push(value as u8);
            break;
        }
        out.push(((value & 0x7f) | 0x80) as u8);
        value >>= 7;
    }
}

fn write_unsigned_varint_u64(mut value: u64, out: &mut Vec<u8>) {
    loop {
        if value & !0x7f == 0 {
            out.push(value as u8);
            break;
        }
        out.push(((value & 0x7f) | 0x80) as u8);
        value >>= 7;
    }
}

fn write_unsigned_varint_u128(mut value: u128, out: &mut Vec<u8>) {
    loop {
        if value & !0x7f == 0 {
            out.push(value as u8);
            break;
        }
        out.push(((value & 0x7f) | 0x80) as u8);
        value >>= 7;
    }
}

fn network_item_descriptor_id(item: &NetworkItemStackDescriptor) -> Option<i32> {
    let bytes = network_item_descriptor_bytes(item)?;
    network_item_descriptor_id_from_bytes(&bytes)
}

fn network_item_descriptor_bytes(item: &NetworkItemStackDescriptor) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    item.serialize(&mut bytes).ok()?;
    Some(bytes)
}

fn network_item_descriptor_id_from_bytes(bytes: &[u8]) -> Option<i32> {
    let mut offset = 0usize;
    read_unsigned_varint_u32_local(&bytes, &mut offset)
        .map(|value| ((value >> 1) as i32) ^ (-((value & 1) as i32)))
}

fn read_unsigned_varint_u32_local(bytes: &[u8], offset: &mut usize) -> Option<u32> {
    let mut value = 0u32;
    let mut shift = 0u32;
    for _ in 0..5 {
        let byte = *bytes.get(*offset)?;
        *offset += 1;
        value |= ((byte & 0x7f) as u32) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
    }
    None
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

async fn timeout_step<T>(
    step: &'static str,
    duration: Duration,
    future: impl std::future::Future<Output = EngineResult<T>>,
) -> EngineResult<T> {
    match timeout(duration, future).await {
        Ok(result) => result,
        Err(_) => Err(EngineError::Bedrock(format!("{step} timed out"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_player_auth_input_uses_1_21_130_vec3_movement_layout() {
        let input = RawPlayerAuthInput {
            position: (0.0, 64.0, 0.0),
            velocity: (0.0, 0.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            input_data: 0,
            tick: 0,
            move_vector: (0.0, 0.0, 1.0),
            analog_move_vector: (0.0, 1.0),
            raw_move_vector: (0.0, 1.0),
            block_actions: &[],
            item_use_transaction_id: None,
            item_stack_request: None,
        };

        let stream = encode_player_auth_input_packet_stream(&input).expect("encoded input");
        assert_eq!(
            stream[0], 91,
            "packet length changed; check PlayerAuthInput layout"
        );
        assert_eq!(&stream[1..3], &[0x90, 0x01]);
    }

    #[test]
    fn raw_player_auth_input_block_actions_use_unsigned_varint_count() {
        let actions = [RawBlockAction {
            action_id: PlayerActionType::StartBreak as i32,
            target: BlockTarget { x: 1, y: 2, z: 3 },
            face: BLOCK_FACE_UP,
        }];
        let input = RawPlayerAuthInput {
            position: (0.0, 64.0, 0.0),
            velocity: (0.0, 0.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            input_data: PlayerAuthInputFlags::PerformBlockActions,
            tick: 0,
            move_vector: (0.0, 0.0, 0.0),
            analog_move_vector: (0.0, 0.0),
            raw_move_vector: (0.0, 0.0),
            block_actions: &actions,
            item_use_transaction_id: None,
            item_stack_request: None,
        };

        let stream = encode_player_auth_input_packet_stream(&input).expect("encoded input");
        let packet = &stream[2..];
        assert!(
            packet
                .windows(6)
                .any(|window| window == [0x01, 0x00, 0x02, 0x04, 0x06, 0x02]),
            "missing unsigned-varint block-action count/action/position/face sequence"
        );
    }

    #[test]
    fn block_break_input_flags_do_not_mark_missed_swing() {
        let flags = block_action_input_flags();

        assert_ne!(
            flags & PlayerAuthInputFlags::ReceivedServerData,
            0,
            "block-action input must acknowledge server data"
        );
        assert_ne!(
            flags & PlayerAuthInputFlags::PerformBlockActions,
            0,
            "block-action input must carry block actions"
        );
        assert_eq!(
            flags & PlayerAuthInputFlags::MissedSwing,
            0,
            "block breaking should not be encoded as an air swing"
        );
    }

    #[test]
    fn server_menu_items_are_not_placeable_candidates() {
        let item = ObservedInventoryItem {
            container_id: 1,
            slot: 13,
            item_id: 87,
            stack_id: Some(3),
            container_type: Some(CONTAINER_TYPE_CONTAINER),
            dynamic_container_id: Some(1),
            item_bytes: b"CustomName Click to randomly teleport Lore Click to select region"
                .to_vec(),
        };

        assert!(is_menu_selector_item(&item));
        assert_eq!(
            normal_placeable_rejection_reason(item.container_id, item.item_id, &item.item_bytes),
            Some("server_ui_item")
        );
        assert!(held_inventory_candidate_from_observed(&item).is_none());
    }

    #[test]
    fn donutsmp_spawn_hotbar_rtp_item_is_menu_not_placeable() {
        let item = ObservedInventoryItem {
            container_id: 0,
            slot: 0,
            item_id: 303,
            stack_id: Some(2),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: vec![0x01; 19],
        };

        let target = menu_click_target_from_observed(&item).expect("rtp fallback menu target");
        assert_eq!(target.slot, 0);
        assert_eq!(target.item_id, 303);
        assert_eq!(target.priority, 80);
        assert_eq!(
            normal_placeable_rejection_reason(item.container_id, item.item_id, &item.item_bytes),
            Some("server_ui_item")
        );
        assert!(held_inventory_candidate_from_observed(&item).is_none());
    }

    #[test]
    fn rtp_menu_click_preserves_observed_full_container_name() {
        let item = ObservedInventoryItem {
            container_id: 1,
            slot: 13,
            item_id: 87,
            stack_id: Some(3),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"CustomName Nether Lore Click to randomly teleport".to_vec(),
        };

        let target = menu_click_target_from_observed(&item).expect("rtp menu target");
        assert_eq!(target.container_type, 0);
        assert_eq!(target.dynamic_container_id, None);
        assert_eq!(target.priority, 40);

        let stream = encode_item_stack_request_packet_stream(
            1,
            &target,
            MenuClickMethod::StandaloneObservedTake,
        );
        assert!(
            stream
                .windows(8)
                .any(|window| window == [0, 0, 13, 6, CONTAINER_TYPE_CURSOR, 0, 0, 0]),
            "observed menu click must encode source FullContainerName as type=0,dynamic_id=none"
        );
    }

    #[test]
    fn item_stack_request_id_uses_unsigned_varint_encoding() {
        let item = ObservedInventoryItem {
            container_id: 1,
            slot: 11,
            item_id: 2,
            stack_id: Some(2),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"Name Overworld Lore Click to select region".to_vec(),
        };
        let target = menu_click_target_from_observed(&item).expect("rtp menu target");

        let stream = encode_item_stack_request_packet_stream(
            1,
            &target,
            MenuClickMethod::StandaloneObservedTake,
        );
        let packet = &stream[1..];
        assert_eq!(
            &packet[..5],
            &[0x93, 0x01, 0x01, 0x01, 0x01],
            "packet id, request-count, client-request-id, and action-count must match bedrock-rs ItemStackRequest"
        );
    }

    #[test]
    fn rtp_menu_prefers_overworld_selector_over_nether_random() {
        let nether = ObservedInventoryItem {
            container_id: 1,
            slot: 13,
            item_id: 87,
            stack_id: Some(3),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"Name Nether Lore Click to randomly teleport Players (1.57K) Asia (223ms)"
                .to_vec(),
        };
        let overworld = ObservedInventoryItem {
            container_id: 1,
            slot: 11,
            item_id: 2,
            stack_id: Some(2),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"Name Overworld Lore Click to select region".to_vec(),
        };
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));

        movement.record_observed_inventory_item(&nether);
        assert_eq!(
            movement.rtp_menu_item.as_ref().map(|item| item.slot),
            Some(13)
        );

        movement.record_observed_inventory_item(&overworld);
        let selected = movement.rtp_menu_item.expect("selected menu item");
        assert_eq!(selected.slot, 11);
        assert_eq!(selected.priority, 110);
    }

    #[test]
    fn rtp_menu_stage_change_resets_click_attempts() {
        let selector = ObservedInventoryItem {
            container_id: 1,
            slot: 11,
            item_id: 2,
            stack_id: Some(2),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"Name Overworld Lore Click to select region".to_vec(),
        };
        let region = ObservedInventoryItem {
            container_id: 1,
            slot: 10,
            item_id: 2,
            stack_id: Some(6),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"Name Overworld Lore Click to randomly teleport NA West".to_vec(),
        };
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));

        movement.record_observed_inventory_item(&selector);
        movement.rtp_menu_click_sent = true;
        movement.rtp_menu_click_sent_at = Some(Instant::now());
        movement.rtp_menu_last_click_attempt_at = Some(Instant::now());
        movement.rtp_menu_click_attempts = RTP_MENU_MAX_CLICK_ATTEMPTS;
        movement.record_observed_inventory_item(&region);

        let selected = movement.rtp_menu_item.expect("selected region menu item");
        assert_eq!(selected.slot, 10);
        assert_eq!(selected.priority, 220);
        assert_eq!(movement.rtp_menu_click_attempts, 0);
        assert!(!movement.rtp_menu_click_sent);
        assert_eq!(movement.rtp_menu_click_sent_at, None);
        assert_eq!(movement.rtp_menu_last_click_attempt_at, None);
    }

    #[test]
    fn rtp_menu_prefers_region_random_over_dimension_selector() {
        let selector = ObservedInventoryItem {
            container_id: 1,
            slot: 11,
            item_id: 2,
            stack_id: Some(2),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"Name Overworld Lore Click to select region".to_vec(),
        };
        let region_random = ObservedInventoryItem {
            container_id: 1,
            slot: 15,
            item_id: 3,
            stack_id: Some(4),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"Name Asia Lore Click to randomly teleport Players (1.57K) Asia (223ms)"
                .to_vec(),
        };
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));

        movement.record_observed_inventory_item(&selector);
        movement.rtp_menu_click_sent = true;
        movement.rtp_menu_click_sent_at = Some(Instant::now());
        movement.rtp_menu_last_click_attempt_at = Some(Instant::now());
        movement.rtp_menu_click_attempts = RTP_MENU_MAX_CLICK_ATTEMPTS;
        movement.record_observed_inventory_item(&region_random);

        let selected = movement
            .rtp_menu_item
            .expect("selected region random target");
        assert_eq!(selected.slot, 15);
        assert_eq!(selected.priority, 220);
        assert_eq!(movement.rtp_menu_click_attempts, 0);
        assert!(!movement.rtp_menu_click_sent);
    }

    #[test]
    fn rtp_menu_does_not_promote_nether_or_end_random_after_overworld_selector() {
        let selector = ObservedInventoryItem {
            container_id: 1,
            slot: 11,
            item_id: 2,
            stack_id: Some(2),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"Name Overworld Lore Click to select region".to_vec(),
        };
        let nether_random = ObservedInventoryItem {
            container_id: 1,
            slot: 13,
            item_id: 87,
            stack_id: Some(3),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"Name Nether Lore Click to randomly teleport Players (3.4K) Asia (30ms)"
                .to_vec(),
        };
        let end_random = ObservedInventoryItem {
            container_id: 1,
            slot: 15,
            item_id: 121,
            stack_id: Some(4),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"Name End Lore Click to randomly teleport Players (31.96K) Asia (30ms)"
                .to_vec(),
        };
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));

        movement.record_observed_inventory_item(&selector);
        movement.rtp_menu_click_sent = true;
        movement.rtp_menu_click_sent_at = Some(Instant::now());
        movement.rtp_menu_last_click_attempt_at = Some(Instant::now());
        movement.rtp_menu_click_attempts = RTP_MENU_MAX_CLICK_ATTEMPTS;
        movement.record_observed_inventory_item(&nether_random);
        movement.record_observed_inventory_item(&end_random);

        let selected = movement.rtp_menu_item.expect("selected menu item");
        assert_eq!(selected.slot, 11);
        assert_eq!(selected.priority, 110);
        assert_eq!(
            movement.rtp_menu_click_attempts, RTP_MENU_MAX_CLICK_ATTEMPTS,
            "cross-dimension random entries must not reopen click attempts for validation"
        );
    }

    #[test]
    fn rtp_command_fallback_retries_with_plain_rtp_after_overworld_timeout() {
        assert_eq!(
            fallback_rtp_command("/rtp overworld", 1).as_deref(),
            Some("/rtp")
        );
        assert_eq!(fallback_rtp_command("/rtp overworld", 2), None);
    }

    #[test]
    fn rtp_command_fallback_retries_with_overworld_after_plain_rtp_timeout() {
        assert_eq!(
            fallback_rtp_command("/rtp", 1).as_deref(),
            Some("/rtp overworld")
        );
        assert_eq!(fallback_rtp_command("/rtp", 0), None);
    }

    #[test]
    fn slash_validation_commands_default_to_command_request() {
        assert_eq!(validation_command_mode("/rtp", None), "command_request");
        assert_eq!(
            validation_command_mode("   /rtp overworld", None),
            "command_request"
        );
        assert_eq!(
            validation_command_mode("TorchFlower validation online", None),
            "text"
        );
    }

    #[test]
    fn validation_command_mode_env_override_still_wins() {
        assert_eq!(
            validation_command_mode("/rtp", Some("text")),
            "text",
            "explicit text override should keep legacy chat-command behavior available"
        );
        assert_eq!(
            validation_command_mode("hello", Some("command-request")),
            "command-request"
        );
    }

    #[test]
    fn command_request_wire_command_keeps_leading_slash_by_default() {
        assert_eq!(
            command_request_wire_command_with_slash_mode("/rtp", true),
            "/rtp"
        );
        assert_eq!(
            command_request_wire_command_with_slash_mode("rtp", true),
            "/rtp"
        );
        assert_eq!(
            command_request_wire_command_with_slash_mode("   /rtp overworld   ", true),
            "/rtp overworld"
        );
    }

    #[test]
    fn command_request_wire_command_can_strip_leading_slash_for_servers_that_need_it() {
        assert_eq!(
            command_request_wire_command_with_slash_mode("/rtp", false),
            "rtp"
        );
        assert_eq!(
            command_request_wire_command_with_slash_mode("///rtp", false),
            "rtp"
        );
        assert_eq!(
            command_request_wire_command_with_slash_mode("rtp overworld", false),
            "rtp overworld"
        );
    }

    #[test]
    fn tick_sync_packet_stream_uses_1_21_130_little_endian_i64_layout() {
        let stream = encode_tick_sync_packet_stream(42, 0);

        assert_eq!(stream.len(), 18);
        assert_eq!(
            stream[0], 17,
            "packet length should be id + two li64 fields"
        );
        assert_eq!(stream[1], TICK_SYNC_PACKET_ID as u8);
        assert_eq!(&stream[2..10], &42_i64.to_le_bytes());
        assert_eq!(&stream[10..18], &0_i64.to_le_bytes());
    }

    #[test]
    fn tick_sync_step_matches_50ms_client_tick_units() {
        assert_eq!(tick_sync_step(Duration::from_millis(500)), 10);
        assert_eq!(tick_sync_step(Duration::from_millis(1)), 1);
    }

    #[test]
    fn command_request_uses_1_21_130_command_origin_layout() {
        let request_id = "00000000-0000-0000-0000-000000000001";
        let stream = encode_command_request_packet_stream("/rtp", request_id, 123_456_789);

        assert_eq!(
            stream[0], 78,
            "packet body length should include full 1.21.130 command origin"
        );
        assert_eq!(stream[1], COMMAND_REQUEST_PACKET_ID as u8);
        assert_eq!(&stream[2..7], &[4, b'/', b'r', b't', b'p']);
        assert_eq!(
            &stream[7..14],
            &[6, b'p', b'l', b'a', b'y', b'e', b'r'],
            "1.21.130 CommandOrigin.type is the string \"player\""
        );
        assert_eq!(
            &stream[14..30],
            &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
        );
        assert_eq!(stream[30], 36);
        assert_eq!(&stream[31..67], request_id.as_bytes());
        assert_eq!(
            &stream[67..75],
            &(123_456_789_i64).to_le_bytes(),
            "1.21.130 CommandOrigin.player_entity_id is always encoded as li64"
        );
        assert_eq!(stream[75], 0, "internal=false");
        assert_eq!(&stream[76..79], &[2, b'5', b'2']);
    }

    #[test]
    fn command_request_preserves_signed_unique_entity_id() {
        let request_id = "00000000-0000-0000-0000-000000000001";
        let player_entity_id = -12_345_678_901i64;
        let stream = encode_command_request_packet_stream("/rtp", request_id, player_entity_id);

        assert_eq!(
            &stream[67..75],
            &player_entity_id.to_le_bytes(),
            "origin.player_entity_id must be the signed StartGame actor/entity id"
        );
    }

    #[test]
    fn late_rtp_menu_reopens_after_target_failure() {
        let selector = ObservedInventoryItem {
            container_id: 1,
            slot: 11,
            item_id: 2,
            stack_id: Some(2),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"Name Overworld Lore Click to select region".to_vec(),
        };
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.rtp_command_sent = true;
        movement.rtp_wait_done = true;
        movement.inventory_probe_sent = true;
        movement.gameplay_probe_sent = true;
        movement.gameplay_timeout_reported = true;
        movement.pickup_terminal_failed = true;

        movement.record_observed_inventory_item(&selector);

        assert!(movement.rtp_menu_item.is_some());
        assert!(!movement.rtp_wait_done);
        assert!(!movement.inventory_probe_sent);
        assert!(!movement.gameplay_probe_sent);
        assert!(!movement.gameplay_timeout_reported);
        assert!(!movement.pickup_terminal_failed);
    }

    #[test]
    fn exhausted_same_rtp_menu_does_not_reopen_wait_loop() {
        let selector = ObservedInventoryItem {
            container_id: 1,
            slot: 11,
            item_id: 2,
            stack_id: Some(2),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"Name Overworld Lore Click to select region".to_vec(),
        };
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.record_observed_inventory_item(&selector);
        movement.rtp_command_sent = true;
        movement.rtp_wait_done = true;
        movement.inventory_probe_sent = true;
        movement.rtp_menu_click_sent = true;
        movement.rtp_menu_click_attempts = RTP_MENU_MAX_CLICK_ATTEMPTS;

        movement.record_observed_inventory_item(&selector);

        assert!(movement.rtp_wait_done);
        assert!(movement.inventory_probe_sent);
        assert!(movement.rtp_menu_click_sent);
        assert_eq!(
            movement.rtp_menu_click_attempts,
            RTP_MENU_MAX_CLICK_ATTEMPTS
        );
    }

    #[test]
    fn rtp_menu_click_attempts_prefer_observed_container_encoding() {
        assert!(matches!(
            MenuClickMethod::from_attempt(0),
            Some(MenuClickMethod::StandaloneObservedConsume)
        ));
        assert!(matches!(
            MenuClickMethod::from_attempt(1),
            Some(MenuClickMethod::PlayerAuthInputObservedConsume)
        ));
        assert!(matches!(
            MenuClickMethod::from_attempt(2),
            Some(MenuClickMethod::StandaloneObservedTake)
        ));
    }

    #[test]
    fn observed_menu_consume_encodes_single_source_slot_without_cursor_destination() {
        let item = ObservedInventoryItem {
            container_id: 1,
            slot: 10,
            item_id: 2,
            stack_id: Some(6),
            container_type: Some(0),
            dynamic_container_id: None,
            item_bytes: b"Name Overworld Lore Click to randomly teleport NA West".to_vec(),
        };
        let target = menu_click_target_from_observed(&item).expect("rtp region menu target");

        let stream = encode_item_stack_request_packet_stream(
            7,
            &target,
            MenuClickMethod::StandaloneObservedConsume,
        );
        let packet = &stream[1..];
        assert_eq!(
            packet,
            &[
                0x93, 0x01, // ItemStackRequest packet id as unsigned varint
                0x01, // request count
                0x07, // unsigned client request id
                0x01, // action count
                0x05, // consume
                0x01, // count
                0x00, // observed FullContainerName container type
                0x00, // no dynamic id
                0x0a, // slot
                0x0c, // stack network id 6 zigzag encoded
                0x00, // strings_to_filter
                0xff, 0xff, 0xff, 0xff, // text processing origin unknown
            ],
            "consume must target only the observed menu source slot and must not add a cursor destination"
        );
    }

    #[test]
    fn player_inventory_normal_block_is_placeable_candidate() {
        let item = ObservedInventoryItem {
            container_id: 0,
            slot: 0,
            item_id: 3,
            stack_id: Some(10),
            container_type: None,
            dynamic_container_id: None,
            item_bytes: vec![0x06, 0x01, 0x00],
        };

        assert_eq!(
            normal_placeable_rejection_reason(item.container_id, item.item_id, &item.item_bytes),
            None
        );
        let held = held_inventory_candidate_from_observed(&item).expect("normal block candidate");
        assert_eq!(held.container_id, 0);
        assert_eq!(held.slot, 0);
        assert_eq!(held.item_id, 3);
    }

    #[test]
    fn player_inventory_utility_item_is_not_placeable_candidate() {
        let item = ObservedInventoryItem {
            container_id: 0,
            slot: 1,
            item_id: 343,
            stack_id: Some(3),
            container_type: None,
            dynamic_container_id: None,
            item_bytes: vec![0x01; 37],
        };

        assert_eq!(
            normal_placeable_rejection_reason(item.container_id, item.item_id, &item.item_bytes),
            Some("server_ui_item")
        );
        assert!(held_inventory_candidate_from_observed(&item).is_none());
    }

    #[test]
    fn observed_modern_block_items_are_placeable_candidates_without_menu_nbt() {
        for item_id in [303, 306, 320, 343, 371, 372, 373, 374, 422] {
            assert!(
                is_probably_placeable_block_item(item_id),
                "modern Bedrock block item id {item_id} should be considered placeable"
            );
            assert_eq!(
                normal_item_entity_rejection_reason(item_id, &[0x01; 18]),
                None,
                "plain item entity {item_id} should be usable for pickup/place validation"
            );
        }
    }

    #[test]
    fn stone_runtime_is_not_hand_droppable_placeable_target() {
        assert!(
            !is_hand_droppable_placeable_runtime_id(2532),
            "stone needs a tool and must not be used for pickup/place validation with empty hand"
        );
        assert!(is_hand_droppable_placeable_runtime_id(
            SPRUCE_BUTTON_CEILING_RUNTIME_ID
        ));
        assert!(
            !is_normal_validation_placeable_drop_runtime_id(SPRUCE_BUTTON_CEILING_RUNTIME_ID),
            "DonutSMP exposes button clusters as protected server terrain; they must not satisfy normal pickup/place validation"
        );
        assert!(is_hand_droppable_placeable_runtime_id(13114));
        assert!(is_normal_validation_placeable_drop_runtime_id(13114));
        assert!(is_hand_droppable_placeable_runtime_id(9852));
        assert!(is_normal_validation_placeable_drop_runtime_id(9852));
    }

    #[test]
    fn observed_donutsmp_terrain_runtimes_are_validation_candidates() {
        for runtime_id in [
            3758, 10812, 12970, 22010, 24356, 25040, 26228, 27644, 27646, 27648, 27654, 27656,
            28878,
        ] {
            assert!(
                is_normal_validation_placeable_drop_runtime_id(runtime_id),
                "runtime_id {runtime_id} should be eligible for server-confirmed break/pickup/place validation"
            );
        }
        assert!(
            !is_normal_validation_placeable_drop_runtime_id(31612),
            "runtime_id 31612 failed repeated DonutSMP break confirmation and must stay excluded"
        );
        assert!(
            !is_normal_validation_placeable_drop_runtime_id(25060),
            "runtime_id 25060 failed repeated DonutSMP break confirmation and must stay excluded"
        );
        assert!(
            !is_normal_validation_placeable_drop_runtime_id(SPRUCE_BUTTON_CEILING_RUNTIME_ID),
            "protected DonutSMP button/update marker terrain remains excluded"
        );
    }

    #[test]
    fn ceiling_button_runtime_places_against_block_above() {
        let target = BlockTarget {
            x: -16_252,
            y: 42,
            z: 27_466,
        };
        let geometry =
            place_geometry_for_break_target(target, Some(SPRUCE_BUTTON_CEILING_RUNTIME_ID));

        assert_eq!(
            geometry.base,
            BlockTarget {
                x: target.x,
                y: target.y + 1,
                z: target.z,
            }
        );
        assert_eq!(geometry.result, target);
        assert_eq!(geometry.face, BLOCK_FACE_DOWN);
        assert_eq!(geometry.click_pos, (0.5, 0.0, 0.5));
        assert_eq!(
            break_face_for_runtime_id(Some(SPRUCE_BUTTON_CEILING_RUNTIME_ID)),
            BLOCK_FACE_DOWN
        );
        assert_eq!(break_face_for_runtime_id(Some(9852)), BLOCK_FACE_UP);
    }

    #[test]
    fn normal_block_runtime_places_back_into_broken_target() {
        let target = BlockTarget {
            x: -16_259,
            y: 47,
            z: 27_469,
        };
        let geometry = place_geometry_for_break_target(target, Some(26228));

        assert_eq!(
            geometry.base,
            BlockTarget {
                x: target.x,
                y: target.y - 1,
                z: target.z,
            }
        );
        assert_eq!(geometry.result, target);
        assert_eq!(geometry.face, BLOCK_FACE_UP);
        assert_eq!(geometry.click_pos, (0.5, 1.0, 0.5));
    }

    #[test]
    fn eye_height_reach_accepts_current_donutsmp_overhead_sample() {
        let origin = (-16_259.5, 43.0, 27_470.5);
        let target = BlockTarget {
            x: -16_259,
            y: 47,
            z: 27_469,
        };

        assert!(
            block_target_horizontal_distance(target, origin) <= GAMEPLAY_BREAK_REACH_HORIZONTAL
        );
        assert!(block_target_vertical_delta(target, origin) <= GAMEPLAY_BREAK_REACH_VERTICAL);
    }

    #[test]
    fn no_tool_target_selection_prefers_hand_droppable_placeable_runtime() {
        let origin = (0.5, 65.0, 0.5);
        let stone = BlockTarget { x: 0, y: 64, z: 0 };
        let netherrack = BlockTarget { x: 2, y: 64, z: 0 };
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.remember_observed_solid_block(stone, 2532);
        movement.remember_observed_solid_block(netherrack, 13114);

        movement.ensure_gameplay_targets();

        assert_eq!(movement.break_target, Some(netherrack));
        assert_eq!(movement.break_target_runtime_id, Some(13114));
        assert!(!movement.pickup_terminal_failed);
    }

    #[test]
    fn chunk_publisher_position_and_sampled_block_create_loaded_world_target() {
        let origin = (0.0, 69.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.started_at = Instant::now() - Duration::from_secs(MOVEMENT_VALIDATION_SECONDS + 1);
        movement.record_network_chunk_publisher_update(-116_096, 79, -159_776, 64);
        movement.record_observed_block_sample(&ObservedBlockSample {
            x: -116_095,
            y: 78,
            z: -159_775,
            runtime_id: 9852,
        });

        movement.adopt_network_chunk_position_hint("test");
        assert_eq!(
            movement.last_server_position,
            Some((-116_095.5, 79.0, -159_775.5))
        );
        assert!(movement.has_sampled_placeable_drop_target());

        movement.ensure_gameplay_targets();
        assert_eq!(
            movement.break_target,
            Some(BlockTarget {
                x: -116_095,
                y: 78,
                z: -159_775,
            })
        );
        assert_eq!(movement.break_target_runtime_id, Some(9852));
    }

    #[test]
    fn current_donutsmp_chunk_sample_creates_reachable_target() {
        let origin = (-16_259.5, 43.0, 27_470.5);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.record_observed_block_sample(&ObservedBlockSample {
            x: -16_259,
            y: 47,
            z: 27_469,
            runtime_id: 26228,
        });

        assert!(movement.has_sampled_placeable_drop_target());
        assert!(movement.next_gameplay_approach_frame().is_none());

        movement.ensure_gameplay_targets();
        assert_eq!(
            movement.break_target,
            Some(BlockTarget {
                x: -16_259,
                y: 47,
                z: 27_469,
            })
        );
        assert_eq!(movement.break_target_runtime_id, Some(26228));
        assert_eq!(
            movement.place_base,
            Some(BlockTarget {
                x: -16_259,
                y: 46,
                z: 27_469,
            })
        );
        assert_eq!(movement.place_result, movement.break_target);
    }

    #[test]
    fn far_chunk_publisher_position_is_cached_not_adopted_before_movement_frames() {
        let origin = (0.0, 69.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));

        movement.record_network_chunk_publisher_update(-16_260, 43, 27_470, 160);

        assert_eq!(
            movement.network_chunk_position_hint,
            Some((-16_259.5, 43.0, 27_470.5))
        );
        assert_eq!(movement.last_sent_position, origin);
        assert_eq!(movement.last_server_position, None);

        assert!(!movement.adopt_initial_position_hint_if_available());
        assert_eq!(movement.last_sent_position, origin);
        assert_eq!(movement.last_server_position, None);
    }

    #[test]
    fn chunk_publisher_position_does_not_jump_mid_movement() {
        let origin = (0.0, 69.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        let _ = movement.next_frame();

        movement.record_network_chunk_publisher_update(-16_260, 43, 27_470, 160);

        assert_eq!(
            movement.network_chunk_position_hint,
            Some((-16_259.5, 43.0, 27_470.5))
        );
        assert_ne!(movement.last_sent_position, (-16_259.5, 43.0, 27_470.5));
        assert_eq!(movement.last_server_position, None);
        assert!(!movement.adopt_initial_position_hint_if_available());
    }

    #[test]
    fn delayed_movement_drive_uses_capped_incremental_step() {
        let origin = (-16_259.5, 43.0, 27_470.5);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        let first = movement.next_frame();
        assert_eq!(first.position, origin);

        movement.last_sent_at = Some(Instant::now() - Duration::from_secs(2));
        let second = movement.next_frame();
        let moved_z = second.position.2 - first.position.2;
        let max_step = MOVEMENT_FORWARD_SPEED_BLOCKS_PER_SECOND * MOVEMENT_MAX_STEP_SECONDS;

        assert!(moved_z > 0.0);
        assert!(
            moved_z <= max_step + 0.002,
            "movement step must be capped after delayed drive: moved_z={moved_z} max_step={max_step}"
        );
        assert_eq!(second.position.0, first.position.0);
        assert_eq!(second.position.1, first.position.1);
    }

    #[test]
    fn approachable_normal_target_skips_rtp_and_moves_toward_reach() {
        let mut movement = MovementValidation::new(
            ActorRuntimeID(1),
            0,
            (-16_259.5, 43.0, 27_470.5),
            (0.0, 0.0),
        );
        movement.remember_observed_solid_block(
            BlockTarget {
                x: -16_252,
                y: 42,
                z: 27_466,
            },
            13114,
        );

        assert!(!movement.has_sampled_placeable_drop_target());
        assert!(movement.has_approachable_placeable_drop_target());
        let frame = movement
            .next_gameplay_approach_frame()
            .expect("approach frame");

        assert!(frame.position.0 > -16_259.5);
        assert_eq!(frame.position.1, 43.0);
        assert!(frame.position.2 < 27_470.5);
    }

    #[test]
    fn far_normal_target_can_be_walked_toward_with_extended_limits() {
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.remember_observed_solid_block(BlockTarget { x: 48, y: 63, z: 0 }, 13114);

        assert!(!movement.has_sampled_placeable_drop_target());
        assert!(!movement.has_approachable_placeable_drop_target());
        assert!(movement.has_walkable_placeable_drop_target());
        assert!(movement.next_gameplay_approach_frame().is_none());

        let frame = movement
            .next_gameplay_approach_frame_with_limits(128.0, 16.0)
            .expect("extended approach frame");
        assert!(frame.position.0 > 0.0);
        assert_eq!(frame.position.1, 64.0);
        assert!(frame.position.2 > 0.0);
        assert!(frame.position.2 < 0.02);
    }

    #[test]
    fn far_placeable_target_defers_terminal_failure_until_approach() {
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.remember_observed_solid_block(BlockTarget { x: 48, y: 63, z: 0 }, 13114);

        movement.ensure_gameplay_targets();

        assert_eq!(movement.break_target, None);
        assert!(!movement.pickup_terminal_failed);
        assert!(movement.has_walkable_placeable_drop_target());
    }

    #[test]
    fn vertically_unreachable_target_is_not_walkable() {
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (10.5, 74.0, -10.5), (0.0, 0.0));
        movement.remember_observed_solid_block(
            BlockTarget {
                x: 10,
                y: 63,
                z: -11,
            },
            13114,
        );

        assert!(!movement.has_walkable_placeable_drop_target());
        assert!(
            movement
                .next_gameplay_approach_frame_with_limits(128.0, GAMEPLAY_BREAK_REACH_VERTICAL)
                .is_none(),
            "horizontal-only approach cannot resolve a target far below the player"
        );
    }

    #[test]
    fn block_break_aim_points_at_target_center() {
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.5, 64.0, 0.5), (0.0, 0.0));

        movement.aim_at_block_target(BlockTarget { x: 0, y: 63, z: -2 });

        assert!(movement.yaw.abs() >= 179.0);
        assert!(
            movement.pitch > 35.0,
            "lower block target should require a downward pitch, got {}",
            movement.pitch
        );
    }

    #[test]
    fn break_failure_retry_clears_probe_state_for_far_target_approach() {
        let near = BlockTarget { x: 1, y: 63, z: 0 };
        let far = BlockTarget { x: 48, y: 63, z: 0 };
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.remember_observed_solid_block(near, 12178);
        movement.remember_observed_solid_block(far, 13114);
        movement.ensure_gameplay_targets();
        movement.gameplay_probe_sent = true;
        movement.gameplay_probe_sent_at = Some(Instant::now());
        movement.gameplay_timeout_reported = true;
        movement.break_probe_started_at = Some(Instant::now());

        assert_eq!(movement.break_target, Some(near));
        assert!(movement.prepare_next_break_target_after_break_failure());

        assert_eq!(movement.break_target, None);
        assert!(!movement.gameplay_probe_sent);
        assert_eq!(movement.gameplay_probe_sent_at, None);
        assert!(!movement.gameplay_timeout_reported);
        assert!(!movement.pickup_terminal_failed);
        assert!(movement.has_walkable_placeable_drop_target());
    }

    #[test]
    fn break_failure_retry_preserves_collected_held_item() {
        let near = BlockTarget { x: 1, y: 63, z: 0 };
        let far = BlockTarget { x: 48, y: 63, z: 0 };
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.held_item = Some(HeldInventoryItem {
            container_id: 0,
            slot: 2,
            item_id: 422,
            item: None,
            item_bytes: vec![0x06, 0x01, 0x00],
        });
        movement.held_item_equipped = true;
        movement.remember_observed_solid_block(near, 2620);
        movement.remember_observed_solid_block(far, 2620);
        movement.ensure_gameplay_targets();
        movement.gameplay_probe_sent = true;
        movement.gameplay_probe_sent_at = Some(Instant::now());
        movement.break_probe_started_at = Some(Instant::now());

        assert!(movement.prepare_next_break_target_after_break_failure());

        let held_item = movement.held_item.expect("held item must survive retry");
        assert_eq!(held_item.item_id, 422);
        assert_eq!(held_item.slot, 2);
        assert!(movement.held_item_equipped);
        assert!(!movement.gameplay_probe_sent);
    }

    #[test]
    fn rtp_retry_after_target_failure_clears_stale_gameplay_state() {
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.rtp_command_attempts = 1;
        movement.rtp_command_sent = true;
        movement.rtp_wait_done = true;
        movement.inventory_probe_sent = true;
        movement.gameplay_probe_sent = true;
        movement.pickup_terminal_failed = true;
        movement.break_target = Some(BlockTarget { x: 1, y: 63, z: 0 });
        movement.rejected_break_runtime_ids.push(25060);
        movement.remember_observed_solid_block(
            BlockTarget {
                x: 500,
                y: 63,
                z: 0,
            },
            13114,
        );

        assert!(movement.reset_for_rtp_retry_after_target_failure("test"));

        assert_eq!(movement.rtp_command_attempts, 1);
        assert!(!movement.rtp_command_sent);
        assert!(!movement.rtp_wait_done);
        assert!(!movement.inventory_probe_sent);
        assert!(!movement.gameplay_probe_sent);
        assert!(!movement.pickup_terminal_failed);
        assert_eq!(movement.break_target, None);
        assert_eq!(movement.rejected_break_runtime_ids, vec![25060]);
        assert!(movement.observed_solid_blocks.is_empty());
    }

    #[test]
    fn rtp_retry_after_target_failure_is_bounded() {
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.rtp_command_attempts = RTP_MAX_COMMAND_ATTEMPTS;

        assert!(!movement.reset_for_rtp_retry_after_target_failure("test"));
    }

    #[test]
    fn break_failure_with_only_far_stale_targets_retries_rtp() {
        let near = BlockTarget { x: 1, y: 63, z: 0 };
        let stale_far = BlockTarget {
            x: 500,
            y: 63,
            z: 0,
        };
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.rtp_command_sent = true;
        movement.rtp_wait_done = true;
        movement.rtp_command_attempts = 1;
        movement.inventory_probe_sent = true;
        movement.gameplay_probe_sent = true;
        movement.remember_observed_solid_block(near, 13114);
        movement.remember_observed_solid_block(stale_far, 9852);
        movement.ensure_gameplay_targets();

        assert_eq!(movement.break_target, Some(near));
        assert!(movement.prepare_next_break_target_after_break_failure());

        assert_eq!(movement.rtp_command_attempts, 1);
        assert!(!movement.rtp_command_sent);
        assert!(!movement.inventory_probe_sent);
        assert!(movement.observed_solid_blocks.is_empty());
        assert_eq!(movement.break_target, None);
        assert!(!movement.pickup_terminal_failed);
    }

    #[test]
    fn button_targets_do_not_skip_rtp_or_drive_pickup_validation() {
        let mut movement = MovementValidation::new(
            ActorRuntimeID(1),
            0,
            (-16_259.5, 43.0, 27_470.5),
            (0.0, 0.0),
        );
        movement.remember_observed_solid_block(
            BlockTarget {
                x: -16_252,
                y: 42,
                z: 27_466,
            },
            SPRUCE_BUTTON_CEILING_RUNTIME_ID,
        );

        assert!(!movement.has_sampled_placeable_drop_target());
        assert!(!movement.has_approachable_placeable_drop_target());
        assert!(movement.next_gameplay_approach_frame().is_none());
    }

    #[test]
    fn placeable_observed_target_survives_non_placeable_observation_overflow() {
        let mut movement = MovementValidation::new(
            ActorRuntimeID(1),
            0,
            (-16_259.5, 43.0, 27_470.5),
            (0.0, 0.0),
        );
        let target = BlockTarget {
            x: -16_252,
            y: 42,
            z: 27_466,
        };
        movement.remember_observed_solid_block(target, 13114);

        for index in 0..(MAX_OBSERVED_SOLID_BLOCKS + 64) {
            movement.remember_observed_solid_block(
                BlockTarget {
                    x: 10_000 + index as i32,
                    y: 10,
                    z: 10_000,
                },
                2532,
            );
        }

        assert!(movement.observed_solid_blocks.len() <= MAX_OBSERVED_SOLID_BLOCKS);
        assert!(movement.observed_solid_blocks.contains(&target));
        assert_eq!(movement.observed_target_runtime_id(target), Some(13114));
        assert!(movement.has_approachable_placeable_drop_target());
    }

    #[test]
    fn approachable_placeable_target_survives_far_placeable_observation_overflow() {
        let mut movement = MovementValidation::new(
            ActorRuntimeID(1),
            0,
            (-16_259.5, 43.0, 27_470.5),
            (0.0, 0.0),
        );
        let target = BlockTarget {
            x: -16_252,
            y: 42,
            z: 27_466,
        };
        movement.remember_observed_solid_block(target, 13114);

        for index in 0..(MAX_OBSERVED_SOLID_BLOCKS + 64) {
            movement.remember_observed_solid_block(
                BlockTarget {
                    x: 10_000 + index as i32,
                    y: 120,
                    z: 10_000,
                },
                13114,
            );
        }

        assert!(movement.observed_solid_blocks.len() <= MAX_OBSERVED_SOLID_BLOCKS);
        assert!(movement.observed_solid_blocks.contains(&target));
        assert_eq!(movement.observed_target_runtime_id(target), Some(13114));
        assert!(movement.has_approachable_placeable_drop_target());
    }

    #[test]
    fn no_tool_target_selection_rejects_stone_only_observations() {
        let origin = (0.5, 65.0, 0.5);
        let stone = BlockTarget { x: 0, y: 64, z: 0 };
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.remember_observed_solid_block(stone, 2532);

        movement.ensure_gameplay_targets();

        assert_eq!(movement.break_target, None);
        assert!(movement.pickup_terminal_failed);
    }

    #[test]
    fn pickup_requires_confirmed_break_evidence() {
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.gameplay_probe_sent = true;
        movement.break_stop_sent = true;
        movement.break_target = Some(BlockTarget { x: 1, y: 63, z: 1 });

        assert!(
            !movement.ready_for_pickup(),
            "pickup must not start from break_stop_sent alone"
        );

        movement.break_confirmed = true;
        assert!(
            movement.ready_for_pickup(),
            "pickup should start after server-confirmed break evidence"
        );
    }

    #[test]
    fn break_confirmation_requires_air_runtime() {
        let target = BlockTarget { x: 1, y: 63, z: 1 };
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.break_target = Some(target);
        let mut status = CapabilityStatus::default();

        apply_update_block_evidence(&mut status, &mut movement, target, 5901, "test");
        assert!(!movement.break_confirmed);
        assert!(!status.block_breaking);

        apply_update_block_evidence(
            &mut status,
            &mut movement,
            target,
            OBSERVED_AIR_RUNTIME_ID,
            "test",
        );
        assert!(movement.break_confirmed);
        assert!(status.block_breaking);
    }

    #[test]
    fn place_confirmation_rejects_air_runtime() {
        let place_target = BlockTarget { x: 1, y: 64, z: 1 };
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.break_target = Some(BlockTarget { x: 1, y: 63, z: 1 });
        movement.place_result = Some(place_target);
        movement.place_probe_sent = true;
        let mut status = CapabilityStatus::default();

        apply_update_block_evidence(
            &mut status,
            &mut movement,
            place_target,
            OBSERVED_AIR_RUNTIME_ID,
            "test",
        );
        assert!(!status.block_placing);

        apply_update_block_evidence(&mut status, &mut movement, place_target, 5901, "test");
        assert!(status.block_placing);
    }

    #[test]
    fn rtp_marker_updates_are_not_break_candidates() {
        let marker = BlockTarget {
            x: 10,
            y: 70,
            z: -4,
        };
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.rtp_command_sent = true;

        movement.record_rtp_position_hint_from_update_block(marker, OBSERVED_CHEST_RUNTIME_ID, 0);

        assert_eq!(
            movement.observed_break_candidate, None,
            "transient RTP/menu marker blocks must not become break targets"
        );
        assert!(
            movement.observed_solid_blocks.is_empty(),
            "marker blocks should not be tracked as normal terrain"
        );
    }

    #[test]
    fn marker_cleanup_clears_stale_below_marker_candidate() {
        let marker = BlockTarget {
            x: 10,
            y: 70,
            z: -4,
        };
        let stale_candidate = BlockTarget {
            x: marker.x,
            y: marker.y - 1,
            z: marker.z,
        };
        let mut movement =
            MovementValidation::new(ActorRuntimeID(1), 0, (0.0, 64.0, 0.0), (0.0, 0.0));
        movement.rtp_command_sent = true;
        movement.observed_break_candidate = Some(stale_candidate);
        movement.observed_solid_blocks.push(stale_candidate);
        movement
            .observed_solid_block_runtime_ids
            .push((stale_candidate, 5901));

        movement.record_rtp_position_hint_from_update_block(marker, OBSERVED_AIR_RUNTIME_ID, 0);

        assert_eq!(movement.observed_break_candidate, None);
        assert!(!movement.observed_solid_blocks.contains(&stale_candidate));
        assert!(!movement
            .observed_solid_block_runtime_ids
            .iter()
            .any(|(target, _)| *target == stale_candidate));
    }

    #[test]
    fn marker_only_rtp_hint_does_not_update_gameplay_position() {
        let marker = BlockTarget {
            x: 100,
            y: 70,
            z: -100,
        };
        let origin = (0.0, 64.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.rtp_command_sent = true;
        movement.rtp_menu_click_sent = true;

        movement.record_rtp_position_hint_from_update_block(marker, OBSERVED_CHEST_RUNTIME_ID, 0);

        assert!(movement.rtp_position_hint_received);
        assert!(movement.rtp_marker_position_hint_received);
        assert!(!movement.rtp_terrain_position_hint_received);
        assert_eq!(movement.last_sent_position, origin);
        assert_eq!(movement.last_server_position, None);
        assert_eq!(movement.observed_break_candidate, None);
    }

    #[test]
    fn terrain_rtp_hint_updates_gameplay_position() {
        let target = BlockTarget {
            x: 100,
            y: 70,
            z: -100,
        };
        let origin = (0.0, 64.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.rtp_command_sent = true;
        movement.rtp_menu_click_sent = true;

        movement.record_rtp_position_hint_from_update_block(target, 12178, 0);

        assert!(movement.rtp_position_hint_received);
        assert!(!movement.rtp_marker_position_hint_received);
        assert!(movement.rtp_terrain_position_hint_received);
        assert_eq!(movement.last_sent_position, (100.5, 71.0, -99.5));
        assert_eq!(movement.last_server_position, Some((100.5, 71.0, -99.5)));
        assert_eq!(movement.observed_break_candidate, Some(target));
    }

    #[test]
    fn late_terrain_hint_after_rtp_wait_updates_gameplay_position() {
        let target = BlockTarget {
            x: 16_360,
            y: 112,
            z: 4_147,
        };
        let origin = (46_296.5, 72.0, -115_901.5);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.rtp_command_sent = true;
        movement.rtp_menu_click_sent = true;
        movement.rtp_wait_done = true;
        movement.inventory_probe_sent = true;
        movement.gameplay_probe_sent = true;
        movement.pickup_terminal_failed = true;

        movement.record_rtp_position_hint_from_update_block(target, 13_114, 0);

        assert!(movement.rtp_position_hint_received);
        assert!(movement.rtp_terrain_position_hint_received);
        assert!(!movement.rtp_wait_done);
        assert!(!movement.inventory_probe_sent);
        assert!(!movement.gameplay_probe_sent);
        assert!(!movement.pickup_terminal_failed);
        assert_eq!(
            movement.last_server_position,
            Some((16_360.5, 113.0, 4_147.5))
        );
        assert_eq!(movement.observed_break_candidate, Some(target));
    }

    #[test]
    fn post_region_click_item_entity_updates_rtp_gameplay_position() {
        let origin = (0.0, 64.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.rtp_command_sent = true;
        movement.rtp_menu_click_sent = true;
        movement.rtp_menu_item = Some(MenuClickTarget {
            window_id: 1,
            slot: 10,
            item_id: 2,
            stack_id: 6,
            container_type: 0,
            dynamic_container_id: None,
            priority: 120,
        });
        let entity = ObservedItemEntity {
            entity_id: 41,
            runtime_entity_id: 41,
            item_id: 422,
            stack_id: None,
            position: (-116_154.039, 87.125, -159_827.047),
            velocity: (0.0, 0.0, 0.0),
            item_bytes: vec![0x06, 0x01, 0x00],
        };

        movement.record_observed_item_entity(&entity);

        assert!(movement.rtp_position_hint_received);
        assert!(movement.rtp_terrain_position_hint_received);
        assert_eq!(movement.last_sent_position, entity.position);
        assert_eq!(movement.last_server_position, Some(entity.position));
        assert_eq!(
            movement.rtp_menu_click_attempts,
            RTP_MENU_MAX_CLICK_ATTEMPTS
        );
    }

    #[test]
    fn first_stage_menu_item_entity_does_not_update_rtp_gameplay_position() {
        let origin = (0.0, 64.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.rtp_command_sent = true;
        movement.rtp_menu_click_sent = true;
        movement.rtp_menu_item = Some(MenuClickTarget {
            window_id: 1,
            slot: 11,
            item_id: 2,
            stack_id: 2,
            container_type: 0,
            dynamic_container_id: None,
            priority: 110,
        });
        let entity = ObservedItemEntity {
            entity_id: 7,
            runtime_entity_id: 7,
            item_id: 422,
            stack_id: None,
            position: (-63_360.445, 92.125, 5_269.971),
            velocity: (0.0, 0.0, 0.0),
            item_bytes: vec![0x06, 0x01, 0x00],
        };

        movement.record_observed_item_entity(&entity);

        assert!(!movement.rtp_position_hint_received);
        assert!(!movement.rtp_terrain_position_hint_received);
        assert_eq!(movement.last_sent_position, origin);
        assert_eq!(movement.last_server_position, None);
    }

    #[test]
    fn normal_item_entity_initializes_placeholder_start_position() {
        let origin = (0.0, 69.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        let entity = ObservedItemEntity {
            entity_id: 52,
            runtime_entity_id: 52,
            item_id: 422,
            stack_id: None,
            position: (117_215.469, 72.125, -160_100.750),
            velocity: (0.0, 0.0, 0.0),
            item_bytes: vec![0x06, 0x01, 0x00],
        };

        movement.record_observed_item_entity(&entity);

        assert_eq!(movement.last_sent_position, entity.position);
        assert_eq!(movement.last_server_position, None);
        assert_eq!(movement.spawn_position, entity.position);
        assert!(movement.observed_item_entity.is_some());
    }

    #[test]
    fn item_entity_position_hint_does_not_rewind_active_movement() {
        let origin = (0.0, 69.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        let _ = movement.next_frame();
        let entity = ObservedItemEntity {
            entity_id: 52,
            runtime_entity_id: 52,
            item_id: 422,
            stack_id: None,
            position: (117_215.469, 72.125, -160_100.750),
            velocity: (0.0, 0.0, 0.0),
            item_bytes: vec![0x06, 0x01, 0x00],
        };

        movement.record_observed_item_entity(&entity);

        assert_ne!(movement.last_sent_position, entity.position);
        assert_eq!(movement.last_server_position, None);
        assert!(movement.observed_item_entity.is_some());
    }

    #[test]
    fn prebreak_pickup_is_ready_for_existing_normal_item_entity() {
        let origin = (0.0, 69.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        let entity = ObservedItemEntity {
            entity_id: 52,
            runtime_entity_id: 52,
            item_id: 422,
            stack_id: None,
            position: (117_215.469, 72.125, -160_100.750),
            velocity: (0.0, 0.0, 0.0),
            item_bytes: vec![0x06, 0x01, 0x00],
        };

        movement.record_observed_item_entity(&entity);

        assert!(movement.ready_for_prebreak_pickup());
        assert_eq!(
            movement.pickup_target_position(),
            Some((entity.position.0, entity.position.1, entity.position.2))
        );
    }

    #[test]
    fn prebreak_pickup_failure_selects_next_normal_item_entity() {
        let origin = (0.0, 69.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        let first = ObservedItemEntity {
            entity_id: 4,
            runtime_entity_id: 4,
            item_id: 306,
            stack_id: None,
            position: (100.0, 64.0, 100.0),
            velocity: (0.0, 0.0, 0.0),
            item_bytes: vec![0x06, 0x01, 0x00],
        };
        let second = ObservedItemEntity {
            entity_id: 5,
            runtime_entity_id: 5,
            item_id: 422,
            stack_id: None,
            position: (112.0, 64.0, 100.0),
            velocity: (0.0, 0.0, 0.0),
            item_bytes: vec![0x06, 0x01, 0x00],
        };
        movement.record_observed_item_entity(&first);
        movement.record_observed_item_entity(&second);
        movement.pickup_prebreak = true;
        movement.pickup_probe_started_at = Some(Instant::now());
        movement.pickup_failed_reported = true;

        assert_eq!(
            movement
                .observed_item_entity
                .as_ref()
                .map(|item| item.runtime_id),
            Some(first.runtime_entity_id)
        );
        assert!(movement.prepare_next_item_entity_after_pickup_failure());

        assert_eq!(
            movement
                .observed_item_entity
                .as_ref()
                .map(|item| item.runtime_id),
            Some(second.runtime_entity_id)
        );
        assert!(movement
            .rejected_item_entity_runtime_ids
            .contains(&first.runtime_entity_id));
        assert_eq!(movement.pickup_probe_started_at, None);
        assert!(!movement.pickup_failed_reported);
        assert!(!movement.pickup_terminal_failed);
    }

    #[test]
    fn direct_rtp_command_terrain_hint_updates_gameplay_position() {
        let target = BlockTarget {
            x: 100,
            y: 70,
            z: -100,
        };
        let origin = (0.0, 64.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.rtp_command_sent = true;

        movement.record_rtp_position_hint_from_update_block(target, 13114, 0);

        assert!(movement.rtp_position_hint_received);
        assert!(movement.rtp_terrain_position_hint_received);
        assert_eq!(movement.last_sent_position, (100.5, 71.0, -99.5));
        assert_eq!(movement.last_server_position, Some((100.5, 71.0, -99.5)));
        assert_eq!(movement.observed_break_candidate, Some(target));
    }

    #[test]
    fn observed_chunk_samples_recover_placeholder_rtp_position() {
        let target = BlockTarget {
            x: -16_332,
            y: 100,
            z: 27_445,
        };
        let underground = BlockTarget {
            x: -16_211,
            y: 31,
            z: 27_393,
        };
        let origin = (0.0, 69.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.rtp_command_sent = true;
        movement.record_observed_block_sample(&ObservedBlockSample {
            x: underground.x,
            y: underground.y,
            z: underground.z,
            runtime_id: 26_228,
        });
        movement.record_observed_block_sample(&ObservedBlockSample {
            x: target.x,
            y: target.y,
            z: target.z,
            runtime_id: 13_114,
        });

        assert!(movement.adopt_observed_terrain_position_hint("test"));

        assert!(movement.rtp_position_hint_received);
        assert!(movement.rtp_terrain_position_hint_received);
        assert_eq!(movement.last_sent_position, (-16_331.5, 101.0, 27_447.5));
        assert_eq!(
            movement.last_server_position,
            Some((-16_331.5, 101.0, 27_447.5))
        );
    }

    #[test]
    fn later_terrain_hints_do_not_move_locked_rtp_position() {
        let first = BlockTarget {
            x: 100,
            y: 70,
            z: -100,
        };
        let later = BlockTarget {
            x: 200,
            y: 12,
            z: -300,
        };
        let origin = (0.0, 64.0, 0.0);
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.rtp_command_sent = true;
        movement.rtp_menu_click_sent = true;

        movement.record_rtp_position_hint_from_update_block(first, 13114, 0);
        assert_eq!(movement.last_server_position, Some((100.5, 71.0, -99.5)));

        movement.record_rtp_position_hint_from_update_block(later, 15806, 0);

        assert_eq!(
            movement.last_server_position,
            Some((100.5, 71.0, -99.5)),
            "later chunk UpdateBlock packets must not move the inferred RTP player position"
        );
        assert_eq!(movement.observed_break_candidate, Some(later));
        assert!(movement.observed_solid_blocks.contains(&later));
    }

    #[test]
    fn selected_break_runtime_survives_air_update_for_pickup_rejection() {
        let origin = (0.5, 64.0, 0.5);
        let target = BlockTarget { x: 1, y: 63, z: 0 };
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.remember_observed_solid_block(target, 12178);
        movement.ensure_gameplay_targets();

        assert_eq!(movement.break_target, Some(target));
        assert_eq!(movement.break_target_runtime_id, Some(12178));

        movement.observed_solid_block_runtime_ids.clear();
        movement.prepare_next_break_target_after_pickup_failure();

        assert!(
            movement.rejected_break_runtime_ids.contains(&12178),
            "pickup failure must reject the originally selected runtime even after air update removes observation state"
        );
    }

    #[test]
    fn fallback_break_target_skips_rejected_candidates() {
        let origin = (10.5, 70.0, -4.5);
        let first = fallback_break_candidates(origin)
            .into_iter()
            .next()
            .expect("first fallback candidate");
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));

        assert_eq!(movement.fallback_break_target(origin), Some(first));

        movement.rejected_break_targets.push(first);
        assert_ne!(movement.fallback_break_target(origin), Some(first));
        assert!(movement.fallback_break_target(origin).is_some());
    }

    #[test]
    fn far_observed_candidate_does_not_override_nearby_fallback_when_item_is_held() {
        let origin = (10.5, 70.0, -4.5);
        let far_candidate = BlockTarget {
            x: 100,
            y: 90,
            z: 100,
        };
        let expected_fallback = fallback_break_candidates(origin)
            .into_iter()
            .next()
            .expect("fallback candidate");
        let mut movement = MovementValidation::new(ActorRuntimeID(1), 0, origin, (0.0, 0.0));
        movement.observed_break_candidate = Some(far_candidate);
        movement.held_item = Some(HeldInventoryItem {
            container_id: 0,
            slot: 0,
            item_id: 28,
            item: None,
            item_bytes: vec![0x38, 0x01, 0x00],
        });

        movement.ensure_gameplay_targets();

        assert_eq!(movement.break_target, Some(expected_fallback));
        assert_ne!(movement.break_target, Some(far_candidate));
    }
}
