use std::net::SocketAddr;

use axum::{
    body::{to_bytes, Body},
    http::{
        header::{
            ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_METHOD, AUTHORIZATION, ORIGIN,
        },
        Request, StatusCode,
    },
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use torchflower_engine::{
    api::build_router,
    config::{parse_token_encryption_key, Config, MicrosoftAuthFlow},
    db::Database,
    diagnostics::Diagnostics,
    error::EngineError,
};
use tower::ServiceExt;

fn test_config(api_key: Option<&str>) -> Config {
    Config {
        microsoft_client_id: "client-id".to_string(),
        microsoft_auth_flow: MicrosoftAuthFlow::Live,
        token_encryption_key: [7u8; 32],
        database_url: "sqlite::memory:".to_string(),
        rust_engine_bind: "127.0.0.1:0".to_string(),
        api_key: api_key.map(ToOwned::to_owned),
        dev_allow_unauth_api: false,
        cors_allowed_origins: vec!["http://localhost:3000".to_string()],
        allowed_server_hosts: vec!["allowed.example".to_string()],
        dangerous_log_auth_bodies: false,
    }
}

async fn test_db() -> Database {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    db
}

#[tokio::test]
async fn health_is_public_but_api_requires_auth() {
    let app = build_router(
        test_config(Some("test-api-key")),
        test_db().await,
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    )
    .unwrap();

    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);

    let accounts = app
        .oneshot(
            Request::builder()
                .uri("/api/accounts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accounts.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_accepts_bearer_or_api_key_header() {
    let app = build_router(
        test_config(Some("test-api-key")),
        test_db().await,
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    )
    .unwrap();

    let bearer = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/accounts")
                .header(AUTHORIZATION, "Bearer test-api-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bearer.status(), StatusCode::OK);

    let custom = app
        .oneshot(
            Request::builder()
                .uri("/api/accounts")
                .header("x-torchflower-api-key", "test-api-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(custom.status(), StatusCode::OK);
}

#[tokio::test]
async fn direct_validation_host_must_be_allowed() {
    let app = build_router(
        test_config(Some("test-api-key")),
        test_db().await,
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    )
    .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/validate-real-server")
                .header(AUTHORIZATION, "Bearer test-api-key")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"account_id":"account","host":"blocked.example","port":19132}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("TORCHFLOWER_ALLOWED_SERVER_HOSTS"));
}

#[tokio::test]
async fn cors_only_allows_configured_origins() {
    let app = build_router(
        test_config(Some("test-api-key")),
        test_db().await,
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    )
    .unwrap();

    let allowed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/accounts")
                .header(ORIGIN, "http://localhost:3000")
                .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        allowed.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
        "http://localhost:3000"
    );

    let rejected = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/accounts")
                .header(ORIGIN, "http://evil.example")
                .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(rejected
        .headers()
        .get(ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_none());
}

#[test]
fn unsafe_unauthenticated_api_is_rejected_on_non_loopback_bind() {
    let mut config = test_config(None);
    config.dev_allow_unauth_api = true;
    assert!(config
        .validate_api_security("0.0.0.0:9080".parse::<SocketAddr>().unwrap())
        .is_err());
}

#[test]
fn token_key_prefers_base64_and_rejects_weak_legacy_secret() {
    let key = [9u8; 32];
    let parsed = parse_token_encryption_key(Some(&STANDARD.encode(key)), None).unwrap();
    assert_eq!(parsed, key);

    let weak = parse_token_encryption_key(None, Some("replace-with-32-plus-random-characters"));
    assert!(weak.is_err());

    let strong_legacy = parse_token_encryption_key(
        None,
        Some("this-is-a-local-legacy-secret-with-enough-entropy-2026"),
    )
    .unwrap();
    assert_eq!(strong_legacy.len(), 32);
}

#[tokio::test]
async fn api_errors_do_not_expose_auth_provider_bodies() {
    let response = EngineError::Auth {
        step: "microsoft_refresh",
        message: r#"{"access_token":"secret-token","error":"bad"}"#.to_string(),
    }
    .into_response();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("authentication failed during microsoft_refresh"));
    assert!(!text.contains("secret-token"));
    assert!(!text.contains("access_token"));
}

#[tokio::test]
async fn form_json_rejects_non_2xx_without_storing_bodies() {
    let url = spawn_auth_endpoint(StatusCode::BAD_REQUEST).await;
    let db = test_db().await;
    let diagnostics = Diagnostics::new_with_body_capture(db.clone(), false);
    let err = diagnostics
        .request_form_json::<Value>(
            &reqwest::Client::new(),
            None,
            "test_form",
            &url,
            vec![("refresh_token", "refresh-secret".to_string())],
        )
        .await
        .unwrap_err();
    assert!(!err.to_string().contains("refresh-secret"));
    assert!(!err.to_string().contains("access-secret"));

    let logs = db.list_logs(10).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert!(logs[0].request_body.is_none());
    assert!(logs[0].response_body.is_none());
}

#[tokio::test]
async fn dangerous_body_capture_still_redacts_sensitive_fields() {
    let url = spawn_auth_endpoint(StatusCode::OK).await;
    let db = test_db().await;
    let diagnostics = Diagnostics::new_with_body_capture(db.clone(), true);
    let (_parsed, _, _) = diagnostics
        .request_form_json::<Value>(
            &reqwest::Client::new(),
            None,
            "test_form",
            &url,
            vec![
                ("refresh_token", "refresh-secret".to_string()),
                ("client_id", "client".to_string()),
            ],
        )
        .await
        .unwrap();

    let logs = db.list_logs(10).await.unwrap();
    let joined = format!(
        "{} {}",
        logs[0].request_body.as_deref().unwrap_or_default(),
        logs[0].response_body.as_deref().unwrap_or_default()
    );
    for secret in [
        "refresh-secret",
        "access-secret",
        "session-secret",
        "cookie-secret",
        "signed-secret",
    ] {
        assert!(!joined.contains(secret), "{secret} leaked into logs");
    }
    assert!(joined.contains("<redacted>"));
}

async fn spawn_auth_endpoint(status: StatusCode) -> String {
    let app = Router::new().route(
        "/token",
        post(move || async move {
            (
                status,
                Json(json!({
                    "access_token": "access-secret",
                    "SessionTicket": "session-secret",
                    "cookie": "cookie-secret",
                    "signedToken": "signed-secret"
                })),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/token")
}
