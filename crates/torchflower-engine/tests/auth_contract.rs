use torchflower_engine::{
    auth::{
        minecraft::{MinecraftAuth, PRISMARINE_MOJANG_PUBLIC_KEY},
        xsts::{BEDROCK_RELYING_PARTY, PLAYFAB_RELYING_PARTY},
    },
    config::{
        MicrosoftAuthFlow, BEDROCK_PROTOCOL_LIVE_CLIENT_ID, BEDROCK_PROTOCOL_LIVE_SCOPE, MSAL_SCOPE,
    },
    db::Database,
};

#[test]
fn auth_contract_uses_required_relying_parties() {
    assert_eq!(BEDROCK_RELYING_PARTY, "https://multiplayer.minecraft.net/");
    assert_eq!(PLAYFAB_RELYING_PARTY, "http://playfab.xboxlive.com/");
}

#[test]
fn microsoft_auth_defaults_to_bedrock_protocol_live_flow() {
    let flow = MicrosoftAuthFlow::from_env_value(None).unwrap();
    assert_eq!(flow, MicrosoftAuthFlow::Live);
    assert_eq!(BEDROCK_PROTOCOL_LIVE_CLIENT_ID, "00000000441cc96b");
    assert_eq!(
        flow.device_code_url(),
        "https://login.live.com/oauth20_connect.srf"
    );
    assert_eq!(flow.token_url(), "https://login.live.com/oauth20_token.srf");
    assert_eq!(flow.scope(), BEDROCK_PROTOCOL_LIVE_SCOPE);
    assert_eq!(MicrosoftAuthFlow::Msal.scope(), MSAL_SCOPE);
}

#[test]
fn jwt_chain_generator_creates_connection_request() {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    // Build a fake chain[0] JWT header with x5u set to a dummy Mojang root CA key.
    let fake_mojang_key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";
    let fake_header = serde_json::json!({"alg": "ES384", "x5u": fake_mojang_key});
    let fake_header_b64 = URL_SAFE_NO_PAD.encode(fake_header.to_string().as_bytes());
    let fake_payload = serde_json::json!({
        "extraData": {
            "displayName": "DonutProfile",
            "XUID": "987654321"
        }
    });
    let fake_payload_b64 = URL_SAFE_NO_PAD.encode(fake_payload.to_string().as_bytes());
    let fake_chain_jwt = format!("{}.{}.sig", fake_header_b64, fake_payload_b64);

    let (signing_key, private_key, public_key) = MinecraftAuth::generate_device_keypair().unwrap();
    let chain = MinecraftAuth::build_jwt_chain(
        vec![fake_chain_jwt],
        signing_key,
        private_key,
        public_key.clone(),
        "TorchFlower",
        "12345",
        Some("test-playfab-id"),
    )
    .unwrap();
    assert_eq!(chain.display_name, "DonutProfile");
    assert_eq!(chain.xuid, "987654321");

    // Client JWT is prepended: chain[0] = client JWT, chain[1] = original Mojang JWT
    assert_eq!(
        chain.chain.len(),
        2,
        "chain should have 2 elements after prepend"
    );

    // Decode chain[0] (the client JWT) payload and verify it has certificateAuthority + identityPublicKey
    let client_jwt_parts: Vec<&str> = chain.chain[0].split('.').collect();
    assert_eq!(client_jwt_parts.len(), 3, "client JWT must be 3-part");
    let payload_bytes = URL_SAFE_NO_PAD.decode(client_jwt_parts[1]).unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).unwrap();
    assert_eq!(payload["certificateAuthority"].as_bool(), Some(true));
    assert_eq!(
        payload["identityPublicKey"].as_str(),
        Some(PRISMARINE_MOJANG_PUBLIC_KEY)
    );
    assert!(payload.get("iat").is_some());
    assert!(payload.get("exp").is_none());
    assert!(payload.get("nbf").is_none());

    // chain[1] must still be the original Mojang JWT
    assert!(chain.chain[1].starts_with(&fake_header_b64));

    // Verify the binary connection_request layout
    let request = MinecraftAuth::connection_request(
        &chain,
        "MCToken test-token",
        "play.example.com:19132",
        Some("test-playfab-id"),
    )
    .unwrap();
    let fingerprint = MinecraftAuth::connection_request_fingerprint(&request).unwrap();
    assert_eq!(fingerprint.authentication_type, Some(0));
    assert_eq!(fingerprint.certificate_chain_count, 2);
    assert_eq!(fingerprint.legacy_token_len, "MCToken test-token".len());
    assert_eq!(
        fingerprint.server_address.as_deref(),
        Some("play.example.com:19132")
    );
    assert_eq!(fingerprint.game_version.as_deref(), Some("1.21.130"));
    assert_eq!(
        fingerprint.third_party_name.as_deref(),
        Some("DonutProfile")
    );
    let fingerprint_playfab_id = fingerprint.playfab_id.as_deref().unwrap();
    assert_eq!(fingerprint_playfab_id.len(), 16);
    assert_ne!(fingerprint_playfab_id, "test-playfab-id");
    assert!(fingerprint_playfab_id
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    assert_eq!(fingerprint.device_model.as_deref(), Some("PrismarineJS"));
    assert_eq!(fingerprint.device_os, Some(7));
    assert_eq!(fingerprint.persona_skin, Some(true));
    assert_eq!(fingerprint.skin_image_width, Some(256));
    assert_eq!(fingerprint.skin_image_height, Some(256));
    assert!(!fingerprint.client_data_has_exp);
    assert!(!fingerprint.client_data_has_nbf);
    assert!(!fingerprint.client_data_has_iat);
    assert_eq!(fingerprint.skin_data_decoded_len, Some(262_144));
    assert_eq!(
        fingerprint.skin_geometry_engine_version.as_deref(),
        Some("1.14.0")
    );
    assert!(fingerprint
        .skin_resource_patch_decoded
        .as_deref()
        .unwrap()
        .contains("geometry.persona"));
    let auth_info_len = u32::from_le_bytes(request[0..4].try_into().unwrap()) as usize;
    let auth_info_json = String::from_utf8(request[4..4 + auth_info_len].to_vec()).unwrap();
    assert!(
        auth_info_json.starts_with(r#"{"Certificate":"#),
        "auth info should match bedrock-protocol's field order"
    );
    let auth_info: serde_json::Value = serde_json::from_str(&auth_info_json).unwrap();
    let certificate_json = auth_info["Certificate"].as_str().unwrap();
    let certificate: serde_json::Value = serde_json::from_str(certificate_json).unwrap();
    assert_eq!(certificate["chain"].as_array().unwrap().len(), 2);
    assert_eq!(auth_info["Token"].as_str().unwrap(), "MCToken test-token");
    let skin_offset = 4 + auth_info_len;
    let skin_len =
        u32::from_le_bytes(request[skin_offset..skin_offset + 4].try_into().unwrap()) as usize;
    let skin_jwt =
        String::from_utf8(request[skin_offset + 4..skin_offset + 4 + skin_len].to_vec()).unwrap();
    assert!(skin_jwt.contains('.'));
    let skin_payload_b64 = skin_jwt.split('.').nth(1).unwrap();
    let skin_payload_bytes = URL_SAFE_NO_PAD.decode(skin_payload_b64).unwrap();
    let skin_payload: serde_json::Value = serde_json::from_slice(&skin_payload_bytes).unwrap();
    assert_eq!(
        skin_payload["ServerAddress"].as_str(),
        Some("play.example.com:19132")
    );
    assert_eq!(skin_payload["GameVersion"].as_str(), Some("1.21.130"));
    assert_eq!(
        skin_payload["ThirdPartyName"].as_str(),
        Some("DonutProfile")
    );
    let skin_playfab_id = skin_payload["PlayFabId"].as_str().unwrap();
    assert_eq!(skin_playfab_id.len(), 16);
    assert_ne!(skin_playfab_id, "test-playfab-id");
    assert!(skin_payload.get("exp").is_none());
    assert!(skin_payload.get("nbf").is_none());
    assert!(skin_payload.get("iat").is_none());
}

#[tokio::test]
async fn sqlite_schema_migrates() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    let accounts = db.list_accounts().await.unwrap();
    assert!(accounts.is_empty());
}

#[tokio::test]
async fn account_import_reuses_existing_email_case_insensitively() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    let first = db
        .upsert_account_email("VaibhavPrakhar4@gmail.com")
        .await
        .unwrap();
    let second = db
        .upsert_account_email("vaibhavprakhar4@gmail.com")
        .await
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(db.list_accounts().await.unwrap().len(), 1);
}

#[tokio::test]
async fn entitlement_retry_state_is_persisted_and_reset_on_success() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    let account_id = db.upsert_account_email("retry@example.com").await.unwrap();

    db.begin_entitlement_provisioning(&account_id)
        .await
        .unwrap();
    let first_retry = db
        .record_entitlement_provisioning_failure(&account_id, "playfab unavailable")
        .await
        .unwrap();
    assert_eq!(first_retry.retry_count, 1);
    assert!(first_retry.next_retry_at.is_some());

    db.begin_entitlement_provisioning(&account_id)
        .await
        .unwrap();
    let second_retry = db
        .record_entitlement_provisioning_failure(&account_id, "session/start unavailable")
        .await
        .unwrap();
    assert_eq!(second_retry.retry_count, 2);
    assert_eq!(
        db.entitlement_retry_state(&account_id)
            .await
            .unwrap()
            .last_error
            .as_deref(),
        Some("session/start unavailable")
    );

    db.upsert_entitlement(
        &account_id,
        true,
        Some("playfab-id"),
        Some("encrypted-session-ticket"),
        Some("encrypted-minecraft-token"),
        "provisioned",
        Some("request-id"),
        None,
    )
    .await
    .unwrap();
    let success_state = db.entitlement_retry_state(&account_id).await.unwrap();
    assert_eq!(success_state.retry_count, 0);
    assert!(success_state.next_retry_at.is_none());
    assert!(success_state.last_error.is_none());

    let entitlements = db.list_entitlements().await.unwrap();
    assert_eq!(entitlements.len(), 1);
    assert_eq!(entitlements[0].account_id, account_id);
    assert_eq!(entitlements[0].account_email, "retry@example.com");
    assert!(entitlements[0].has_entitlement);
    assert_eq!(entitlements[0].playfab_id.as_deref(), Some("playfab-id"));
    assert_eq!(entitlements[0].provisioning_status, "provisioned");
    assert_eq!(
        entitlements[0].last_request_id.as_deref(),
        Some("request-id")
    );
}

#[tokio::test]
async fn bot_runtime_state_tracks_join_leave_position_and_inventory() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    let account_id = db
        .upsert_account_email("bot-state@example.com")
        .await
        .unwrap();
    let server_id = db.create_server("local", "127.0.0.1", 19132).await.unwrap();
    let bot_id = db.create_bot(&account_id, &server_id).await.unwrap();

    db.mark_bot_joined(&bot_id).await.unwrap();
    db.update_bot_runtime_state(
        &bot_id,
        Some("1.000,64.000,2.000"),
        Some(&serde_json::json!({ "last_event": "inventory_slot", "slot": 0 })),
    )
    .await
    .unwrap();

    let bots = db.list_bots().await.unwrap();
    let bot = bots.iter().find(|bot| bot.id == bot_id).unwrap();
    assert_eq!(bot.status, "connected");
    assert_eq!(bot.current_position.as_deref(), Some("1.000,64.000,2.000"));
    assert_eq!(bot.inventory_json["last_event"], "inventory_slot");

    db.mark_bot_left(&bot_id, "stopped", None).await.unwrap();
    let bot = db
        .list_bots()
        .await
        .unwrap()
        .into_iter()
        .find(|bot| bot.id == bot_id)
        .unwrap();
    assert_eq!(bot.status, "stopped");
    assert!(bot.last_error.is_none());
}
