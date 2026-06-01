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
}

impl Diagnostics {
    pub fn new(db: Database) -> Self {
        Self { db }
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
        let redacted_body = redact_value(body.clone());
        let redacted_body_text = redacted_body.to_string();
        let response = request.json(&body).send().await?;
        let status = response.status();
        let request_id = request_id(response.headers());
        let response_body = response.text().await?;
        let redacted_response_body = redact_json_string(&response_body);
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
                request_body: Some(&redacted_body_text),
                response_body: Some(&redacted_response_body),
                message: if status.is_success() {
                    "authentication HTTP step succeeded"
                } else {
                    "authentication HTTP step failed"
                },
                metadata_json: Some("{}"),
            })
            .await?;
        if !status.is_success() {
            return Err(EngineError::Auth {
                step,
                message: response_body,
            });
        }
        Ok((serde_json::from_str(&response_body)?, request_id, status))
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
        let redacted_body = redact_value(body_json).to_string();
        let response = client
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(encode_form(&form))
            .send()
            .await?;
        let status = response.status();
        let request_id = request_id(response.headers());
        let response_body = response.text().await?;
        let redacted_response_body = redact_json_string(&response_body);
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
                request_body: Some(&redacted_body),
                response_body: Some(&redacted_response_body),
                message: if status.is_success() {
                    "authentication HTTP step succeeded"
                } else {
                    "authentication HTTP step failed"
                },
                metadata_json: Some("{}"),
            })
            .await?;
        Ok((serde_json::from_str(&response_body)?, request_id, status))
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
        .unwrap_or_else(|_| raw.chars().take(16_384).collect())
}

fn redact_value(value: Value) -> Value {
    match value {
        Value::Object(mut map) => {
            for key in [
                "access_token",
                "refresh_token",
                "Token",
                "token",
                "SessionTicket",
                "XboxToken",
                "RpsTicket",
                "Authorization",
                "identity",
                "chain",
            ] {
                if map.contains_key(key) {
                    map.insert(key.to_string(), Value::String("<redacted>".to_string()));
                }
            }
            Value::Object(map.into_iter().map(|(k, v)| (k, redact_value(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(redact_value).collect()),
        other => other,
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
