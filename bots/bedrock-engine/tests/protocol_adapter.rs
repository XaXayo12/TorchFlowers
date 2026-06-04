use bedrock::{
    network::codec::{decode_packets, encode_packets},
    protocol::{
        ProtoVersion, Unknown, V975 as BedrockProto,
        unknown::packets::RequestNetworkSettingsPacket,
        v662::{
            enums::{
                ComplexInventoryTransactionType, ConnectionFailReason, InputMode,
                NewInteractionModel, PlayerPositionMode, ResourcePackResponse,
            },
            packets::{
                ClientCacheStatusPacket, ClientToServerHandshakePacket, InventoryTransactionPacket,
                MovePlayerPacket, NetworkStackLatencyPacket, RequestChunkRadiusPacket,
                ResourcePackClientResponsePacket, ServerToClientHandshakePacket,
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
use bedrock_engine::{
    auth::minecraft::MinecraftAuth, bedrock::protocol_adapter::BedrockProtocolAdapter,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use p384::{
    PublicKey, SecretKey,
    pkcs8::{DecodePrivateKey, DecodePublicKey},
};

#[test]
fn bedrock_rs_round_trips_validation_packet_surface() {
    round_trip(BedrockProto::LoginPacket(Box::new(
        bedrock::protocol::v662::packets::LoginPacket {
            client_network_version: BedrockProto::PROTOCOL_VERSION as i32,
            connection_request: br#"{"chain":[],"skin":""}"#.to_vec(),
        },
    )));
    round_trip(BedrockProto::ServerToClientHandshakePacket(Box::new(
        ServerToClientHandshakePacket {
            handshake_web_token: "header.payload.signature".to_string(),
        },
    )));
    round_trip(BedrockProto::ClientToServerHandshakePacket(Box::new(
        ClientToServerHandshakePacket {},
    )));
    round_trip(BedrockProto::ResourcePackClientResponsePacket(Box::new(
        ResourcePackClientResponsePacket::<BedrockProto> {
            response: ResourcePackResponse::ResourcePackStackFinished,
            downloading_packs: vec![],
        },
    )));
    round_trip(BedrockProto::SetLocalPlayerAsInitializedPacket(Box::new(
        SetLocalPlayerAsInitializedPacket::<BedrockProto> {
            player_id: ActorRuntimeID(1),
        },
    )));
    round_trip(BedrockProto::RequestChunkRadiusPacket(Box::new(
        RequestChunkRadiusPacket {
            chunk_radius: 4,
            max_chunk_radius: 4,
        },
    )));
    round_trip(BedrockProto::NetworkStackLatencyPacket(Box::new(
        NetworkStackLatencyPacket {
            creation_time: 42,
            is_from_server: false,
        },
    )));
    round_trip(BedrockProto::TextPacket(Box::new(TextPacket::<
        BedrockProto,
    > {
        localize: false,
        message_type: TextPacketType::Chat {
            player_name: "RustRock".to_string(),
            message: "validation".to_string(),
        },
        sender_xuid: String::new(),
        platform_id: String::new(),
        filtered_message: None,
    })));
    round_trip(BedrockProto::ModalFormResponsePacket(Box::new(
        bedrock::protocol::v662::packets::ModalFormResponsePacket::<BedrockProto> {
            form_id: 7,
            json_response: Some("null".to_string()),
            form_cancel_reason: None,
        },
    )));
    round_trip(BedrockProto::InventoryTransactionPacket(Box::new(
        InventoryTransactionPacket::<BedrockProto> {
            raw_id: 0,
            legacy_set_item_slots: vec![],
            transaction_type: ComplexInventoryTransactionType::NormalTransaction,
            transaction: InventoryTransaction::<BedrockProto> { action: vec![] },
        },
    )));
    round_trip(BedrockProto::MovePlayerPacket(Box::new(
        MovePlayerPacket::<BedrockProto> {
            player_runtime_id: ActorRuntimeID(1),
            position: (0.0, 64.0, 0.0),
            rotation: (0.0, 0.0),
            y_head_rotation: 0.0,
            position_mode: PlayerPositionMode::Normal,
            on_ground: true,
            riding_runtime_id: ActorRuntimeID(0),
            tick: 1,
        },
    )));
    round_trip(BedrockProto::PlayerAuthInputPacket(Box::new(
        PlayerAuthInputPacket::<BedrockProto> {
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
    round_trip(BedrockProto::DisconnectPacket(Box::new(
        DisconnectPacket::<BedrockProto> {
            reason: ConnectionFailReason::NoReason,
            message: Some(DisconnectMessage {
                kick_message: "bye".to_string(),
                filtered_message: "bye".to_string(),
            }),
        },
    )));
}

#[test]
fn resource_pack_info_response_batch_matches_bedrock_state_machine() {
    let encoded = encode_packets::<BedrockProto>(
        &[
            BedrockProto::ResourcePackClientResponsePacket(Box::new(
                ResourcePackClientResponsePacket::<BedrockProto> {
                    response: ResourcePackResponse::DownloadingFinished,
                    downloading_packs: vec![],
                },
            )),
            BedrockProto::ClientCacheStatusPacket(Box::new(ClientCacheStatusPacket {
                is_cache_supported: false,
            })),
        ],
        None,
        None,
    )
    .unwrap();

    let mut cursor = 0usize;
    let response_len = read_unsigned_varint_u32(&encoded, &mut cursor) as usize;
    let response_end = cursor + response_len;
    let response_header = read_unsigned_varint_u32(&encoded, &mut cursor);
    assert_eq!(response_header & 0x3ff, 8);
    assert_eq!(
        encoded[cursor], 3,
        "ResourcePacksInfo response must mean have_all_packs"
    );
    cursor += 1;
    assert_eq!(&encoded[cursor..cursor + 2], &[0, 0], "empty pack list");
    cursor = response_end;

    let cache_len = read_unsigned_varint_u32(&encoded, &mut cursor) as usize;
    let cache_end = cursor + cache_len;
    let cache_header = read_unsigned_varint_u32(&encoded, &mut cursor);
    assert_eq!(cache_header & 0x3ff, 129);
    assert_eq!(encoded[cursor], 0, "client cache must be disabled");
    cursor += 1;
    assert_eq!(cursor, cache_end);
    assert_eq!(cursor, encoded.len());
}

#[test]
fn resource_pack_stack_response_is_stack_finished() {
    let encoded = encode_packets::<BedrockProto>(
        &[BedrockProto::ResourcePackClientResponsePacket(Box::new(
            ResourcePackClientResponsePacket::<BedrockProto> {
                response: ResourcePackResponse::ResourcePackStackFinished,
                downloading_packs: vec![],
            },
        ))],
        None,
        None,
    )
    .unwrap();

    let mut cursor = 0usize;
    let response_len = read_unsigned_varint_u32(&encoded, &mut cursor) as usize;
    let response_end = cursor + response_len;
    let response_header = read_unsigned_varint_u32(&encoded, &mut cursor);
    assert_eq!(response_header & 0x3ff, 8);
    assert_eq!(
        encoded[cursor], 4,
        "ResourcePackStack response must be stack_finished"
    );
    cursor += 1;
    assert_eq!(&encoded[cursor..cursor + 2], &[0, 0], "empty pack list");
    cursor += 2;
    assert_eq!(cursor, response_end);
    assert_eq!(cursor, encoded.len());
}

#[test]
fn adapter_derives_bedrock_encryption_from_server_handshake() {
    let fake_mojang_key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";
    let fake_header = serde_json::json!({"alg": "ES384", "x5u": fake_mojang_key});
    let fake_header_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        fake_header.to_string().as_bytes(),
    );
    let fake_chain_jwt = format!("{}.payload.sig", fake_header_b64);
    let (client_signing_key, client_private_key_pem, client_public_key) =
        MinecraftAuth::generate_device_keypair().unwrap();
    let client_chain = MinecraftAuth::build_jwt_chain(
        vec![fake_chain_jwt],
        client_signing_key,
        client_private_key_pem,
        client_public_key,
        "RustRock",
        "12345",
        Some("test-playfab-id"),
    )
    .unwrap();
    let (_, server_private_key_pem, server_public_key) =
        MinecraftAuth::generate_device_keypair().unwrap();
    let salt = [9_u8; 16];
    let mut header = Header::new(Algorithm::ES384);
    header.x5u = Some(server_public_key);
    let token = encode(
        &header,
        &serde_json::json!({ "salt": base64_standard(&salt) }),
        &EncodingKey::from_ec_pem(server_private_key_pem.as_bytes()).unwrap(),
    )
    .unwrap();

    let mut client_encryption =
        BedrockProtocolAdapter::derive_encryption_from_handshake(&client_chain, &token).unwrap();

    let client_public_der = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &client_chain.public_key_der_base64,
    )
    .unwrap();
    let client_public = PublicKey::from_public_key_der(&client_public_der).unwrap();
    let server_private = SecretKey::from_pkcs8_pem(&server_private_key_pem).unwrap();
    let mut server_encryption =
        bedrock::network::encryption::Encryption::new(&server_private, &client_public, &salt);

    let plaintext = vec![1, 2, 3, 4, 5, 6];
    let encrypted = client_encryption.encrypt(plaintext.clone()).unwrap();
    let decrypted = server_encryption.decrypt(encrypted).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn unknown_request_network_settings_round_trips() {
    let packet = Unknown::RequestNetworkSettingsPacket(Box::new(RequestNetworkSettingsPacket {
        client_network_version: BedrockProto::PROTOCOL_VERSION as i32,
    }));
    let encoded = encode_packets::<Unknown>(&[packet], None, None).unwrap();
    let decoded = decode_packets::<Unknown>(encoded, None, None).unwrap();
    assert!(matches!(
        decoded.first(),
        Some(Unknown::RequestNetworkSettingsPacket(_))
    ));
}

#[test]
fn adapter_login_encoding_matches_bedrock_protocol_encapsulated_tokens() {
    let connection_request = vec![0x11, 0x22, 0x33, 0x44, b'{', b'}'];
    let encoded =
        BedrockProtocolAdapter::encode_login_packet_batch(&connection_request, None).unwrap();

    let mut cursor = 0usize;
    let packet_len = read_unsigned_varint_u32(&encoded, &mut cursor) as usize;
    assert_eq!(packet_len, encoded.len() - cursor);

    let packet_id = read_unsigned_varint_u32(&encoded, &mut cursor);
    assert_eq!(packet_id, 1);
    assert_eq!(&encoded[cursor..cursor + 4], &898_i32.to_be_bytes());
    cursor += 4;

    let token_len = read_unsigned_varint_u32(&encoded, &mut cursor) as usize;
    assert_eq!(token_len, connection_request.len());
    assert_eq!(&encoded[cursor..], connection_request.as_slice());
}

fn round_trip(packet: BedrockProto) {
    let encoded = encode_packets::<BedrockProto>(&[packet], None, None).unwrap();
    let decoded = decode_packets::<BedrockProto>(encoded, None, None).unwrap();
    assert_eq!(decoded.len(), 1);
}

fn base64_standard(bytes: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
}

fn read_unsigned_varint_u32(data: &[u8], cursor: &mut usize) -> u32 {
    let mut value = 0u32;
    let mut shift = 0u32;
    loop {
        let byte = data[*cursor];
        *cursor += 1;
        value |= u32::from(byte & 0x7f) << shift;
        if (byte & 0x80) == 0 {
            return value;
        }
        shift += 7;
    }
}
