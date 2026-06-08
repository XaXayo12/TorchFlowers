use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use torchflower_engine::native_client::NativeBedrockClient;
use torchflower_network::{
    native::{NativePingClient, NativePingServer},
    protocol::mcpe::motd::{Gamemode, Motd},
};

fn test_motd() -> Motd {
    Motd {
        edition: "MCPE".to_string(),
        name: "TorchFlower Native Integration".to_string(),
        sub_name: "native".to_string(),
        protocol: 975,
        version: "1.21.130".to_string(),
        player_count: 1,
        player_max: 20,
        gamemode: Gamemode::Survival,
        server_guid: 123,
        port: Some("19132".to_string()),
        ipv6_port: Some("19133".to_string()),
        nintendo_limited: Some(false),
    }
}

#[tokio::test]
async fn native_ping_client_server_vertical_slice_works() {
    let server = NativePingServer::bind("127.0.0.1:0".parse().unwrap(), test_motd())
        .await
        .unwrap();
    let addr = server.local_addr().unwrap();
    let server_task = tokio::spawn(async move { server.serve_once().await.unwrap() });

    let response = NativePingClient::new(Duration::from_secs(2))
        .ping_addr(addr)
        .await
        .unwrap();
    let served = server_task.await.unwrap();

    assert_eq!(response.server_id, 123);
    assert_eq!(response.motd.name, "TorchFlower Native Integration");
    assert_eq!(response.motd.version, "1.21.130");
    assert_eq!(served.remote_addr.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
}

#[tokio::test]
async fn engine_native_validation_reports_ping_success() {
    let server = NativePingServer::bind("127.0.0.1:0".parse().unwrap(), test_motd())
        .await
        .unwrap();
    let addr: SocketAddr = server.local_addr().unwrap();
    let server_task = tokio::spawn(async move { server.serve_once().await.unwrap() });

    let status = NativeBedrockClient::new(Duration::from_secs(2))
        .validate_ping("127.0.0.1", addr.port(), Duration::from_secs(30))
        .await
        .unwrap();
    let _ = server_task.await.unwrap();

    assert!(status.success);
    assert!(status.keepalive);
    assert!(!status.login);
    assert_eq!(status.requested_duration_seconds, 30);
    assert!(status
        .missing_capabilities
        .iter()
        .any(|item| item == "login_native_protocol_not_yet_implemented"));
}
