use bedrock_engine::{
    auth::{
        minecraft::MinecraftAuth,
        xsts::{BEDROCK_RELYING_PARTY, PLAYFAB_RELYING_PARTY},
    },
    db::Database,
};

#[test]
fn auth_contract_uses_required_relying_parties() {
    assert_eq!(BEDROCK_RELYING_PARTY, "https://multiplayer.minecraft.net/");
    assert_eq!(PLAYFAB_RELYING_PARTY, "http://playfab.xboxlive.com/");
}

#[test]
fn jwt_chain_generator_creates_connection_request() {
    let (signing_key, private_key, public_key) = MinecraftAuth::generate_device_keypair().unwrap();
    let chain = MinecraftAuth::build_jwt_chain(
        vec!["legacy-chain-segment".to_string()],
        signing_key,
        private_key,
        public_key,
        "RustRock",
        "12345",
    )
    .unwrap();
    let request = MinecraftAuth::connection_request(&chain).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&request).unwrap();
    assert!(value["chain"].as_array().unwrap().len() >= 2);
    assert!(value["skin"].as_str().unwrap().contains('.'));
}

#[tokio::test]
async fn sqlite_schema_migrates() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    let accounts = db.list_accounts().await.unwrap();
    assert!(accounts.is_empty());
}
