use std::net::SocketAddr;

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{
        header::{HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE},
        HeaderMap, Method, Request, StatusCode,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use subtle::ConstantTimeEq;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{
    auth::{entitlement::EntitlementProvisioner, microsoft::MicrosoftAuth},
    bot::supervisor::BotSupervisor,
    config::Config,
    db::Database,
    error::EngineResult,
    models::{Account, Bot, CapabilityStatus, DeviceAuthSession, Entitlement, LogEntry, Server},
};

#[derive(Clone)]
pub struct AppState {
    config: Config,
    db: Database,
    bots: BotSupervisor,
}

pub async fn serve(config: Config, db: Database, bind: SocketAddr) -> anyhow::Result<()> {
    let app = build_router(config, db, bind)?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "torchflower engine API listening");
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn build_router(config: Config, db: Database, bind: SocketAddr) -> EngineResult<Router> {
    config.validate_api_security(bind)?;
    let state = AppState {
        bots: BotSupervisor::new(config.clone(), db.clone()),
        config,
        db,
    };
    let cors = cors_layer(&state.config)?;
    let auth_config = state.config.clone();
    let api = Router::new()
        .route("/accounts", get(list_accounts).post(import_account))
        .route("/entitlements", get(list_entitlements))
        .route("/accounts/{account_id}/provision", post(provision_account))
        .route("/auth/sessions/{session_id}/poll", post(poll_auth_session))
        .route("/servers", get(list_servers).post(create_server))
        .route("/bots", get(list_bots).post(create_bot))
        .route("/bots/{bot_id}/start", post(start_bot))
        .route("/bots/{bot_id}/stop", post(stop_bot))
        .route("/logs", get(list_logs))
        .route("/validate-real-server", post(validate_real_server))
        .route_layer(middleware::from_fn_with_state(
            auth_config,
            require_api_auth,
        ));
    let app = Router::new()
        .route("/health", get(health))
        .nest("/api", api)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    Ok(app)
}

fn cors_layer(config: &Config) -> EngineResult<CorsLayer> {
    let origins = config
        .cors_allowed_origins
        .iter()
        .map(|origin| {
            origin.parse::<HeaderValue>().map_err(|_| {
                crate::error::EngineError::InvalidRequest(format!(
                    "invalid CORS origin configured: {origin}"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            AUTHORIZATION,
            CONTENT_TYPE,
            HeaderName::from_static("x-torchflower-api-key"),
        ]))
}

async fn require_api_auth(
    State(config): State<Config>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if config.dev_allow_unauth_api && config.api_key.is_none() {
        return next.run(request).await;
    }
    let authorized = config
        .api_key
        .as_deref()
        .zip(supplied_api_key(request.headers()))
        .is_some_and(|(expected, supplied)| expected.as_bytes().ct_eq(supplied.as_bytes()).into());
    if authorized {
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": {
                "code": "unauthorized",
                "message": "valid TorchFlower API credentials are required",
                "request_id": null
            }
        })),
    )
        .into_response()
}

fn supplied_api_key(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            headers
                .get("x-torchflower-api-key")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "service": "bedrock-engine" }))
}

async fn list_accounts(State(state): State<AppState>) -> EngineResult<Json<Vec<Account>>> {
    Ok(Json(state.db.list_accounts().await?))
}

async fn list_entitlements(State(state): State<AppState>) -> EngineResult<Json<Vec<Entitlement>>> {
    Ok(Json(state.db.list_entitlements().await?))
}

#[derive(Debug, Deserialize)]
struct ImportAccountRequest {
    email: String,
}

#[derive(Debug, Serialize)]
struct ImportAccountResponse {
    session: DeviceAuthSession,
}

async fn import_account(
    State(state): State<AppState>,
    Json(input): Json<ImportAccountRequest>,
) -> EngineResult<Json<ImportAccountResponse>> {
    let session = MicrosoftAuth::new(&state.config, state.db.clone())
        .start_device_auth(&input.email)
        .await?;
    Ok(Json(ImportAccountResponse { session }))
}

#[derive(Debug, Serialize)]
struct PollResponse {
    status: String,
    account: Option<Account>,
}

async fn poll_auth_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> EngineResult<Json<PollResponse>> {
    let microsoft = MicrosoftAuth::new(&state.config, state.db.clone());
    let (session, _) = state.db.get_auth_session_secret(&session_id).await?;
    if let Some(tokens) = microsoft.poll_device_auth(&session_id).await? {
        let provisioner = EntitlementProvisioner::new(&state.config, state.db.clone());
        provisioner
            .save_microsoft_tokens(&session.account_id, &tokens)
            .await?;
        let _ = provisioner.provision(&session.account_id).await?;
        let account = state.db.get_account(&session.account_id).await?;
        Ok(Json(PollResponse {
            status: "authenticated".to_string(),
            account: Some(account),
        }))
    } else {
        Ok(Json(PollResponse {
            status: "pending".to_string(),
            account: None,
        }))
    }
}

async fn provision_account(
    State(state): State<AppState>,
    Path(account_id): Path<String>,
) -> EngineResult<Json<Account>> {
    EntitlementProvisioner::new(&state.config, state.db.clone())
        .provision(&account_id)
        .await?;
    Ok(Json(state.db.get_account(&account_id).await?))
}

async fn list_servers(State(state): State<AppState>) -> EngineResult<Json<Vec<Server>>> {
    Ok(Json(state.db.list_servers().await?))
}

#[derive(Debug, Deserialize)]
struct CreateServerRequest {
    name: String,
    host: String,
    port: Option<i64>,
}

async fn create_server(
    State(state): State<AppState>,
    Json(input): Json<CreateServerRequest>,
) -> EngineResult<Json<Server>> {
    let id = state
        .db
        .create_server(&input.name, &input.host, input.port.unwrap_or(19132))
        .await?;
    let servers = state.db.list_servers().await?;
    Ok(Json(
        servers
            .into_iter()
            .find(|server| server.id == id)
            .expect("server exists after insert"),
    ))
}

async fn list_bots(State(state): State<AppState>) -> EngineResult<Json<Vec<Bot>>> {
    Ok(Json(state.db.list_bots().await?))
}

#[derive(Debug, Deserialize)]
struct CreateBotRequest {
    account_id: String,
    server_id: String,
}

async fn create_bot(
    State(state): State<AppState>,
    Json(input): Json<CreateBotRequest>,
) -> EngineResult<Json<Bot>> {
    let id = state
        .db
        .create_bot(&input.account_id, &input.server_id)
        .await?;
    let bots = state.db.list_bots().await?;
    Ok(Json(
        bots.into_iter()
            .find(|bot| bot.id == id)
            .expect("bot exists after insert"),
    ))
}

async fn start_bot(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
) -> EngineResult<Json<serde_json::Value>> {
    state.bots.start(&bot_id).await?;
    Ok(Json(json!({ "status": "starting" })))
}

async fn stop_bot(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
) -> EngineResult<Json<serde_json::Value>> {
    state.bots.stop(&bot_id).await?;
    Ok(Json(json!({ "status": "stopped" })))
}

#[derive(Debug, Deserialize)]
struct LogsQuery {
    limit: Option<i64>,
}

async fn list_logs(
    State(state): State<AppState>,
    Query(query): Query<LogsQuery>,
) -> EngineResult<Json<Vec<LogEntry>>> {
    Ok(Json(
        state
            .db
            .list_logs(query.limit.unwrap_or(250).clamp(1, 1000))
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
struct ValidateRealServerRequest {
    account_id: String,
    server_id: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    duration_seconds: Option<u64>,
}

async fn validate_real_server(
    State(state): State<AppState>,
    Json(input): Json<ValidateRealServerRequest>,
) -> EngineResult<Json<CapabilityStatus>> {
    let (host, port) = if let Some(server_id) = input.server_id.as_deref() {
        let server = state.db.get_server(server_id).await?;
        (server.host, server.port as u16)
    } else {
        let host = input.host.ok_or_else(|| {
            crate::error::EngineError::InvalidRequest(
                "validate-real-server requires server_id or host".to_string(),
            )
        })?;
        if !state.config.is_server_host_allowed(&host) {
            return Err(crate::error::EngineError::InvalidRequest(format!(
                "host {host} is not in TORCHFLOWER_ALLOWED_SERVER_HOSTS"
            )));
        }
        let port = input.port.unwrap_or(19132);
        (host, port)
    };
    Ok(Json(
        state
            .bots
            .validate_once_for(
                &input.account_id,
                &host,
                port,
                std::time::Duration::from_secs(input.duration_seconds.unwrap_or(300).clamp(5, 900)),
            )
            .await?,
    ))
}
