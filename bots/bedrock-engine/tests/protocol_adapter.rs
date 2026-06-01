use bedrock::{
    network::codec::{decode_packets, encode_packets},
    protocol::{
        ProtoVersion, Unknown, V975,
        unknown::packets::RequestNetworkSettingsPacket,
        v662::{
            enums::{
                ComplexInventoryTransactionType, ConnectionFailReason, InputMode,
                NewInteractionModel, PlayerPositionMode, ResourcePackResponse,
            },
            packets::{
                InventoryTransactionPacket, MovePlayerPacket, NetworkStackLatencyPacket,
                RequestChunkRadiusPacket, ResourcePackClientResponsePacket,
                SetLocalPlayerAsInitializedPacket,
            },
            types::{ActorRuntimeID, InventoryTransaction},
        },
        v712::packets::{DisconnectMessage, DisconnectPacket},
        v766::packets::{ClientPlayMode, PlayerAuthInputPacket},
        v898::packets::TextPacket,
        v924::enums::TextPacketType,
    },
};

#[test]
fn bedrock_rs_round_trips_validation_packet_surface() {
    round_trip(V975::LoginPacket(Box::new(
        bedrock::protocol::v662::packets::LoginPacket {
            client_network_version: V975::PROTOCOL_VERSION as i32,
            connection_request: br#"{"chain":[],"skin":""}"#.to_vec(),
        },
    )));
    round_trip(V975::ResourcePackClientResponsePacket(Box::new(
        ResourcePackClientResponsePacket::<V975> {
            response: ResourcePackResponse::ResourcePackStackFinished,
            downloading_packs: vec![],
        },
    )));
    round_trip(V975::SetLocalPlayerAsInitializedPacket(Box::new(
        SetLocalPlayerAsInitializedPacket::<V975> {
            player_id: ActorRuntimeID(1),
        },
    )));
    round_trip(V975::RequestChunkRadiusPacket(Box::new(
        RequestChunkRadiusPacket {
            chunk_radius: 4,
            max_chunk_radius: 4,
        },
    )));
    round_trip(V975::NetworkStackLatencyPacket(Box::new(
        NetworkStackLatencyPacket {
            creation_time: 42,
            is_from_server: false,
        },
    )));
    round_trip(V975::TextPacket(Box::new(TextPacket::<V975> {
        localize: false,
        message_type: TextPacketType::Chat {
            player_name: "RustRock".to_string(),
            message: "validation".to_string(),
        },
        sender_xuid: String::new(),
        platform_id: String::new(),
        filtered_message: None,
    })));
    round_trip(V975::ModalFormResponsePacket(Box::new(
        bedrock::protocol::v662::packets::ModalFormResponsePacket::<V975> {
            form_id: 7,
            json_response: Some("null".to_string()),
            form_cancel_reason: None,
        },
    )));
    round_trip(V975::InventoryTransactionPacket(Box::new(
        InventoryTransactionPacket::<V975> {
            raw_id: 0,
            legacy_set_item_slots: vec![],
            transaction_type: ComplexInventoryTransactionType::NormalTransaction,
            transaction: InventoryTransaction::<V975> { action: vec![] },
        },
    )));
    round_trip(V975::MovePlayerPacket(Box::new(MovePlayerPacket::<V975> {
        player_runtime_id: ActorRuntimeID(1),
        position: (0.0, 64.0, 0.0),
        rotation: (0.0, 0.0),
        y_head_rotation: 0.0,
        position_mode: PlayerPositionMode::Normal,
        on_ground: true,
        riding_runtime_id: ActorRuntimeID(0),
        tick: 1,
    })));
    round_trip(V975::PlayerAuthInputPacket(Box::new(
        PlayerAuthInputPacket::<V975> {
            player_rotation: (0.0, 0.0),
            player_position: (0.0, 64.0, 0.0),
            move_vector: (0.0, 0.0, 0.0),
            player_head_rotation: 0.0,
            input_data: 0,
            input_mode: InputMode::Mouse,
            play_mode: ClientPlayMode::Normal,
            new_interaction_model: NewInteractionModel::Crosshair,
            interact_rotation: (0.0, 0.0),
            client_tick: 1,
            velocity: (0.0, 0.0, 0.0),
            item_use_transaction: None,
            item_stack_request: None,
            player_block_actions: None,
            client_predicted_vehicle: None,
            analog_move_vector: (0.0, 0.0),
            camera_orientation: (0.0, 0.0, 0.0),
            raw_move_vector: (0.0, 0.0),
        },
    )));
    round_trip(V975::DisconnectPacket(Box::new(DisconnectPacket::<V975> {
        reason: ConnectionFailReason::NoReason,
        message: Some(DisconnectMessage {
            kick_message: "bye".to_string(),
            filtered_message: "bye".to_string(),
        }),
    })));
}

#[test]
fn unknown_request_network_settings_round_trips() {
    let packet = Unknown::RequestNetworkSettingsPacket(Box::new(RequestNetworkSettingsPacket {
        client_network_version: V975::PROTOCOL_VERSION as i32,
    }));
    let encoded = encode_packets::<Unknown>(&[packet], None, None).unwrap();
    let decoded = decode_packets::<Unknown>(encoded, None, None).unwrap();
    assert!(matches!(
        decoded.first(),
        Some(Unknown::RequestNetworkSettingsPacket(_))
    ));
}

fn round_trip(packet: V975) {
    let encoded = encode_packets::<V975>(&[packet], None, None).unwrap();
    let decoded = decode_packets::<V975>(encoded, None, None).unwrap();
    assert_eq!(decoded.len(), 1);
}
