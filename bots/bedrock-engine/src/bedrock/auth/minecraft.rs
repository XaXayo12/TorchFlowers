use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use p384::{
    ecdsa::SigningKey,
    elliptic_curve::Generate,
    pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    auth::XstsToken,
    db::Database,
    diagnostics::Diagnostics,
    error::{EngineError, EngineResult},
};

const CLASSIC_SKIN_WIDTH: usize = 64;
const CLASSIC_SKIN_HEIGHT: usize = 64;
const PRISMARINE_DEFAULT_GAME_VERSION: &str = "1.21.130";
pub const PRISMARINE_MOJANG_PUBLIC_KEY: &str = "MHYwEAYHKoZIzj0CAQYFK4EEACIDYgAECRXueJeTDqNRRgJi/vlRufByu/2G0i2Ebt6YMar5QX/R0DIIyrJMcUpruK4QveTfJSTp3Shlq4Gk34cD/4GUWwkv0DVuzeuB+tXija7HBxii03NHDbPAD0AKnLr2wdAp";

#[derive(Clone)]
pub struct MinecraftAuth {
    client: reqwest::Client,
    diagnostics: Diagnostics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedrockJwtChain {
    pub chain: Vec<String>,
    pub skin: String,
    pub public_key_der_base64: String,
    pub private_key_pem: String,
    pub display_name: String,
    pub xuid: String,
    pub title_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LegacyAuthResponse {
    chain: Vec<String>,
    #[serde(default, alias = "Token")]
    token: String,
}

#[derive(Debug, Clone)]
pub struct LegacyBedrockAuth {
    pub chain: Vec<String>,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LoginRequestFingerprint {
    pub authentication_type: Option<u64>,
    pub auth_info_len: usize,
    pub certificate_len: usize,
    pub certificate_chain_count: usize,
    pub legacy_token_len: usize,
    pub client_data_jwt_len: usize,
    pub client_data_claim_count: usize,
    pub client_data_has_iat: bool,
    pub client_data_has_exp: bool,
    pub client_data_has_nbf: bool,
    pub game_version: Option<String>,
    pub server_address: Option<String>,
    pub third_party_name: Option<String>,
    pub playfab_id: Option<String>,
    pub device_model: Option<String>,
    pub device_os: Option<i64>,
    pub persona_skin: Option<bool>,
    pub skin_image_width: Option<i64>,
    pub skin_image_height: Option<i64>,
    pub skin_data_base64_len: usize,
    pub skin_data_decoded_len: Option<usize>,
    pub skin_geometry_engine_version: Option<String>,
    pub skin_resource_patch_decoded: Option<String>,
}

#[derive(Serialize)]
struct LoginAuthInfo<'a> {
    #[serde(rename = "Certificate")]
    certificate: &'a str,
    #[serde(rename = "AuthenticationType")]
    authentication_type: u8,
    #[serde(rename = "Token")]
    token: &'a str,
}

impl MinecraftAuth {
    pub fn new(db: Database) -> Self {
        Self {
            client: reqwest::Client::new(),
            diagnostics: Diagnostics::new(db),
        }
    }

    pub async fn legacy_bedrock_auth(
        &self,
        account_id: &str,
        standard_xsts: &XstsToken,
        identity_public_key: &str,
    ) -> EngineResult<LegacyBedrockAuth> {
        let body = json!({ "identityPublicKey": identity_public_key });
        let authorization = format!(
            "XBL3.0 x={};{}",
            standard_xsts.user_hash, standard_xsts.token
        );
        let (response, _, _) = self
            .diagnostics
            .request_json::<LegacyAuthResponse>(
                &self.client,
                Some(account_id),
                "legacy_bedrock_authentication",
                Method::POST,
                "https://multiplayer.minecraft.net/authentication",
                vec![
                    ("Authorization", authorization),
                    ("User-Agent", "MCPE/UWP".to_string()),
                ],
                body,
            )
            .await?;
        Ok(LegacyBedrockAuth {
            chain: response.chain,
            token: response.token,
        })
    }

    pub fn generate_device_keypair() -> EngineResult<(SigningKey, String, String)> {
        let signing_key = SigningKey::generate();
        let private_pem = signing_key
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|err| EngineError::Crypto(err.to_string()))?
            .to_string();
        let public_der = signing_key
            .verifying_key()
            .to_public_key_der()
            .map_err(|err| EngineError::Crypto(err.to_string()))?;
        let public_der_base64 = STANDARD.encode(public_der.as_bytes());
        Ok((signing_key, private_pem, public_der_base64))
    }

    pub fn build_jwt_chain(
        legacy_chain: Vec<String>,
        signing_key: SigningKey,
        private_key_pem: String,
        public_key_der_base64: String,
        display_name: &str,
        xuid: &str,
        playfab_id: Option<&str>,
    ) -> EngineResult<BedrockJwtChain> {
        // The legacy chain is [mojang_root_ca_jwt, mojang_user_jwt].
        let legacy_profile = legacy_profile_from_chain(&legacy_chain);
        let display_name = legacy_profile
            .display_name
            .as_deref()
            .unwrap_or(display_name);
        let xuid = legacy_profile.xuid.as_deref().unwrap_or(xuid);

        if legacy_chain.is_empty() {
            return Err(EngineError::Crypto("legacy chain is empty".to_string()));
        }

        // ── Step 2: build the client linking JWT ──
        // header.x5u  = client's own public key (proves who signed this JWT)
        // payload.identityPublicKey matches bedrock-protocol's Mojang root key.
        // payload.certificateAuthority = true (required by server)
        let encoding_key = EncodingKey::from_ec_pem(private_key_pem.as_bytes())
            .map_err(|e| EngineError::Crypto(e.to_string()))?;
        let now = Utc::now().timestamp();

        let mut client_claims = serde_json::Map::<String, Value>::new();
        client_claims.insert(
            "identityPublicKey".to_string(),
            json!(PRISMARINE_MOJANG_PUBLIC_KEY),
        );
        client_claims.insert("certificateAuthority".to_string(), json!(true));
        client_claims.insert("iat".to_string(), json!(now));
        if use_wide_client_identity_times() {
            client_claims.insert("exp".to_string(), json!(now + 21_600));
            client_claims.insert("nbf".to_string(), json!(now - 21_600));
        }
        let client_claims = Value::Object(client_claims);
        let mut header = Header::new(Algorithm::ES384);
        header.typ = None;
        header.x5u = Some(public_key_der_base64.clone());
        let client_jwt = encode(&header, &client_claims, &encoding_key)
            .map_err(|e| EngineError::Crypto(e.to_string()))?;

        // ── Step 3: prepend client JWT so chain order is [client_jwt, chain[0], chain[1]] ──
        // The server reads: chain[0] (client) → chain[1] (Mojang root CA) → chain[2] (Mojang user+extraData)
        let mut chain = legacy_chain;
        chain.insert(0, client_jwt);
        let _ = signing_key;

        // ── Step 4: build skin / client-data JWT ──
        let skin_claims = client_data_claims(
            display_name,
            "",
            playfab_id,
            legacy_profile.title_id.as_deref(),
        )?;
        let skin = encode(&header, &skin_claims, &encoding_key)
            .map_err(|e| EngineError::Crypto(e.to_string()))?;

        Ok(BedrockJwtChain {
            chain,
            skin,
            public_key_der_base64,
            private_key_pem,
            display_name: display_name.to_string(),
            xuid: xuid.to_string(),
            title_id: legacy_profile.title_id,
        })
    }

    pub fn client_data_jwt_for_server(
        chain: &BedrockJwtChain,
        server_address: &str,
        playfab_id: Option<&str>,
    ) -> EngineResult<String> {
        if server_address.is_empty() {
            return Ok(chain.skin.clone());
        }

        let encoding_key = EncodingKey::from_ec_pem(chain.private_key_pem.as_bytes())
            .map_err(|e| EngineError::Crypto(e.to_string()))?;
        let mut header = Header::new(Algorithm::ES384);
        header.typ = None;
        header.x5u = Some(chain.public_key_der_base64.clone());

        let skin_claims = client_data_claims(
            &chain.display_name,
            server_address,
            playfab_id,
            chain.title_id.as_deref(),
        )?;

        encode(&header, &skin_claims, &encoding_key).map_err(|e| EngineError::Crypto(e.to_string()))
    }

    pub fn connection_request(
        chain: &BedrockJwtChain,
        legacy_bedrock_token: &str,
        server_address: &str,
        playfab_id: Option<&str>,
    ) -> EngineResult<Vec<u8>> {
        let certificate = serde_json::to_string(&json!({
            "chain": chain.chain
        }))?;

        let auth_info = LoginAuthInfo {
            certificate: &certificate,
            authentication_type: 0,
            token: legacy_bedrock_token,
        };
        let auth_info_json = serde_json::to_string(&auth_info)?;
        let client_data_jwt = Self::client_data_jwt_for_server(chain, server_address, playfab_id)?;

        let mut buf = Vec::new();

        // Write auth_info_json length as u32 LE
        buf.extend_from_slice(&(auth_info_json.len() as u32).to_le_bytes());
        // Write auth_info_json bytes
        buf.extend_from_slice(auth_info_json.as_bytes());

        // Write client_data_jwt length as u32 LE
        buf.extend_from_slice(&(client_data_jwt.len() as u32).to_le_bytes());
        // Write client_data_jwt bytes
        buf.extend_from_slice(client_data_jwt.as_bytes());

        Ok(buf)
    }

    pub fn connection_request_fingerprint(
        connection_request: &[u8],
    ) -> EngineResult<LoginRequestFingerprint> {
        let mut cursor = 0usize;
        let auth_info_json = read_le_string(connection_request, &mut cursor, "auth info")?;
        let client_data_jwt = read_le_string(connection_request, &mut cursor, "client data JWT")?;

        let auth_info: Value = serde_json::from_str(&auth_info_json)?;
        let certificate = auth_info
            .get("Certificate")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let certificate_json: Value =
            serde_json::from_str(certificate).unwrap_or_else(|_| json!({ "chain": [] }));
        let certificate_chain_count = certificate_json
            .get("chain")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);

        let client_data = decode_jwt_payload(&client_data_jwt)?;
        let skin_data = client_data
            .get("SkinData")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let skin_resource_patch_decoded = client_data
            .get("SkinResourcePatch")
            .and_then(Value::as_str)
            .and_then(|value| STANDARD.decode(value).ok())
            .and_then(|bytes| String::from_utf8(bytes).ok());
        let skin_geometry_engine_version = client_data
            .get("SkinGeometryDataEngineVersion")
            .and_then(Value::as_str)
            .and_then(|value| STANDARD.decode(value).ok())
            .and_then(|bytes| String::from_utf8(bytes).ok());

        Ok(LoginRequestFingerprint {
            authentication_type: auth_info.get("AuthenticationType").and_then(Value::as_u64),
            auth_info_len: auth_info_json.len(),
            certificate_len: certificate.len(),
            certificate_chain_count,
            legacy_token_len: auth_info
                .get("Token")
                .and_then(Value::as_str)
                .map_or(0, str::len),
            client_data_jwt_len: client_data_jwt.len(),
            client_data_claim_count: client_data.as_object().map_or(0, serde_json::Map::len),
            client_data_has_iat: client_data.get("iat").is_some(),
            client_data_has_exp: client_data.get("exp").is_some(),
            client_data_has_nbf: client_data.get("nbf").is_some(),
            game_version: string_claim(&client_data, "GameVersion"),
            server_address: string_claim(&client_data, "ServerAddress"),
            third_party_name: string_claim(&client_data, "ThirdPartyName"),
            playfab_id: string_claim(&client_data, "PlayFabId"),
            device_model: string_claim(&client_data, "DeviceModel"),
            device_os: client_data.get("DeviceOS").and_then(Value::as_i64),
            persona_skin: client_data.get("PersonaSkin").and_then(Value::as_bool),
            skin_image_width: client_data.get("SkinImageWidth").and_then(Value::as_i64),
            skin_image_height: client_data.get("SkinImageHeight").and_then(Value::as_i64),
            skin_data_base64_len: skin_data.len(),
            skin_data_decoded_len: STANDARD.decode(skin_data).ok().map(|bytes| bytes.len()),
            skin_geometry_engine_version,
            skin_resource_patch_decoded,
        })
    }

    pub fn bedrock_chain_fingerprint(
        chain: &BedrockJwtChain,
        bedrock_login_token: &str,
    ) -> EngineResult<Value> {
        let chain_items = chain
            .chain
            .iter()
            .enumerate()
            .map(|(index, jwt)| jwt_fingerprint(index, jwt))
            .collect::<EngineResult<Vec<_>>>()?;
        let token = jwt_fingerprint(usize::MAX, bedrock_login_token).unwrap_or_else(|_| {
            json!({
                "jwt": "bedrock_login_token",
                "decode": "failed",
                "len": bedrock_login_token.len()
            })
        });

        Ok(json!({
            "chain_len": chain.chain.len(),
            "display_name": chain.display_name,
            "xuid_present": !chain.xuid.is_empty(),
            "public_key_der_base64_len": chain.public_key_der_base64.len(),
            "chain": chain_items,
            "bedrock_login_token": token
        }))
    }
}

fn client_data_claims(
    display_name: &str,
    server_address: &str,
    playfab_id: Option<&str>,
    _title_id: Option<&str>,
) -> EngineResult<Value> {
    let (device_os, device_model) = (7, "PrismarineJS".to_string());

    let mut payload = if use_classic_skin_profile() {
        classic_skin_payload()?
    } else {
        bedrock_protocol_skin_payload()?
    };

    payload.insert(
        "ClientRandomId".to_string(),
        json!(Utc::now().timestamp_millis()),
    );
    payload.insert("CompatibleWithClientSideChunkGen".to_string(), json!(false));
    payload.insert("CurrentInputMode".to_string(), json!(1));
    payload.insert("DefaultInputMode".to_string(), json!(1));
    payload.insert("DeviceId".to_string(), json!(Uuid::new_v4().to_string()));
    payload.insert("DeviceModel".to_string(), json!(device_model));
    payload.insert("DeviceOS".to_string(), json!(device_os));
    payload.insert("GameVersion".to_string(), json!(login_game_version()));
    payload.insert("GraphicsMode".to_string(), json!(1));
    payload.insert("GuiScale".to_string(), json!(-1));
    payload.insert("IsEditorMode".to_string(), json!(false));
    payload.insert("LanguageCode".to_string(), json!("en_GB"));
    payload.insert("MaxViewDistance".to_string(), json!(0));
    payload.insert("MemoryTier".to_string(), json!(0));
    payload.insert("OverrideSkin".to_string(), json!(false));
    payload.insert("PlatformOfflineId".to_string(), json!(""));
    payload.insert("PlatformOnlineId".to_string(), json!(""));
    payload.insert("PlatformType".to_string(), json!(0));
    let playfab_id_value = client_data_playfab_id(playfab_id);
    payload.insert("PlayFabId".to_string(), json!(playfab_id_value));
    payload.insert("PremiumSkin".to_string(), json!(false));
    payload.insert(
        "SelfSignedId".to_string(),
        json!(Uuid::new_v4().to_string()),
    );
    payload.insert("ServerAddress".to_string(), json!(server_address));
    payload.insert("ThirdPartyName".to_string(), json!(display_name));
    payload
        .entry("TrustedSkin".to_string())
        .or_insert_with(|| json!(false));
    payload.insert("UIProfile".to_string(), json!(0));

    if use_client_data_times() {
        let now = Utc::now().timestamp();
        payload.insert("iat".to_string(), json!(now));
        payload.insert("exp".to_string(), json!(now + 86400));
        payload.insert("nbf".to_string(), json!(now - 60));
    }

    Ok(Value::Object(payload))
}

fn bedrock_protocol_skin_payload() -> EngineResult<serde_json::Map<String, Value>> {
    let value: Value =
        serde_json::from_str(include_str!("../../../assets/bedrock_1_21_70_steve.json"))?;
    value.as_object().cloned().ok_or_else(|| {
        EngineError::Bedrock("bedrock-protocol skin asset must be a JSON object".to_string())
    })
}

fn classic_skin_payload() -> EngineResult<serde_json::Map<String, Value>> {
    let mut payload = serde_json::Map::<String, Value>::new();
    payload.insert("AnimatedImageData".to_string(), json!([]));
    payload.insert("ArmSize".to_string(), json!("wide"));
    payload.insert("CapeData".to_string(), json!(""));
    payload.insert("CapeId".to_string(), json!(""));
    payload.insert("CapeImageHeight".to_string(), json!(0));
    payload.insert("CapeImageWidth".to_string(), json!(0));
    payload.insert("CapeOnClassicSkin".to_string(), json!(false));
    payload.insert("PersonaPieces".to_string(), json!([]));
    payload.insert("PersonaSkin".to_string(), json!(false));
    payload.insert("PieceTintColors".to_string(), json!([]));
    payload.insert("PremiumSkin".to_string(), json!(false));
    payload.insert("SkinAnimationData".to_string(), json!(""));
    payload.insert("SkinColor".to_string(), json!("#0"));
    payload.insert("SkinData".to_string(), json!(classic_skin_data_base64()));
    payload.insert(
        "SkinGeometryData".to_string(),
        json!(base64_json(&classic_skin_geometry())?),
    );
    payload.insert(
        "SkinGeometryDataEngineVersion".to_string(),
        json!(STANDARD.encode("1.14.0")),
    );
    payload.insert(
        "SkinId".to_string(),
        json!(format!("Custom{}", Uuid::new_v4())),
    );
    payload.insert(
        "SkinImageHeight".to_string(),
        json!(CLASSIC_SKIN_HEIGHT as i64),
    );
    payload.insert(
        "SkinImageWidth".to_string(),
        json!(CLASSIC_SKIN_WIDTH as i64),
    );
    payload.insert(
        "SkinResourcePatch".to_string(),
        json!(base64_json(&json!({
            "geometry": {
                "default": "geometry.humanoid.custom"
            }
        }))?),
    );
    Ok(payload)
}

fn use_wide_client_identity_times() -> bool {
    std::env::var("BEDROCK_CLIENT_IDENTITY_TIMES")
        .map(|value| value.eq_ignore_ascii_case("wide"))
        .unwrap_or(false)
}

fn use_client_data_times() -> bool {
    std::env::var("BEDROCK_CLIENT_DATA_TIMES")
        .map(|value| value.eq_ignore_ascii_case("wide"))
        .unwrap_or(false)
}

fn use_classic_skin_profile() -> bool {
    std::env::var("BEDROCK_SKIN_PROFILE")
        .map(|value| value.eq_ignore_ascii_case("classic"))
        .unwrap_or(false)
}

fn client_data_playfab_id(playfab_id: Option<&str>) -> String {
    let use_provisioned = std::env::var("BEDROCK_CLIENTDATA_PLAYFAB_ID")
        .map(|value| value.eq_ignore_ascii_case("provisioned"))
        .unwrap_or(false);
    if use_provisioned {
        if let Some(playfab_id) = playfab_id.filter(|s| !s.is_empty()) {
            return playfab_id.to_lowercase();
        }
    }

    Uuid::new_v4()
        .to_string()
        .replace('-', "")
        .chars()
        .take(16)
        .collect::<String>()
        .to_lowercase()
}

fn classic_skin_data_base64() -> String {
    let mut rgba = Vec::with_capacity(CLASSIC_SKIN_WIDTH * CLASSIC_SKIN_HEIGHT * 4);
    for y in 0..CLASSIC_SKIN_HEIGHT {
        for x in 0..CLASSIC_SKIN_WIDTH {
            let checker = ((x / 8) + (y / 8)) % 2 == 0;
            let (r, g, b) = if checker {
                (0x48, 0x7a, 0xb7)
            } else {
                (0x2d, 0x2f, 0x36)
            };
            rgba.extend_from_slice(&[r, g, b, 0xff]);
        }
    }
    STANDARD.encode(rgba)
}

fn login_game_version() -> String {
    if let Ok(version) = std::env::var("BEDROCK_GAME_VERSION") {
        let version = version.trim();
        if !version.is_empty() {
            return version.to_string();
        }
    }

    PRISMARINE_DEFAULT_GAME_VERSION.to_string()
}

fn base64_json(value: &Value) -> EngineResult<String> {
    Ok(STANDARD.encode(serde_json::to_vec(value)?))
}

fn classic_skin_geometry() -> Value {
    json!({
        "format_version": "1.12.0",
        "minecraft:geometry": [
            {
                "description": {
                    "identifier": "geometry.humanoid.custom",
                    "texture_width": 64,
                    "texture_height": 64,
                    "visible_bounds_width": 1,
                    "visible_bounds_height": 2,
                    "visible_bounds_offset": [0, 1, 0]
                },
                "bones": [
                    { "name": "root", "pivot": [0, 0, 0] },
                    { "name": "waist", "parent": "root", "pivot": [0, 12, 0] },
                    {
                        "name": "body",
                        "parent": "waist",
                        "pivot": [0, 24, 0],
                        "cubes": [{ "origin": [-4, 12, -2], "size": [8, 12, 4], "uv": [16, 16] }]
                    },
                    {
                        "name": "head",
                        "parent": "body",
                        "pivot": [0, 24, 0],
                        "cubes": [{ "origin": [-4, 24, -4], "size": [8, 8, 8], "uv": [0, 0] }]
                    },
                    {
                        "name": "hat",
                        "parent": "head",
                        "pivot": [0, 24, 0],
                        "cubes": [{ "origin": [-4, 24, -4], "size": [8, 8, 8], "uv": [32, 0], "inflate": 0.5 }]
                    },
                    {
                        "name": "rightArm",
                        "parent": "body",
                        "pivot": [-5, 22, 0],
                        "cubes": [{ "origin": [-8, 12, -2], "size": [4, 12, 4], "uv": [40, 16] }]
                    },
                    {
                        "name": "leftArm",
                        "parent": "body",
                        "pivot": [5, 22, 0],
                        "cubes": [{ "origin": [4, 12, -2], "size": [4, 12, 4], "uv": [32, 48] }]
                    },
                    {
                        "name": "rightLeg",
                        "parent": "root",
                        "pivot": [-1.9, 12, 0],
                        "cubes": [{ "origin": [-3.9, 0, -2], "size": [4, 12, 4], "uv": [0, 16] }]
                    },
                    {
                        "name": "leftLeg",
                        "parent": "root",
                        "pivot": [1.9, 12, 0],
                        "cubes": [{ "origin": [-0.1, 0, -2], "size": [4, 12, 4], "uv": [16, 48] }]
                    }
                ]
            }
        ]
    })
}

#[derive(Debug, Default)]
struct LegacyProfile {
    display_name: Option<String>,
    xuid: Option<String>,
    title_id: Option<String>,
}

fn legacy_profile_from_chain(chain: &[String]) -> LegacyProfile {
    let mut profile = LegacyProfile::default();
    for jwt in chain {
        let Some(payload_b64) = jwt.split('.').nth(1) else {
            continue;
        };
        let Ok(payload_bytes) = URL_SAFE_NO_PAD.decode(payload_b64) else {
            continue;
        };
        let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&payload_bytes) else {
            continue;
        };
        let Some(extra_data) = payload.get("extraData") else {
            continue;
        };
        if profile.display_name.is_none() {
            profile.display_name = extra_data
                .get("displayName")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
        }
        if profile.xuid.is_none() {
            profile.xuid = extra_data
                .get("XUID")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
        }
        if profile.title_id.is_none() {
            profile.title_id = extra_data.get("titleId").map(|value| match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            });
        }
    }
    profile
}

fn read_le_string(data: &[u8], cursor: &mut usize, label: &str) -> EngineResult<String> {
    let Some(length_bytes) = data.get(*cursor..*cursor + 4) else {
        return Err(EngineError::Bedrock(format!(
            "connection request missing {label} length"
        )));
    };
    *cursor += 4;
    let length =
        u32::from_le_bytes(length_bytes.try_into().expect("slice has four bytes")) as usize;
    let Some(bytes) = data.get(*cursor..*cursor + length) else {
        return Err(EngineError::Bedrock(format!(
            "connection request {label} length {length} exceeds buffer"
        )));
    };
    *cursor += length;
    String::from_utf8(bytes.to_vec()).map_err(|err| {
        EngineError::Bedrock(format!("connection request {label} is not UTF-8: {err}"))
    })
}

fn decode_jwt_payload(jwt: &str) -> EngineResult<Value> {
    let payload_b64 = jwt
        .split('.')
        .nth(1)
        .ok_or_else(|| EngineError::Bedrock("JWT missing payload segment".to_string()))?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|err| EngineError::Bedrock(format!("decode JWT payload: {err}")))?;
    serde_json::from_slice(&payload)
        .map_err(|err| EngineError::Bedrock(format!("parse JWT payload: {err}")))
}

fn jwt_fingerprint(index: usize, jwt: &str) -> EngineResult<Value> {
    let mut parts = jwt.split('.');
    let header_b64 = parts
        .next()
        .ok_or_else(|| EngineError::Bedrock("JWT missing header segment".to_string()))?;
    let payload_b64 = parts
        .next()
        .ok_or_else(|| EngineError::Bedrock("JWT missing payload segment".to_string()))?;

    let header_bytes = URL_SAFE_NO_PAD
        .decode(header_b64)
        .or_else(|_| STANDARD.decode(header_b64))
        .map_err(|err| EngineError::Bedrock(format!("decode JWT header: {err}")))?;
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| STANDARD.decode(payload_b64))
        .map_err(|err| EngineError::Bedrock(format!("decode JWT payload: {err}")))?;
    let header: Value = serde_json::from_slice(&header_bytes)
        .map_err(|err| EngineError::Bedrock(format!("parse JWT header: {err}")))?;
    let payload: Value = serde_json::from_slice(&payload_bytes)
        .map_err(|err| EngineError::Bedrock(format!("parse JWT payload: {err}")))?;
    let extra_data = payload
        .get("extraData")
        .cloned()
        .unwrap_or_else(|| json!({}));

    Ok(json!({
        "index": if index == usize::MAX { json!("bedrock_login_token") } else { json!(index) },
        "jwt_len": jwt.len(),
        "header_alg": header.get("alg").and_then(Value::as_str),
        "header_x5u_len": header.get("x5u").and_then(Value::as_str).map(str::len),
        "payload_keys": sorted_object_keys(&payload),
        "certificate_authority": payload.get("certificateAuthority").and_then(Value::as_bool),
        "identity_public_key_len": payload.get("identityPublicKey").and_then(Value::as_str).map(str::len),
        "identity_public_key_matches_prismarine": payload.get("identityPublicKey").and_then(Value::as_str).is_some_and(|value| value == PRISMARINE_MOJANG_PUBLIC_KEY),
        "extra_data_keys": sorted_object_keys(&extra_data),
        "extra_display_name": extra_data.get("displayName").and_then(Value::as_str),
        "extra_title_id": extra_data.get("titleId").map(|value| match value {
            Value::String(value) => value.clone(),
            other => other.to_string(),
        }),
        "extra_xuid_present": extra_data.get("XUID").and_then(Value::as_str).is_some_and(|value| !value.is_empty()),
        "has_iat": payload.get("iat").is_some(),
        "has_exp": payload.get("exp").is_some(),
        "has_nbf": payload.get("nbf").is_some(),
        "cpk_len": payload.get("cpk").and_then(Value::as_str).map(str::len),
        "pfcd_len": payload.get("pfcd").and_then(Value::as_str).map(str::len),
        "xid_present": payload.get("xid").and_then(Value::as_str).is_some_and(|value| !value.is_empty()),
        "xname": payload.get("xname").and_then(Value::as_str),
    }))
}

fn sorted_object_keys(value: &Value) -> Vec<String> {
    let mut keys = value
        .as_object()
        .map(|object| object.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort();
    keys
}

fn string_claim(payload: &Value, key: &str) -> Option<String> {
    payload.get(key).and_then(Value::as_str).map(str::to_string)
}
