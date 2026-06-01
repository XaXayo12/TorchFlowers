use std::time::{Duration, Instant};

use bedrock::protocol::{
    V975,
    v662::{
        enums::{
            ComplexInventoryTransactionType, InputMode, NewInteractionModel, PlayerPositionMode,
            ResourcePackResponse,
        },
        packets::{
            InventoryTransactionPacket, MovePlayerPacket, RequestChunkRadiusPacket,
            ResourcePackClientResponsePacket, SetLocalPlayerAsInitializedPacket,
        },
        types::{ActorRuntimeID, InventoryTransaction},
    },
    v766::packets::{ClientPlayMode, PlayerAuthInputPacket},
    v898::packets::TextPacket,
    v924::enums::TextPacketType,
};
use serde_json::json;
use tokio::time::timeout;

use crate::{
    auth::ProvisionedBedrockSession, bedrock::protocol_adapter::BedrockProtocolAdapter,
    db::Database, diagnostics::Diagnostics, error::EngineResult, models::CapabilityStatus,
};

pub struct BedrockBotSession {
    db: Database,
    diagnostics: Diagnostics,
}

impl BedrockBotSession {
    pub fn new(db: Database) -> Self {
        Self {
            diagnostics: Diagnostics::new(db.clone()),
            db,
        }
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
        let mut status = CapabilityStatus::default();
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

        conn.request_network_settings().await?;
        status.keepalive = true;
        conn.send_login(session).await?;
        status.login = true;

        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(35) {
            let packets = timeout(Duration::from_secs(10), conn.recv())
                .await
                .map_err(|err| {
                    crate::error::EngineError::Bedrock(format!("Bedrock recv timed out: {err}"))
                })??;
            for packet in packets {
                match packet {
                    V975::PlayStatusPacket(_) => {
                        status.login = true;
                    }
                    V975::ResourcePacksInfoPacket(_) | V975::ResourcePackStackPacket(_) => {
                        conn.send(&[V975::ResourcePackClientResponsePacket(Box::new(
                            ResourcePackClientResponsePacket::<V975> {
                                response: ResourcePackResponse::ResourcePackStackFinished,
                                downloading_packs: vec![],
                            },
                        ))])
                        .await?;
                    }
                    V975::StartGamePacket(start) => {
                        let runtime_id = start.target_runtime_id;
                        status.spawn = true;
                        conn.send(&[
                            V975::SetLocalPlayerAsInitializedPacket(Box::new(
                                SetLocalPlayerAsInitializedPacket::<V975> {
                                    player_id: runtime_id.clone(),
                                },
                            )),
                            V975::RequestChunkRadiusPacket(Box::new(RequestChunkRadiusPacket {
                                chunk_radius: 4,
                                max_chunk_radius: 4,
                            })),
                        ])
                        .await?;
                        self.send_movement_probe(&mut conn, runtime_id, 1).await?;
                        status.movement = true;
                        self.send_inventory_probe(&mut conn).await?;
                        status.inventory_transactions = true;
                        if send_chat_probe {
                            self.send_chat_probe(&mut conn).await?;
                            status.chat = true;
                        }
                    }
                    V975::NetworkStackLatencyPacket(latency) => {
                        if latency.is_from_server {
                            conn.send(&[V975::NetworkStackLatencyPacket(Box::new(
                                bedrock::protocol::v662::packets::NetworkStackLatencyPacket {
                                    creation_time: latency.creation_time,
                                    is_from_server: false,
                                },
                            ))])
                            .await?;
                            status.keepalive = true;
                        }
                    }
                    V975::TextPacket(_) => {
                        status.chat = true;
                    }
                    V975::ModalFormRequestPacket(form) => {
                        conn.send(&[V975::ModalFormResponsePacket(Box::new(
                            bedrock::protocol::v662::packets::ModalFormResponsePacket::<V975> {
                                form_id: form.form_id,
                                json_response: None,
                                form_cancel_reason: None,
                            },
                        ))])
                        .await?;
                        status.forms = true;
                    }
                    V975::InventoryContentPacket(_) | V975::InventorySlotPacket(_) => {
                        status.inventory_transactions = true;
                    }
                    V975::DisconnectPacket(disconnect) => {
                        status.disconnect_handling = true;
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
                        self.fill_missing(&mut status);
                        return Ok(status);
                    }
                    _ => {}
                }
            }
            if status.spawn && status.keepalive && status.movement && status.inventory_transactions
            {
                break;
            }
        }

        conn.close().await;
        status.disconnect_handling = true;
        self.fill_missing(&mut status);
        if let Some(bot_id) = bot_id {
            self.db
                .update_bot_capabilities(bot_id, &serde_json::to_value(&status)?)
                .await?;
        }
        Ok(status)
    }

    async fn send_chat_probe(&self, conn: &mut BedrockProtocolAdapter) -> EngineResult<()> {
        conn.send(&[V975::TextPacket(Box::new(TextPacket::<V975> {
            localize: false,
            message_type: TextPacketType::Chat {
                player_name: "RustRock".to_string(),
                message: "RustRock validation online".to_string(),
            },
            sender_xuid: String::new(),
            platform_id: String::new(),
            filtered_message: None,
        }))])
        .await
    }

    async fn send_movement_probe(
        &self,
        conn: &mut BedrockProtocolAdapter,
        runtime_id: ActorRuntimeID,
        tick: u64,
    ) -> EngineResult<()> {
        conn.send(&[
            V975::MovePlayerPacket(Box::new(MovePlayerPacket::<V975> {
                player_runtime_id: runtime_id,
                position: (0.0, 64.0, 0.0),
                rotation: (0.0, 0.0),
                y_head_rotation: 0.0,
                position_mode: PlayerPositionMode::Normal,
                on_ground: true,
                riding_runtime_id: ActorRuntimeID(0),
                tick,
            })),
            V975::PlayerAuthInputPacket(Box::new(PlayerAuthInputPacket::<V975> {
                player_rotation: (0.0, 0.0),
                player_position: (0.0, 64.0, 0.0),
                move_vector: (0.0, 0.0, 0.0),
                player_head_rotation: 0.0,
                input_data: 0,
                input_mode: InputMode::Mouse,
                play_mode: ClientPlayMode::Normal,
                new_interaction_model: NewInteractionModel::Crosshair,
                interact_rotation: (0.0, 0.0),
                client_tick: tick,
                velocity: (0.0, 0.0, 0.0),
                item_use_transaction: None,
                item_stack_request: None,
                player_block_actions: None,
                client_predicted_vehicle: None,
                analog_move_vector: (0.0, 0.0),
                camera_orientation: (0.0, 0.0, 0.0),
                raw_move_vector: (0.0, 0.0),
            })),
        ])
        .await
    }

    async fn send_inventory_probe(&self, conn: &mut BedrockProtocolAdapter) -> EngineResult<()> {
        conn.send(&[V975::InventoryTransactionPacket(Box::new(
            InventoryTransactionPacket::<V975> {
                raw_id: 0,
                legacy_set_item_slots: vec![],
                transaction_type: ComplexInventoryTransactionType::NormalTransaction,
                transaction: InventoryTransaction::<V975> { action: vec![] },
            },
        ))])
        .await
    }

    fn fill_missing(&self, status: &mut CapabilityStatus) {
        let missing = [
            ("login", status.login),
            ("spawn", status.spawn),
            ("keepalive", status.keepalive),
            ("chat", status.chat),
            ("forms", status.forms),
            ("inventory_transactions", status.inventory_transactions),
            ("movement", status.movement),
            ("disconnect_handling", status.disconnect_handling),
        ]
        .into_iter()
        .filter_map(|(name, ok)| (!ok).then_some(name.to_string()))
        .collect();
        status.missing_capabilities = missing;
    }
}
