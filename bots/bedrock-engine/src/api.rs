use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::Method,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use crate::{
    auth::{entitlement::EntitlementProvisioner, microsoft::MicrosoftAuth},
    bot::supervisor::BotSupervisor,
    config::Config,
    db::Database,
    error::EngineResult,
    models::{Account, Bot, CapabilityStatus, DeviceAuthSession, LogEntry, Server},
};

#[derive(Clone)]
pub struct AppState {
    config: Config,
    db: Database,
    bots: BotSupervisor,
}

pub async fn serve(config: Config, db: Database, bind: SocketAddr) -> anyhow::Result<()> {
    let state = AppState {
        bots: BotSupervisor::new(config.clone(), db.clone()),
        config,
        db,
    };
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers(Any);
    let app = Router::new()
        .route("/health", get(health))
        .route("/api/accounts", get(list_accounts).post(import_account))
        .route(
            "/api/accounts/{account_id}/provision",
            post(provision_account),
        )
        .route(
            "/api/auth/sessions/{session_id}/poll",
            post(poll_auth_session),
        )
        .route("/api/servers", get(list_servers).post(create_server))
        .route("/api/bots", get(list_bots).post(create_bot))
        .route("/api/bots/{bot_id}/start", post(start_bot))
        .route("/api/bots/{bot_id}/stop", post(stop_bot))
        .route("/api/logs", get(list_logs))
        .route("/api/validate-real-server", post(validate_real_server))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "bedrock engine API listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "service": "bedrock-engine" }))
}

async fn list_accounts(State(state): State<AppState>) -> EngineResult<Json<Vec<Account>>> {
    Ok(Json(state.db.list_accounts().await?))
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
    host: String,
    port: Option<u16>,
}

async fn validate_real_server(
    State(state): State<AppState>,
    Json(input): Json<ValidateRealServerRequest>,
) -> EngineResult<Json<CapabilityStatus>> {
    Ok(Json(
        state
            .bots
            .validate_once(&input.account_id, &input.host, input.port.unwrap_or(19132))
            .await?,
    ))
}
