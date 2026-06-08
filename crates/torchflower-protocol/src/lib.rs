use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use torchflower_protocol_core::*;

pub use torchflower_protocol_core::{BlockPosition, CoreError, Vector2f, Vector3f};

pub mod compat;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ProtocolVersion {
    V662 = 662,
    V766 = 766,
    V898 = 898,
    V975 = 975,
    #[default]
    Auto,
}

impl ProtocolVersion {
    pub fn to_u32(self) -> u32 {
        match self {
            Self::V662 => 662,
            Self::V766 => 766,
            Self::V898 => 898,
            Self::V975 => 975,
            Self::Auto => 898, // default fallback
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedPacket {
    pub id: u32,
    pub name: String,
    pub data: serde_json::Value,
    pub raw: Bytes,
}

pub struct PacketRegistry;

impl PacketRegistry {
    pub fn id_to_name(id: u32) -> Option<&'static str> {
        match id {
            0x01 => Some("login"),
            0x02 => Some("play_status"),
            0x03 => Some("server_to_client_handshake"),
            0x04 => Some("client_to_server_handshake"),
            0x05 => Some("disconnect"),
            0x06 => Some("resource_packs_info"),
            0x07 => Some("resource_pack_stack"),
            0x08 => Some("resource_pack_client_response"),
            0x09 => Some("text"),
            0x0b => Some("start_game"),
            0x13 => Some("move_player"),
            0x15 => Some("update_block"),
            0x19 => Some("level_event"),
            0x1e => Some("inventory_transaction"),
            0x1f => Some("mob_equipment"),
            0x2c => Some("animate"),
            0x2d => Some("respawn"),
            0x31 => Some("inventory_content"),
            0x32 => Some("inventory_slot"),
            0x45 => Some("request_chunk_radius"),
            0x4f => Some("command_output"),
            0x64 => Some("modal_form_request"),
            0x65 => Some("modal_form_response"),
            0x71 => Some("set_local_player_as_initialized"),
            0x73 => Some("network_stack_latency"),
            0x81 => Some("client_cache_status"),
            0x90 => Some("player_auth_input"),
            0x94 => Some("item_stack_response"),
            0xa1 => Some("correct_player_move_prediction"),
            0xc1 => Some("request_network_settings"),
            0xc2 => Some("network_settings"),
            _ => None,
        }
    }

    pub fn name_to_id(name: &str) -> Option<u32> {
        match name {
            "login" => Some(0x01),
            "play_status" => Some(0x02),
            "server_to_client_handshake" => Some(0x03),
            "client_to_server_handshake" => Some(0x04),
            "disconnect" => Some(0x05),
            "resource_packs_info" => Some(0x06),
            "resource_pack_stack" => Some(0x07),
            "resource_pack_client_response" => Some(0x08),
            "text" => Some(0x09),
            "start_game" => Some(0x0b),
            "move_player" => Some(0x13),
            "update_block" => Some(0x15),
            "level_event" => Some(0x19),
            "inventory_transaction" => Some(0x1e),
            "mob_equipment" => Some(0x1f),
            "animate" => Some(0x2c),
            "respawn" => Some(0x2d),
            "inventory_content" => Some(0x31),
            "inventory_slot" => Some(0x32),
            "request_chunk_radius" => Some(0x45),
            "command_output" => Some(0x4f),
            "modal_form_request" => Some(0x64),
            "modal_form_response" => Some(0x65),
            "set_local_player_as_initialized" => Some(0x71),
            "network_stack_latency" => Some(0x73),
            "client_cache_status" => Some(0x81),
            "player_auth_input" => Some(0x90),
            "item_stack_response" => Some(0x94),
            "correct_player_move_prediction" => Some(0xa1),
            "request_network_settings" => Some(0xc1),
            "network_settings" => Some(0xc2),
            _ => None,
        }
    }
}

// Packet Definitions

#[derive(Debug, Clone, PartialEq)]
pub enum Packet {
    RequestNetworkSettings(RequestNetworkSettingsPacket),
    NetworkSettings(NetworkSettingsPacket),
    Login(LoginPacket),
    PlayStatus(PlayStatusPacket),
    ServerToClientHandshake(ServerToClientHandshakePacket),
    ClientToServerHandshake(ClientToServerHandshakePacket),
    Disconnect(DisconnectPacket),
    ResourcePacksInfo(ResourcePacksInfoPacket),
    ResourcePackStack(ResourcePackStackPacket),
    ResourcePackClientResponse(ResourcePackClientResponsePacket),
    Text(TextPacket),
    StartGame(StartGamePacket),
    MovePlayer(MovePlayerPacket),
    UpdateBlock(UpdateBlockPacket),
    LevelEvent(LevelEventPacket),
    MobEquipment(MobEquipmentPacket),
    InventoryContent(InventoryContentPacket),
    InventorySlot(InventorySlotPacket),
    Animate(AnimatePacket),
    Respawn(RespawnPacket),
    CommandOutput(CommandOutputPacket),
    ModalFormRequest(ModalFormRequestPacket),
    ModalFormResponse(ModalFormResponsePacket),
    SetLocalPlayerAsInitialized(SetLocalPlayerAsInitializedPacket),
    NetworkStackLatency(NetworkStackLatencyPacket),
    ClientCacheStatus(ClientCacheStatusPacket),
    ItemStackResponse(ItemStackResponsePacket),
    CorrectPlayerMovePrediction(CorrectPlayerMovePredictionPacket),
    InventoryTransaction(InventoryTransactionPacket),
    RequestChunkRadius(RequestChunkRadiusPacket),
    Unknown { id: u32, payload: Bytes },
}

impl Packet {
    pub fn id(&self) -> u32 {
        match self {
            Self::RequestNetworkSettings(_) => 0xc1,
            Self::NetworkSettings(_) => 0xc2,
            Self::Login(_) => 0x01,
            Self::PlayStatus(_) => 0x02,
            Self::ServerToClientHandshake(_) => 0x03,
            Self::ClientToServerHandshake(_) => 0x04,
            Self::Disconnect(_) => 0x05,
            Self::ResourcePacksInfo(_) => 0x06,
            Self::ResourcePackStack(_) => 0x07,
            Self::ResourcePackClientResponse(_) => 0x08,
            Self::Text(_) => 0x09,
            Self::StartGame(_) => 0x0b,
            Self::MovePlayer(_) => 0x13,
            Self::UpdateBlock(_) => 0x15,
            Self::LevelEvent(_) => 0x19,
            Self::MobEquipment(_) => 0x1f,
            Self::InventoryContent(_) => 0x31,
            Self::InventorySlot(_) => 0x32,
            Self::Animate(_) => 0x2c,
            Self::Respawn(_) => 0x2d,
            Self::CommandOutput(_) => 0x4f,
            Self::ModalFormRequest(_) => 0x64,
            Self::ModalFormResponse(_) => 0x65,
            Self::SetLocalPlayerAsInitialized(_) => 0x71,
            Self::NetworkStackLatency(_) => 0x73,
            Self::ClientCacheStatus(_) => 0x81,
            Self::ItemStackResponse(_) => 0x94,
            Self::CorrectPlayerMovePrediction(_) => 0xa1,
            Self::InventoryTransaction(_) => 0x1e,
            Self::RequestChunkRadius(_) => 0x45,
            Self::Unknown { id, .. } => *id,
        }
    }

    pub fn encode(&self, version: ProtocolVersion) -> Result<Bytes, CoreError> {
        let mut buf = BytesMut::new();
        match self {
            Self::RequestNetworkSettings(p) => {
                buf.put_i32_le(p.protocol_version);
            }
            Self::NetworkSettings(p) => {
                buf.put_u16_le(p.compression_threshold);
                buf.put_u16_le(p.compression_algorithm);
                buf.put_u8(if p.client_cache_enabled { 1 } else { 0 });
            }
            Self::Login(p) => {
                buf.put_i32_le(p.protocol_version);
                // The JWT connection request chain and skin tokens are packaged inside a single VarString payload
                let mut inner = BytesMut::new();
                put_le_string(&mut inner, &p.chain_json);
                put_le_string(&mut inner, &p.client_data_jwt);
                let inner_bytes = inner.freeze();
                put_var_u32(&mut buf, inner_bytes.len() as u32);
                buf.put_slice(&inner_bytes);
            }
            Self::PlayStatus(p) => {
                buf.put_i32(p.status);
            }
            Self::ServerToClientHandshake(p) => {
                put_string(&mut buf, &p.handshake_web_token);
            }
            Self::ClientToServerHandshake(_) => {
                // empty payload in standard v662+
            }
            Self::Disconnect(p) => {
                put_var_i32(&mut buf, p.reason);
                buf.put_u8(if p.hide_reason { 1 } else { 0 });
                if !p.hide_reason {
                    put_string(&mut buf, p.message.as_deref().unwrap_or(""));
                }
            }
            Self::ResourcePacksInfo(p) => {
                buf.put_u8(if p.must_accept { 1 } else { 0 });
                buf.put_u8(if p.has_addons { 1 } else { 0 });
                buf.put_u16_le(p.behavior_packs.len() as u16);
                for pack in &p.behavior_packs {
                    put_string(&mut buf, &pack.id);
                    put_string(&mut buf, &pack.version);
                    buf.put_u64_le(pack.size);
                    put_string(&mut buf, &pack.content_key);
                    put_string(&mut buf, &pack.sub_pack_name);
                    put_string(&mut buf, &pack.content_identity);
                    buf.put_u8(if pack.has_scripts { 1 } else { 0 });
                }
                buf.put_u16_le(p.resource_packs.len() as u16);
                for pack in &p.resource_packs {
                    put_string(&mut buf, &pack.id);
                    put_string(&mut buf, &pack.version);
                    buf.put_u64_le(pack.size);
                    put_string(&mut buf, &pack.content_key);
                    put_string(&mut buf, &pack.sub_pack_name);
                    put_string(&mut buf, &pack.content_identity);
                    buf.put_u8(if pack.has_scripts { 1 } else { 0 });
                    buf.put_u8(if pack.rtx_enabled { 1 } else { 0 });
                }
            }
            Self::ResourcePackStack(p) => {
                buf.put_u8(if p.must_accept { 1 } else { 0 });
                put_var_u32(&mut buf, p.behavior_packs.len() as u32);
                for pack in &p.behavior_packs {
                    put_string(&mut buf, &pack.id);
                    put_string(&mut buf, &pack.version);
                    put_string(&mut buf, &pack.sub_pack_name);
                }
                put_var_u32(&mut buf, p.resource_packs.len() as u32);
                for pack in &p.resource_packs {
                    put_string(&mut buf, &pack.id);
                    put_string(&mut buf, &pack.version);
                    put_string(&mut buf, &pack.sub_pack_name);
                }
                put_string(&mut buf, &p.game_version);
                buf.put_u32_le(p.experiments.len() as u32);
                for exp in &p.experiments {
                    put_string(&mut buf, &exp.name);
                    buf.put_u8(if exp.enabled { 1 } else { 0 });
                }
                buf.put_u8(if p.experiments_previously_used { 1 } else { 0 });
            }
            Self::ResourcePackClientResponse(p) => {
                buf.put_u8(p.response_status);
                buf.put_u16_le(p.resource_pack_ids.len() as u16);
                for id in &p.resource_pack_ids {
                    put_string(&mut buf, id);
                }
            }
            Self::Text(p) => {
                buf.put_u8(p.packet_type);
                buf.put_u8(if p.needs_translation { 1 } else { 0 });
                match p.packet_type {
                    0 | 1 | 7 | 8 => {
                        put_string(&mut buf, &p.source_name);
                        put_string(&mut buf, &p.message);
                    }
                    2 | 3 | 4 => {
                        put_string(&mut buf, &p.message);
                        put_var_u32(&mut buf, p.parameters.len() as u32);
                        for param in &p.parameters {
                            put_string(&mut buf, param);
                        }
                    }
                    _ => {
                        put_string(&mut buf, &p.source_name);
                        put_string(&mut buf, &p.message);
                    }
                }
                put_string(&mut buf, &p.xbox_user_id);
                put_string(&mut buf, &p.platform_chat_id);
            }
            Self::StartGame(p) => {
                put_zigzag_i64(&mut buf, p.target_actor_id);
                put_var_u64(&mut buf, p.target_runtime_id);
                put_var_i32(&mut buf, p.actor_game_mode);
                buf.put_f32_le(p.position.x);
                buf.put_f32_le(p.position.y);
                buf.put_f32_le(p.position.z);
                buf.put_f32_le(p.rotation.x);
                buf.put_f32_le(p.rotation.y);
                // simplified remainder encoding as we mostly parse it, but if encoding a mock server StartGame:
                buf.put_slice(&p.remainder);
            }
            Self::MovePlayer(p) => {
                put_var_u64(&mut buf, p.runtime_id);
                buf.put_f32_le(p.position.x);
                buf.put_f32_le(p.position.y);
                buf.put_f32_le(p.position.z);
                buf.put_f32_le(p.pitch);
                buf.put_f32_le(p.yaw);
                buf.put_f32_le(p.head_yaw);
                buf.put_u8(p.mode);
                buf.put_u8(if p.on_ground { 1 } else { 0 });
                put_var_u64(&mut buf, p.riding_runtime_id);
                if p.mode == 2 {
                    // Teleport
                    buf.put_i32_le(p.teleport_cause);
                    buf.put_i32_le(p.teleport_item_id);
                }
                put_var_u64(&mut buf, p.tick);
            }
            Self::UpdateBlock(p) => {
                put_zigzag_i32(&mut buf, p.position.x);
                put_var_u32(&mut buf, p.position.y as u32);
                put_zigzag_i32(&mut buf, p.position.z);
                put_var_u32(&mut buf, p.block_runtime_id);
                put_var_u32(&mut buf, p.flags);
                put_var_u32(&mut buf, p.layer);
            }
            Self::LevelEvent(p) => {
                put_var_i32(&mut buf, p.event_id);
                buf.put_f32_le(p.position.x);
                buf.put_f32_le(p.position.y);
                buf.put_f32_le(p.position.z);
                put_var_i32(&mut buf, p.data);
            }
            Self::MobEquipment(p) => {
                put_var_u64(&mut buf, p.runtime_entity_id);
                // item descriptor
                buf.put_u8(0); // null descriptor
                buf.put_u8(p.selected_slot);
                buf.put_u8(p.slot);
                buf.put_u8(p.container_id);
            }
            Self::InventoryContent(p) => {
                put_var_u32(&mut buf, p.container_id);
                put_var_u32(&mut buf, p.slots.len() as u32);
                for slot in &p.slots {
                    put_var_i32(&mut buf, slot.network_id);
                    if slot.network_id > 0 {
                        buf.put_u16_le(slot.count);
                        put_var_u32(&mut buf, slot.metadata_val);
                        put_zigzag_i32(&mut buf, slot.block_runtime_id);
                        // simplified: put empty extra data / nbt
                        put_var_i32(&mut buf, 0);
                    }
                }
            }
            Self::InventorySlot(p) => {
                put_var_u32(&mut buf, p.container_id);
                put_var_u32(&mut buf, p.slot);
                // item descriptor
                put_var_i32(&mut buf, p.network_id);
                if p.network_id > 0 {
                    buf.put_u16_le(p.count);
                    put_var_u32(&mut buf, p.metadata_val);
                    put_zigzag_i32(&mut buf, p.block_runtime_id);
                    put_var_i32(&mut buf, 0);
                }
            }
            Self::Animate(p) => {
                put_var_i32(&mut buf, p.action_id);
                put_var_u64(&mut buf, p.runtime_entity_id);
                if p.action_id == 1 {
                    // swing arm
                    buf.put_f32_le(p.rowing_time);
                }
            }
            Self::Respawn(p) => {
                buf.put_f32_le(p.position.x);
                buf.put_f32_le(p.position.y);
                buf.put_f32_le(p.position.z);
                buf.put_u8(p.state);
                put_var_u64(&mut buf, p.runtime_entity_id);
            }
            Self::CommandOutput(p) => {
                // simple serialization of output messages count
                put_var_u32(&mut buf, p.output_messages.len() as u32);
                for msg in &p.output_messages {
                    buf.put_u8(if msg.is_internal { 1 } else { 0 });
                    put_string(&mut buf, &msg.message_id);
                    put_var_u32(&mut buf, msg.parameters.len() as u32);
                    for param in &msg.parameters {
                        put_string(&mut buf, param);
                    }
                }
            }
            Self::ModalFormRequest(p) => {
                put_var_u32(&mut buf, p.form_id);
                put_string(&mut buf, &p.form_content);
            }
            Self::ModalFormResponse(p) => {
                put_var_u32(&mut buf, p.form_id);
                buf.put_u8(if p.has_response_data { 1 } else { 0 });
                if p.has_response_data {
                    put_string(&mut buf, &p.response_data);
                }
                buf.put_u8(if p.has_cancel_reason { 1 } else { 0 });
                if p.has_cancel_reason {
                    buf.put_u8(p.cancel_reason);
                }
            }
            Self::SetLocalPlayerAsInitialized(p) => {
                put_var_u64(&mut buf, p.runtime_entity_id);
            }
            Self::NetworkStackLatency(p) => {
                buf.put_u64_le(p.timestamp);
                buf.put_u8(if p.needs_response { 1 } else { 0 });
            }
            Self::ClientCacheStatus(p) => {
                buf.put_u8(if p.support_client_cache { 1 } else { 0 });
            }
            Self::ItemStackResponse(p) => {
                put_var_u32(&mut buf, p.responses.len() as u32);
                for resp in &p.responses {
                    buf.put_u8(resp.status);
                    put_var_i32(&mut buf, resp.client_request_id);
                }
            }
            Self::CorrectPlayerMovePrediction(p) => {
                buf.put_f32_le(p.position.x);
                buf.put_f32_le(p.position.y);
                buf.put_f32_le(p.position.z);
                buf.put_f32_le(p.delta.x);
                buf.put_f32_le(p.delta.y);
                buf.put_f32_le(p.delta.z);
                buf.put_u8(if p.on_ground { 1 } else { 0 });
                put_var_u64(&mut buf, p.tick);
            }
            Self::InventoryTransaction(p) => {
                put_var_u32(&mut buf, p.transaction_type);
                put_var_u32(&mut buf, p.actions.len() as u32);
                buf.put_slice(&p.transaction_data);
            }
            Self::RequestChunkRadius(p) => {
                put_var_i32(&mut buf, p.radius);
                buf.put_u8(p.max_radius);
            }
            Self::Unknown { payload, .. } => {
                buf.put_slice(payload);
            }
        }
        Ok(buf.freeze())
    }

    pub fn decode(id: u32, buf: &mut Bytes, version: ProtocolVersion) -> Result<Self, CoreError> {
        match id {
            0xc1 => {
                if buf.remaining() < 4 {
                    return Err(CoreError::UnexpectedEof("RequestNetworkSettings"));
                }
                let protocol_version = buf.get_i32_le();
                Ok(Self::RequestNetworkSettings(RequestNetworkSettingsPacket {
                    protocol_version,
                }))
            }
            0xc2 => {
                if buf.remaining() < 5 {
                    return Err(CoreError::UnexpectedEof("NetworkSettings"));
                }
                let compression_threshold = buf.get_u16_le();
                let compression_algorithm = buf.get_u16_le();
                let client_cache_enabled = buf.get_u8() != 0;
                Ok(Self::NetworkSettings(NetworkSettingsPacket {
                    compression_threshold,
                    compression_algorithm,
                    client_cache_enabled,
                }))
            }
            0x01 => {
                if buf.remaining() < 4 {
                    return Err(CoreError::UnexpectedEof("Login protocol version"));
                }
                let protocol_version = buf.get_i32_le();
                let payload_len = get_var_u32(buf)? as usize;
                if buf.remaining() < payload_len {
                    return Err(CoreError::UnexpectedEof("Login body"));
                }
                let mut body = buf.copy_to_bytes(payload_len);
                let chain_json = get_le_string(&mut body)?;
                let client_data_jwt = get_le_string(&mut body)?;
                Ok(Self::Login(LoginPacket {
                    protocol_version,
                    chain_json,
                    client_data_jwt,
                }))
            }
            0x02 => {
                if buf.remaining() < 4 {
                    return Err(CoreError::UnexpectedEof("PlayStatus"));
                }
                let status = buf.get_i32();
                Ok(Self::PlayStatus(PlayStatusPacket { status }))
            }
            0x03 => {
                let handshake_web_token = get_string(buf)?;
                Ok(Self::ServerToClientHandshake(
                    ServerToClientHandshakePacket {
                        handshake_web_token,
                    },
                ))
            }
            0x04 => Ok(Self::ClientToServerHandshake(
                ClientToServerHandshakePacket {},
            )),
            0x05 => {
                let reason = get_var_i32(buf)?;
                if buf.remaining() < 1 {
                    return Err(CoreError::UnexpectedEof("Disconnect hide_reason"));
                }
                let hide_reason = buf.get_u8() != 0;
                let message = if !hide_reason {
                    Some(get_string(buf)?)
                } else {
                    None
                };
                Ok(Self::Disconnect(DisconnectPacket {
                    reason,
                    hide_reason,
                    message,
                }))
            }
            0x06 => {
                if buf.remaining() < 2 {
                    return Err(CoreError::UnexpectedEof("ResourcePacksInfo"));
                }
                let must_accept = buf.get_u8() != 0;
                let has_addons = buf.get_u8() != 0;
                let behavior_packs_len = buf.get_u16_le() as usize;
                let mut behavior_packs = Vec::new();
                for _ in 0..behavior_packs_len {
                    let id = get_string(buf)?;
                    let version = get_string(buf)?;
                    if buf.remaining() < 8 {
                        return Err(CoreError::UnexpectedEof("BehaviorPack size"));
                    }
                    let size = buf.get_u64_le();
                    let content_key = get_string(buf)?;
                    let sub_pack_name = get_string(buf)?;
                    let content_identity = get_string(buf)?;
                    if buf.remaining() < 1 {
                        return Err(CoreError::UnexpectedEof("BehaviorPack scripts"));
                    }
                    let has_scripts = buf.get_u8() != 0;
                    behavior_packs.push(ResourcePackInfoEntry {
                        id,
                        version,
                        size,
                        content_key,
                        sub_pack_name,
                        content_identity,
                        has_scripts,
                        rtx_enabled: false,
                    });
                }
                let resource_packs_len = buf.get_u16_le() as usize;
                let mut resource_packs = Vec::new();
                for _ in 0..resource_packs_len {
                    let id = get_string(buf)?;
                    let version = get_string(buf)?;
                    if buf.remaining() < 8 {
                        return Err(CoreError::UnexpectedEof("ResourcePack size"));
                    }
                    let size = buf.get_u64_le();
                    let content_key = get_string(buf)?;
                    let sub_pack_name = get_string(buf)?;
                    let content_identity = get_string(buf)?;
                    if buf.remaining() < 2 {
                        return Err(CoreError::UnexpectedEof("ResourcePack flags"));
                    }
                    let has_scripts = buf.get_u8() != 0;
                    let rtx_enabled = buf.get_u8() != 0;
                    resource_packs.push(ResourcePackInfoEntry {
                        id,
                        version,
                        size,
                        content_key,
                        sub_pack_name,
                        content_identity,
                        has_scripts,
                        rtx_enabled,
                    });
                }
                Ok(Self::ResourcePacksInfo(ResourcePacksInfoPacket {
                    must_accept,
                    has_addons,
                    behavior_packs,
                    resource_packs,
                }))
            }
            0x07 => {
                if buf.remaining() < 1 {
                    return Err(CoreError::UnexpectedEof("ResourcePackStack"));
                }
                let must_accept = buf.get_u8() != 0;
                let behavior_packs_len = get_var_u32(buf)? as usize;
                let mut behavior_packs = Vec::new();
                for _ in 0..behavior_packs_len {
                    let id = get_string(buf)?;
                    let version = get_string(buf)?;
                    let sub_pack_name = get_string(buf)?;
                    behavior_packs.push(ResourcePackStackEntry {
                        id,
                        version,
                        sub_pack_name,
                    });
                }
                let resource_packs_len = get_var_u32(buf)? as usize;
                let mut resource_packs = Vec::new();
                for _ in 0..resource_packs_len {
                    let id = get_string(buf)?;
                    let version = get_string(buf)?;
                    let sub_pack_name = get_string(buf)?;
                    resource_packs.push(ResourcePackStackEntry {
                        id,
                        version,
                        sub_pack_name,
                    });
                }
                let game_version = get_string(buf)?;
                if buf.remaining() < 4 {
                    return Err(CoreError::UnexpectedEof(
                        "ResourcePackStack experiments len",
                    ));
                }
                let exp_len = buf.get_u32_le() as usize;
                let mut experiments = Vec::new();
                for _ in 0..exp_len {
                    let name = get_string(buf)?;
                    if buf.remaining() < 1 {
                        return Err(CoreError::UnexpectedEof(
                            "ResourcePackStack experiment enabled",
                        ));
                    }
                    let enabled = buf.get_u8() != 0;
                    experiments.push(ExperimentEntry { name, enabled });
                }
                if buf.remaining() < 1 {
                    return Err(CoreError::UnexpectedEof(
                        "ResourcePackStack experiments_previously_used",
                    ));
                }
                let experiments_previously_used = buf.get_u8() != 0;
                Ok(Self::ResourcePackStack(ResourcePackStackPacket {
                    must_accept,
                    behavior_packs,
                    resource_packs,
                    game_version,
                    experiments,
                    experiments_previously_used,
                }))
            }
            0x08 => {
                if buf.remaining() < 1 {
                    return Err(CoreError::UnexpectedEof("ResourcePackClientResponse"));
                }
                let response_status = buf.get_u8();
                if buf.remaining() < 2 {
                    return Err(CoreError::UnexpectedEof(
                        "ResourcePackClientResponse list len",
                    ));
                }
                let len = buf.get_u16_le() as usize;
                let mut resource_pack_ids = Vec::new();
                for _ in 0..len {
                    resource_pack_ids.push(get_string(buf)?);
                }
                Ok(Self::ResourcePackClientResponse(
                    ResourcePackClientResponsePacket {
                        response_status,
                        resource_pack_ids,
                    },
                ))
            }
            0x09 => {
                if buf.remaining() < 2 {
                    return Err(CoreError::UnexpectedEof("Text header"));
                }
                let packet_type = buf.get_u8();
                let needs_translation = buf.get_u8() != 0;
                let mut source_name = String::new();
                let mut message = String::new();
                let mut parameters = Vec::new();
                match packet_type {
                    0 | 1 | 7 | 8 => {
                        source_name = get_string(buf)?;
                        message = get_string(buf)?;
                    }
                    2 | 3 | 4 => {
                        message = get_string(buf)?;
                        let len = get_var_u32(buf)? as usize;
                        for _ in 0..len {
                            parameters.push(get_string(buf)?);
                        }
                    }
                    _ => {
                        source_name = get_string(buf)?;
                        message = get_string(buf)?;
                    }
                }
                let xbox_user_id = get_string(buf)?;
                let platform_chat_id = get_string(buf)?;
                Ok(Self::Text(TextPacket {
                    packet_type,
                    needs_translation,
                    source_name,
                    message,
                    parameters,
                    xbox_user_id,
                    platform_chat_id,
                }))
            }
            0x0b => {
                let target_actor_id = get_zigzag_i64(buf)?;
                let target_runtime_id = get_var_u64(buf)?;
                let actor_game_mode = get_var_i32(buf)?;
                if buf.remaining() < 20 {
                    return Err(CoreError::UnexpectedEof("StartGame position/rotation"));
                }
                let position = Vector3f {
                    x: buf.get_f32_le(),
                    y: buf.get_f32_le(),
                    z: buf.get_f32_le(),
                };
                let rotation = Vector2f {
                    x: buf.get_f32_le(),
                    y: buf.get_f32_le(),
                };
                // save the rest of start_game for fallback parsing if needed
                let remainder = buf.copy_to_bytes(buf.remaining()).to_vec();
                Ok(Self::StartGame(StartGamePacket {
                    target_actor_id,
                    target_runtime_id,
                    actor_game_mode,
                    position,
                    rotation,
                    remainder,
                }))
            }
            0x13 => {
                let runtime_id = get_var_u64(buf)?;
                if buf.remaining() < 24 {
                    return Err(CoreError::UnexpectedEof("MovePlayer body"));
                }
                let position = Vector3f {
                    x: buf.get_f32_le(),
                    y: buf.get_f32_le(),
                    z: buf.get_f32_le(),
                };
                let pitch = buf.get_f32_le();
                let yaw = buf.get_f32_le();
                let head_yaw = buf.get_f32_le();
                if buf.remaining() < 2 {
                    return Err(CoreError::UnexpectedEof("MovePlayer footer"));
                }
                let mode = buf.get_u8();
                let on_ground = buf.get_u8() != 0;
                let riding_runtime_id = get_var_u64(buf)?;
                let mut teleport_cause = 0;
                let mut teleport_item_id = 0;
                if mode == 2 {
                    if buf.remaining() < 8 {
                        return Err(CoreError::UnexpectedEof("MovePlayer teleport details"));
                    }
                    teleport_cause = buf.get_i32_le();
                    teleport_item_id = buf.get_i32_le();
                }
                let tick = get_var_u64(buf)?;
                Ok(Self::MovePlayer(MovePlayerPacket {
                    runtime_id,
                    position,
                    pitch,
                    yaw,
                    head_yaw,
                    mode,
                    on_ground,
                    riding_runtime_id,
                    teleport_cause,
                    teleport_item_id,
                    tick,
                }))
            }
            0x15 => {
                let x = get_zigzag_i32(buf)?;
                let y = get_var_u32(buf)? as i32;
                let z = get_zigzag_i32(buf)?;
                let block_runtime_id = get_var_u32(buf)?;
                let flags = get_var_u32(buf)?;
                let layer = get_var_u32(buf)?;
                Ok(Self::UpdateBlock(UpdateBlockPacket {
                    position: BlockPosition { x, y, z },
                    block_runtime_id,
                    flags,
                    layer,
                }))
            }
            0x19 => {
                let event_id = get_var_i32(buf)?;
                if buf.remaining() < 12 {
                    return Err(CoreError::UnexpectedEof("LevelEvent vector"));
                }
                let position = Vector3f {
                    x: buf.get_f32_le(),
                    y: buf.get_f32_le(),
                    z: buf.get_f32_le(),
                };
                let data = get_var_i32(buf)?;
                Ok(Self::LevelEvent(LevelEventPacket {
                    event_id,
                    position,
                    data,
                }))
            }
            0x1f => {
                let runtime_entity_id = get_var_u64(buf)?;
                // skip item descriptor
                if buf.remaining() < 1 {
                    return Err(CoreError::UnexpectedEof("MobEquipment item desc tag"));
                }
                let desc_tag = buf.get_u8();
                if desc_tag > 0 {
                    // skip simple item metadata fields if present
                    get_zigzag_i32(buf)?;
                }
                if buf.remaining() < 3 {
                    return Err(CoreError::UnexpectedEof("MobEquipment slots"));
                }
                let selected_slot = buf.get_u8();
                let slot = buf.get_u8();
                let container_id = buf.get_u8();
                Ok(Self::MobEquipment(MobEquipmentPacket {
                    runtime_entity_id,
                    selected_slot,
                    slot,
                    container_id,
                }))
            }
            0x31 => {
                let container_id = get_var_u32(buf)?;
                let slots_len = get_var_u32(buf)? as usize;
                let mut slots = Vec::new();
                for _ in 0..slots_len {
                    let network_id = get_var_i32(buf)?;
                    let mut count = 0;
                    let mut metadata_val = 0;
                    let mut block_runtime_id = 0;
                    if network_id > 0 {
                        if buf.remaining() < 2 {
                            return Err(CoreError::UnexpectedEof("InventoryContent item count"));
                        }
                        count = buf.get_u16_le();
                        metadata_val = get_var_u32(buf)?;
                        block_runtime_id = get_zigzag_i32(buf)?;
                        // skip extra NBT / components
                        let extra_len = get_var_i32(buf)? as usize;
                        if buf.remaining() < extra_len {
                            return Err(CoreError::UnexpectedEof(
                                "InventoryContent item extra bytes",
                            ));
                        }
                        buf.advance(extra_len);
                    }
                    slots.push(InventoryItem {
                        network_id,
                        count,
                        metadata_val,
                        block_runtime_id,
                    });
                }
                Ok(Self::InventoryContent(InventoryContentPacket {
                    container_id,
                    slots,
                }))
            }
            0x32 => {
                let container_id = get_var_u32(buf)?;
                let slot = get_var_u32(buf)?;
                let network_id = get_var_i32(buf)?;
                let mut count = 0;
                let mut metadata_val = 0;
                let mut block_runtime_id = 0;
                if network_id > 0 {
                    if buf.remaining() < 2 {
                        return Err(CoreError::UnexpectedEof("InventorySlot item count"));
                    }
                    count = buf.get_u16_le();
                    metadata_val = get_var_u32(buf)?;
                    block_runtime_id = get_zigzag_i32(buf)?;
                    let extra_len = get_var_i32(buf)? as usize;
                    if buf.remaining() < extra_len {
                        return Err(CoreError::UnexpectedEof("InventorySlot item extra bytes"));
                    }
                    buf.advance(extra_len);
                }
                Ok(Self::InventorySlot(InventorySlotPacket {
                    container_id,
                    slot,
                    network_id,
                    count,
                    metadata_val,
                    block_runtime_id,
                }))
            }
            0x2c => {
                let action_id = get_var_i32(buf)?;
                let runtime_entity_id = get_var_u64(buf)?;
                let mut rowing_time = 0.0;
                if action_id == 1 {
                    if buf.remaining() < 4 {
                        return Err(CoreError::UnexpectedEof("Animate rowing_time"));
                    }
                    rowing_time = buf.get_f32_le();
                }
                Ok(Self::Animate(AnimatePacket {
                    action_id,
                    runtime_entity_id,
                    rowing_time,
                }))
            }
            0x2d => {
                if buf.remaining() < 13 {
                    return Err(CoreError::UnexpectedEof("Respawn header"));
                }
                let position = Vector3f {
                    x: buf.get_f32_le(),
                    y: buf.get_f32_le(),
                    z: buf.get_f32_le(),
                };
                let state = buf.get_u8();
                let runtime_entity_id = get_var_u64(buf)?;
                Ok(Self::Respawn(RespawnPacket {
                    position,
                    state,
                    runtime_entity_id,
                }))
            }
            0x4f => {
                let len = get_var_u32(buf)? as usize;
                let mut output_messages = Vec::new();
                for _ in 0..len {
                    if buf.remaining() < 1 {
                        return Err(CoreError::UnexpectedEof("CommandOutput message type"));
                    }
                    let is_internal = buf.get_u8() != 0;
                    let message_id = get_string(buf)?;
                    let params_len = get_var_u32(buf)? as usize;
                    let mut parameters = Vec::new();
                    for _ in 0..params_len {
                        parameters.push(get_string(buf)?);
                    }
                    output_messages.push(CommandOutputMessage {
                        is_internal,
                        message_id,
                        parameters,
                    });
                }
                Ok(Self::CommandOutput(CommandOutputPacket { output_messages }))
            }
            0x64 => {
                let form_id = get_var_u32(buf)?;
                let form_content = get_string(buf)?;
                Ok(Self::ModalFormRequest(ModalFormRequestPacket {
                    form_id,
                    form_content,
                }))
            }
            0x65 => {
                let form_id = get_var_u32(buf)?;
                if buf.remaining() < 1 {
                    return Err(CoreError::UnexpectedEof("ModalFormResponse has_response"));
                }
                let has_response_data = buf.get_u8() != 0;
                let response_data = if has_response_data {
                    get_string(buf)?
                } else {
                    String::new()
                };
                let mut has_cancel_reason = false;
                let mut cancel_reason = 0;
                if buf.remaining() > 0 {
                    has_cancel_reason = buf.get_u8() != 0;
                    if has_cancel_reason && buf.remaining() > 0 {
                        cancel_reason = buf.get_u8();
                    }
                }
                Ok(Self::ModalFormResponse(ModalFormResponsePacket {
                    form_id,
                    has_response_data,
                    response_data,
                    has_cancel_reason,
                    cancel_reason,
                }))
            }
            0x71 => {
                let runtime_entity_id = get_var_u64(buf)?;
                Ok(Self::SetLocalPlayerAsInitialized(
                    SetLocalPlayerAsInitializedPacket { runtime_entity_id },
                ))
            }
            0x73 => {
                if buf.remaining() < 9 {
                    return Err(CoreError::UnexpectedEof("NetworkStackLatency"));
                }
                let timestamp = buf.get_u64_le();
                let needs_response = buf.get_u8() != 0;
                Ok(Self::NetworkStackLatency(NetworkStackLatencyPacket {
                    timestamp,
                    needs_response,
                }))
            }
            0x81 => {
                if buf.remaining() < 1 {
                    return Err(CoreError::UnexpectedEof("ClientCacheStatus"));
                }
                let support_client_cache = buf.get_u8() != 0;
                Ok(Self::ClientCacheStatus(ClientCacheStatusPacket {
                    support_client_cache,
                }))
            }
            0x94 => {
                let responses_len = get_var_u32(buf)? as usize;
                let mut responses = Vec::new();
                for _ in 0..responses_len {
                    if buf.remaining() < 1 {
                        return Err(CoreError::UnexpectedEof("ItemStackResponse status"));
                    }
                    let status = buf.get_u8();
                    let client_request_id = get_var_i32(buf)?;
                    responses.push(ItemStackResponseEntry {
                        status,
                        client_request_id,
                    });
                }
                Ok(Self::ItemStackResponse(ItemStackResponsePacket {
                    responses,
                }))
            }
            0xa1 => {
                if buf.remaining() < 25 {
                    return Err(CoreError::UnexpectedEof("CorrectPlayerMovePrediction"));
                }
                let position = Vector3f {
                    x: buf.get_f32_le(),
                    y: buf.get_f32_le(),
                    z: buf.get_f32_le(),
                };
                let delta = Vector3f {
                    x: buf.get_f32_le(),
                    y: buf.get_f32_le(),
                    z: buf.get_f32_le(),
                };
                let on_ground = buf.get_u8() != 0;
                let tick = get_var_u64(buf)?;
                Ok(Self::CorrectPlayerMovePrediction(
                    CorrectPlayerMovePredictionPacket {
                        position,
                        delta,
                        on_ground,
                        tick,
                    },
                ))
            }
            0x1e => {
                let transaction_type = get_var_u32(buf)?;
                let actions_len = get_var_u32(buf)? as usize;
                let mut actions = Vec::new();
                for _ in 0..actions_len {
                    // simple skip of actions bytes/data
                    let act_type = get_var_u32(buf)?;
                    get_var_u32(buf)?; // slot
                }
                let transaction_data = buf.copy_to_bytes(buf.remaining()).to_vec();
                Ok(Self::InventoryTransaction(InventoryTransactionPacket {
                    transaction_type,
                    actions,
                    transaction_data,
                }))
            }
            _ => {
                let payload = buf.copy_to_bytes(buf.remaining());
                Ok(Self::Unknown { id, payload })
            }
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::RequestNetworkSettings(p) => serde_json::json!({
                "protocol_version": p.protocol_version
            }),
            Self::NetworkSettings(p) => serde_json::json!({
                "compression_threshold": p.compression_threshold,
                "compression_algorithm": p.compression_algorithm,
                "client_cache_enabled": p.client_cache_enabled
            }),
            Self::Login(p) => serde_json::json!({
                "protocol_version": p.protocol_version,
                "chain_json": p.chain_json,
                "client_data_jwt": p.client_data_jwt
            }),
            Self::PlayStatus(p) => serde_json::json!({
                "status": p.status
            }),
            Self::Text(p) => serde_json::json!({
                "type": p.packet_type,
                "needs_translation": p.needs_translation,
                "source": p.source_name,
                "message": p.message,
                "parameters": p.parameters
            }),
            Self::ModalFormRequest(p) => serde_json::json!({
                "form_id": p.form_id,
                "content": p.form_content
            }),
            Self::ModalFormResponse(p) => serde_json::json!({
                "form_id": p.form_id,
                "has_response": p.has_response_data,
                "response": p.response_data
            }),
            _ => serde_json::json!({
                "id": self.id(),
                "type": "unimplemented_json"
            }),
        }
    }
}

// Packet Structs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestNetworkSettingsPacket {
    pub protocol_version: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkSettingsPacket {
    pub compression_threshold: u16,
    pub compression_algorithm: u16,
    pub client_cache_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginPacket {
    pub protocol_version: i32,
    pub chain_json: String,
    pub client_data_jwt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayStatusPacket {
    pub status: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerToClientHandshakePacket {
    pub handshake_web_token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientToServerHandshakePacket {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisconnectPacket {
    pub reason: i32,
    pub hide_reason: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourcePackInfoEntry {
    pub id: String,
    pub version: String,
    pub size: u64,
    pub content_key: String,
    pub sub_pack_name: String,
    pub content_identity: String,
    pub has_scripts: bool,
    pub rtx_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourcePacksInfoPacket {
    pub must_accept: bool,
    pub has_addons: bool,
    pub behavior_packs: Vec<ResourcePackInfoEntry>,
    pub resource_packs: Vec<ResourcePackInfoEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourcePackStackEntry {
    pub id: String,
    pub version: String,
    pub sub_pack_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentEntry {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourcePackStackPacket {
    pub must_accept: bool,
    pub behavior_packs: Vec<ResourcePackStackEntry>,
    pub resource_packs: Vec<ResourcePackStackEntry>,
    pub game_version: String,
    pub experiments: Vec<ExperimentEntry>,
    pub experiments_previously_used: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourcePackClientResponsePacket {
    pub response_status: u8,
    pub resource_pack_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPacket {
    pub packet_type: u8,
    pub needs_translation: bool,
    pub source_name: String,
    pub message: String,
    pub parameters: Vec<String>,
    pub xbox_user_id: String,
    pub platform_chat_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartGamePacket {
    pub target_actor_id: i64,
    pub target_runtime_id: u64,
    pub actor_game_mode: i32,
    pub position: Vector3f,
    pub rotation: Vector2f,
    pub remainder: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MovePlayerPacket {
    pub runtime_id: u64,
    pub position: Vector3f,
    pub pitch: f32,
    pub yaw: f32,
    pub head_yaw: f32,
    pub mode: u8,
    pub on_ground: bool,
    pub riding_runtime_id: u64,
    pub teleport_cause: i32,
    pub teleport_item_id: i32,
    pub tick: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateBlockPacket {
    pub position: BlockPosition,
    pub block_runtime_id: u32,
    pub flags: u32,
    pub layer: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LevelEventPacket {
    pub event_id: i32,
    pub position: Vector3f,
    pub data: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobEquipmentPacket {
    pub runtime_entity_id: u64,
    pub selected_slot: u8,
    pub slot: u8,
    pub container_id: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryItem {
    pub network_id: i32,
    pub count: u16,
    pub metadata_val: u32,
    pub block_runtime_id: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryContentPacket {
    pub container_id: u32,
    pub slots: Vec<InventoryItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventorySlotPacket {
    pub container_id: u32,
    pub slot: u32,
    pub network_id: i32,
    pub count: u16,
    pub metadata_val: u32,
    pub block_runtime_id: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AnimatePacket {
    pub action_id: i32,
    pub runtime_entity_id: u64,
    pub rowing_time: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RespawnPacket {
    pub position: Vector3f,
    pub state: u8,
    pub runtime_entity_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutputMessage {
    pub is_internal: bool,
    pub message_id: String,
    pub parameters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutputPacket {
    pub output_messages: Vec<CommandOutputMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModalFormRequestPacket {
    pub form_id: u32,
    pub form_content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModalFormResponsePacket {
    pub form_id: u32,
    pub has_response_data: bool,
    pub response_data: String,
    pub has_cancel_reason: bool,
    pub cancel_reason: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetLocalPlayerAsInitializedPacket {
    pub runtime_entity_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkStackLatencyPacket {
    pub timestamp: u64,
    pub needs_response: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientCacheStatusPacket {
    pub support_client_cache: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemStackResponseEntry {
    pub status: u8,
    pub client_request_id: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemStackResponsePacket {
    pub responses: Vec<ItemStackResponseEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CorrectPlayerMovePredictionPacket {
    pub position: Vector3f,
    pub delta: Vector3f,
    pub on_ground: bool,
    pub tick: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryTransactionAction {
    pub source: u32,
    pub slot: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryTransactionPacket {
    pub transaction_type: u32,
    pub actions: Vec<InventoryTransactionAction>,
    pub transaction_data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestChunkRadiusPacket {
    pub radius: i32,
    pub max_radius: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_login_roundtrip() {
        let original = Packet::Login(LoginPacket {
            protocol_version: 898,
            chain_json: "{\"chain\":[]}".to_string(),
            client_data_jwt: "client.jwt.token".to_string(),
        });
        let bytes = original.encode(ProtocolVersion::V898).unwrap();
        let decoded =
            Packet::decode(original.id(), &mut bytes.clone(), ProtocolVersion::V898).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_play_status_roundtrip() {
        let original = Packet::PlayStatus(PlayStatusPacket { status: 3 });
        let bytes = original.encode(ProtocolVersion::V898).unwrap();
        let decoded =
            Packet::decode(original.id(), &mut bytes.clone(), ProtocolVersion::V898).unwrap();
        assert_eq!(original, decoded);
    }
}
