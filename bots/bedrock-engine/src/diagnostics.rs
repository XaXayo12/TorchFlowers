use reqwest::{Method, StatusCode, header::HeaderMap};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{
    db::{Database, NewLogEntry},
    error::{EngineError, EngineResult},
};

#[derive(Clone)]
pub struct Diagnostics {
    db: Database,
    capture_auth_bodies: bool,
}

impl Diagnostics {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            capture_auth_bodies: dangerous_body_capture_enabled(),
        }
    }

    pub fn new_with_body_capture(db: Database, capture_auth_bodies: bool) -> Self {
        Self {
            db,
            capture_auth_bodies,
        }
    }

    pub async fn log_event(
        &self,
        account_id: Option<&str>,
        bot_id: Option<&str>,
        level: &str,
        category: &str,
        step: Option<&str>,
        message: &str,
        metadata: Value,
    ) -> EngineResult<()> {
        let metadata_json = metadata.to_string();
        self.db
            .log(NewLogEntry {
                account_id,
                bot_id,
                level,
                category,
                step,
                request_id: None,
                method: None,
                url: None,
                status_code: None,
                request_body: None,
                response_body: None,
                message,
                metadata_json: Some(&metadata_json),
            })
            .await
    }

    pub async fn request_json<T: DeserializeOwned>(
        &self,
        client: &reqwest::Client,
        account_id: Option<&str>,
        step: &'static str,
        method: Method,
        url: &str,
        headers: Vec<(&str, String)>,
        body: Value,
    ) -> EngineResult<(T, Option<String>, StatusCode)> {
        let mut request = client.request(method.clone(), url);
        for (name, value) in &headers {
            request = request.header(*name, value);
        }
        let response = request.json(&body).send().await?;
        let status = response.status();
        let request_id = request_id(response.headers());
        let response_body = response.text().await?;
        let request_body = self.capture_json_body(body);
        let response_body_for_log = self.capture_raw_body(&response_body);
        let metadata = http_log_metadata(self.capture_auth_bodies);
        self.db
            .log(NewLogEntry {
                account_id,
                bot_id: None,
                level: if status.is_success() { "info" } else { "error" },
                category: "auth_http",
                step: Some(step),
                request_id: request_id.as_deref(),
                method: Some(method.as_str()),
                url: Some(url),
                status_code: Some(status.as_u16() as i64),
                request_body: request_body.as_deref(),
                response_body: response_body_for_log.as_deref(),
                message: if status.is_success() {
                    "authentication HTTP step succeeded"
                } else {
                    "authentication HTTP step failed"
                },
                metadata_json: Some(&metadata),
            })
            .await?;
        if !status.is_success() {
            return Err(EngineError::Auth {
                step,
                message: auth_error_message(status, request_id.as_deref()),
            });
        }
        let parsed = serde_json::from_str(&response_body).map_err(|err| EngineError::Auth {
            step,
            message: format!(
                "response was not valid JSON: {err}; status={}",
                status.as_u16()
            ),
        })?;
        Ok((parsed, request_id, status))
    }

    pub async fn request_json_text<T: DeserializeOwned>(
        &self,
        client: &reqwest::Client,
        account_id: Option<&str>,
        step: &'static str,
        method: Method,
        url: &str,
        headers: Vec<(&str, String)>,
        body_text: String,
    ) -> EngineResult<(T, Option<String>, StatusCode)> {
        let mut request = client.request(method.clone(), url);
        for (name, value) in &headers {
            request = request.header(*name, value);
        }
        let request_body = self.capture_raw_body(&body_text);
        let response = request.body(body_text).send().await?;
        let status = response.status();
        let request_id = request_id(response.headers());
        let response_body = response.text().await?;
        let response_body_for_log = self.capture_raw_body(&response_body);
        let metadata = http_log_metadata(self.capture_auth_bodies);
        self.db
            .log(NewLogEntry {
                account_id,
                bot_id: None,
                level: if status.is_success() { "info" } else { "error" },
                category: "auth_http",
                step: Some(step),
                request_id: request_id.as_deref(),
                method: Some(method.as_str()),
                url: Some(url),
                status_code: Some(status.as_u16() as i64),
                request_body: request_body.as_deref(),
                response_body: response_body_for_log.as_deref(),
                message: if status.is_success() {
                    "authentication HTTP step succeeded"
                } else {
                    "authentication HTTP step failed"
                },
                metadata_json: Some(&metadata),
            })
            .await?;
        if !status.is_success() {
            return Err(EngineError::Auth {
                step,
                message: auth_error_message(status, request_id.as_deref()),
            });
        }
        let parsed = serde_json::from_str(&response_body).map_err(|err| EngineError::Auth {
            step,
            message: format!(
                "response was not valid JSON: {err}; status={}",
                status.as_u16()
            ),
        })?;
        Ok((parsed, request_id, status))
    }

    pub async fn request_form_json<T: DeserializeOwned>(
        &self,
        client: &reqwest::Client,
        account_id: Option<&str>,
        step: &'static str,
        url: &str,
        form: Vec<(&str, String)>,
    ) -> EngineResult<(T, Option<String>, StatusCode)> {
        let body_json = Value::Object(
            form.iter()
                .map(|(key, value)| ((*key).to_string(), Value::String(value.clone())))
                .collect(),
        );
        let request_body = self.capture_json_body(body_json);
        let response = client
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(encode_form(&form))
            .send()
            .await?;
        let status = response.status();
        let request_id = request_id(response.headers());
        let response_body = response.text().await?;
        let response_body_for_log = self.capture_raw_body(&response_body);
        let metadata = http_log_metadata(self.capture_auth_bodies);
        self.db
            .log(NewLogEntry {
                account_id,
                bot_id: None,
                level: if status.is_success() { "info" } else { "error" },
                category: "auth_http",
                step: Some(step),
                request_id: request_id.as_deref(),
                method: Some("POST"),
                url: Some(url),
                status_code: Some(status.as_u16() as i64),
                request_body: request_body.as_deref(),
                response_body: response_body_for_log.as_deref(),
                message: if status.is_success() {
                    "authentication HTTP step succeeded"
                } else {
                    "authentication HTTP step failed"
                },
                metadata_json: Some(&metadata),
            })
            .await?;
        if !status.is_success() {
            return Err(EngineError::Auth {
                step,
                message: auth_error_message(status, request_id.as_deref()),
            });
        }
        let parsed = serde_json::from_str(&response_body).map_err(|err| EngineError::Auth {
            step,
            message: format!(
                "response was not valid JSON: {err}; status={}",
                status.as_u16()
            ),
        })?;
        Ok((parsed, request_id, status))
    }

    fn capture_json_body(&self, body: Value) -> Option<String> {
        self.capture_auth_bodies
            .then(|| redact_value(body).to_string())
    }

    fn capture_raw_body(&self, raw: &str) -> Option<String> {
        self.capture_auth_bodies.then(|| redact_json_string(raw))
    }
}

fn request_id(headers: &HeaderMap) -> Option<String> {
    ["x-request-id", "request-id", "x-correlation-id", "ms-cv"]
        .iter()
        .find_map(|name| {
            headers
                .get(*name)
                .and_then(|v| v.to_str().ok())
                .map(ToOwned::to_owned)
        })
}

fn redact_json_string(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .map(redact_value)
        .map(|v| v.to_string())
        .unwrap_or_else(|_| "<non-json body redacted>".to_string())
}

fn redact_value(value: Value) -> Value {
    match value {
        Value::Object(mut map) => {
            let keys = map.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                if sensitive_key(&key) {
                    map.insert(key, Value::String("<redacted>".to_string()));
                }
            }
            Value::Object(map.into_iter().map(|(k, v)| (k, redact_value(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(redact_value).collect()),
        other => other,
    }
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "access_token",
        "refresh_token",
        "sessionticket",
        "xboxtoken",
        "rpsticket",
        "authorization",
        "identity",
        "chain",
        "signedtoken",
        "token",
        "secret",
        "password",
        "cookie",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn dangerous_body_capture_enabled() -> bool {
    std::env::var("TORCHFLOWER_DANGEROUS_LOG_AUTH_BODIES")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

fn http_log_metadata(capture_auth_bodies: bool) -> String {
    json!({
        "request_body_capture": if capture_auth_bodies { "dangerous_redacted" } else { "disabled" },
        "response_body_capture": if capture_auth_bodies { "dangerous_redacted" } else { "disabled" }
    })
    .to_string()
}

fn auth_error_message(status: StatusCode, request_id: Option<&str>) -> String {
    match request_id {
        Some(request_id) => format!("status={}; request_id={request_id}", status.as_u16()),
        None => format!("status={}", status.as_u16()),
    }
}

pub fn auth_step_metadata(name: &str) -> Value {
    json!({ "step": name })
}

pub fn encode_form(form: &[(&str, String)]) -> String {
    form.iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                urlencoding::encode(key),
                urlencoding::encode(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}
