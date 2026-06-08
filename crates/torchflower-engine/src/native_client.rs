use std::time::Duration;

use torchflower_network::native::{NativePingClient, PingResponse};

use crate::{error::EngineError, models::CapabilityStatus};

#[derive(Debug, Clone)]
pub struct NativeBedrockClient {
    ping_timeout: Duration,
}

impl NativeBedrockClient {
    pub fn new(ping_timeout: Duration) -> Self {
        Self { ping_timeout }
    }

    pub async fn validate_ping(
        &self,
        host: &str,
        port: u16,
        requested_duration: Duration,
    ) -> Result<CapabilityStatus, EngineError> {
        let response = NativePingClient::new(self.ping_timeout)
            .ping(host, port)
            .await
            .map_err(|err| EngineError::Bedrock(err.to_string()))?;
        Ok(native_ping_status(response, requested_duration))
    }
}

impl Default for NativeBedrockClient {
    fn default() -> Self {
        Self::new(Duration::from_secs(5))
    }
}

fn native_ping_status(response: PingResponse, requested_duration: Duration) -> CapabilityStatus {
    let mut status = CapabilityStatus::default();
    status.success = true;
    status.keepalive = true;
    status.requested_duration_seconds = requested_duration.as_secs();
    status.connected_duration_seconds = 0;
    status.disconnect_reason = None;
    status.missing_capabilities = vec![
        "login_native_protocol_not_yet_implemented".to_string(),
        "spawn_native_protocol_not_yet_implemented".to_string(),
        "movement_native_protocol_not_yet_implemented".to_string(),
        "chat_native_protocol_not_yet_implemented".to_string(),
        "inventory_native_protocol_not_yet_implemented".to_string(),
        "gameplay_actions_native_protocol_not_yet_implemented".to_string(),
    ];
    status.optional_capabilities_missing = vec![
        format!("server_name={}", response.motd.name),
        format!("server_version={}", response.motd.version),
        format!("server_protocol={}", response.motd.protocol),
        format!("server_addr={}", response.remote_addr),
        format!("ping_latency_ms={}", response.latency.as_millis()),
    ];
    status
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, time::Duration};

    use torchflower_network::{
        native::{NativePingServer, PingResponse},
        protocol::mcpe::motd::{Gamemode, Motd},
    };

    use super::{native_ping_status, NativeBedrockClient};

    fn motd() -> Motd {
        Motd {
            edition: "MCPE".to_string(),
            name: "TorchFlower Native Test".to_string(),
            sub_name: "native".to_string(),
            protocol: 975,
            version: "1.21.130".to_string(),
            player_count: 1,
            player_max: 20,
            gamemode: Gamemode::Survival,
            server_guid: 99,
            port: Some("19132".to_string()),
            ipv6_port: Some("19133".to_string()),
            nintendo_limited: Some(false),
        }
    }

    #[tokio::test]
    async fn native_engine_validation_uses_ping_client() {
        let server = NativePingServer::bind("127.0.0.1:0".parse().unwrap(), motd())
            .await
            .unwrap();
        let addr = server.local_addr().unwrap();
        let server_task = tokio::spawn(async move { server.serve_once().await.unwrap() });

        let status = NativeBedrockClient::new(Duration::from_secs(2))
            .validate_ping("127.0.0.1", addr.port(), Duration::from_secs(30))
            .await
            .unwrap();
        let _served = server_task.await.unwrap();

        assert!(status.success);
        assert!(status.keepalive);
        assert!(!status.login);
        assert!(status
            .missing_capabilities
            .contains(&"login_native_protocol_not_yet_implemented".to_string()));
    }

    #[test]
    fn native_ping_status_carries_server_metadata() {
        let response = PingResponse {
            remote_addr: SocketAddr::from(([127, 0, 0, 1], 19132)),
            latency: Duration::from_millis(12),
            timestamp: 1,
            server_id: 99,
            motd: motd(),
        };
        let status = native_ping_status(response, Duration::from_secs(10));
        assert!(status.success);
        assert!(status
            .optional_capabilities_missing
            .iter()
            .any(|item| item == "server_protocol=975"));
    }
}
