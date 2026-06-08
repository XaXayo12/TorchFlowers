use bytes::{Buf, BufMut, BytesMut};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use torchflower_network::Connection as RakConnection;
use torchflower_protocol::{
    LoginPacket, NetworkStackLatencyPacket, Packet, ProtocolVersion, RequestNetworkSettingsPacket,
    ResourcePackClientResponsePacket, SetLocalPlayerAsInitializedPacket,
};
use torchflower_protocol_core::{get_var_u32, put_var_u32};

#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub username: String,
    pub protocol_version: ProtocolVersion,
    pub auth_config: Option<torchflower_auth::AuthConfig>,
}

#[derive(Debug, Clone)]
pub enum ClientEvent {
    Connected,
    Disconnected(String),
    Packet(Packet),
    Text { source: String, message: String },
}

pub struct Client {
    addr: SocketAddr,
    options: ClientOptions,
    rak_conn: Option<Arc<Mutex<RakConnection>>>,
    event_tx: Option<mpsc::Sender<ClientEvent>>,
}

pub fn create_client(addr: SocketAddr, options: ClientOptions) -> Client {
    Client {
        addr,
        options,
        rak_conn: None,
        event_tx: None,
    }
}

impl Client {
    pub async fn start(&mut self) -> Result<mpsc::Receiver<ClientEvent>, anyhow::Error> {
        let (tx, rx) = mpsc::channel(1024);
        self.event_tx = Some(tx.clone());

        // Connect via RakNet
        let conn = RakConnection::connect(self.addr).await?;
        let rak_conn = Arc::new(Mutex::new(conn));
        self.rak_conn = Some(rak_conn.clone());

        let options = self.options.clone();
        tokio::spawn(async move {
            if let Err(err) = Self::run_client_loop(rak_conn, options, tx.clone()).await {
                let _ = tx.send(ClientEvent::Disconnected(err.to_string())).await;
            }
        });

        Ok(rx)
    }

    async fn run_client_loop(
        rak_conn: Arc<Mutex<RakConnection>>,
        options: ClientOptions,
        tx: mpsc::Sender<ClientEvent>,
    ) -> Result<(), anyhow::Error> {
        // 1. Send RequestNetworkSettings
        let req_settings = Packet::RequestNetworkSettings(RequestNetworkSettingsPacket {
            protocol_version: options.protocol_version.to_u32() as i32,
        });
        Self::send_raw_packet(&rak_conn, &req_settings, options.protocol_version).await?;

        // 2. Receive NetworkSettings
        let settings_bytes = {
            let mut guard = rak_conn.lock().await;
            guard.recv().await?
        };
        let mut settings_buf = settings_bytes.clone();
        if settings_buf.get(0) == Some(&0xfe) {
            settings_buf.advance(1); // skip RakNet game packet wrapper if present
        }
        // read packet payload length (varint)
        let _len = get_var_u32(&mut settings_buf)?;
        let packet_id = get_var_u32(&mut settings_buf)?;
        if packet_id != 0xc2 {
            return Err(anyhow::anyhow!(
                "Expected NetworkSettings (0xc2), got 0x{:02x}",
                packet_id
            ));
        }
        let settings = Packet::decode(0xc2, &mut settings_buf, options.protocol_version)?;

        // 3. Send Login
        // In native mode, if AuthConfig is present, we could generate Xbox live token.
        // For simple native client, we package a local mock/offline chain:
        let chain_json = format!("{{\"chain\":[\"{}\"]}}", options.username);
        let client_data_jwt = "mock.jwt.payload".to_string();

        let login_packet = Packet::Login(LoginPacket {
            protocol_version: options.protocol_version.to_u32() as i32,
            chain_json,
            client_data_jwt,
        });
        Self::send_raw_packet(&rak_conn, &login_packet, options.protocol_version).await?;

        // 4. Perform Handshake & Resource Pack Flow
        let mut play_status_received = false;
        let mut resource_packs_received = false;
        let mut resource_stack_received = false;

        loop {
            let payload = {
                let mut guard = rak_conn.lock().await;
                guard.recv().await?
            };
            let mut buf = payload.clone();
            if buf.get(0) == Some(&0xfe) {
                buf.advance(1);
            }
            // read length
            let _len = get_var_u32(&mut buf)?;
            let id = get_var_u32(&mut buf)?;
            let packet = Packet::decode(id, &mut buf, options.protocol_version)?;

            match &packet {
                Packet::PlayStatus(p) => {
                    if p.status == 0 {
                        // Login success
                        play_status_received = true;
                    }
                }
                Packet::ResourcePacksInfo(p) => {
                    resource_packs_received = true;
                    // Respond completed
                    let resp =
                        Packet::ResourcePackClientResponse(ResourcePackClientResponsePacket {
                            response_status: 3, // completed
                            resource_pack_ids: vec![],
                        });
                    Self::send_raw_packet(&rak_conn, &resp, options.protocol_version).await?;
                }
                Packet::ResourcePackStack(_) => {
                    resource_stack_received = true;
                    let resp =
                        Packet::ResourcePackClientResponse(ResourcePackClientResponsePacket {
                            response_status: 4, // completed
                            resource_pack_ids: vec![],
                        });
                    Self::send_raw_packet(&rak_conn, &resp, options.protocol_version).await?;
                }
                Packet::StartGame(p) => {
                    // Send SetLocalPlayerAsInitialized
                    let init =
                        Packet::SetLocalPlayerAsInitialized(SetLocalPlayerAsInitializedPacket {
                            runtime_entity_id: p.target_runtime_id,
                        });
                    Self::send_raw_packet(&rak_conn, &init, options.protocol_version).await?;

                    tx.send(ClientEvent::Connected).await?;
                    break;
                }
                Packet::Disconnect(p) => {
                    return Err(anyhow::anyhow!("Disconnected by server: {:?}", p.message));
                }
                _ => {}
            }
        }

        // 5. Game loop
        loop {
            let payload = {
                let mut guard = rak_conn.lock().await;
                guard.recv().await?
            };
            let mut buf = payload.clone();
            if buf.get(0) == Some(&0xfe) {
                buf.advance(1);
            }
            let _len = get_var_u32(&mut buf)?;
            let id = get_var_u32(&mut buf)?;
            let packet = Packet::decode(id, &mut buf, options.protocol_version)?;

            // Automatically respond to NetworkStackLatency
            if let Packet::NetworkStackLatency(p) = &packet {
                if p.needs_response {
                    let resp = Packet::NetworkStackLatency(NetworkStackLatencyPacket {
                        timestamp: p.timestamp,
                        needs_response: false,
                    });
                    Self::send_raw_packet(&rak_conn, &resp, options.protocol_version).await?;
                }
            }

            if let Packet::Text(p) = &packet {
                tx.send(ClientEvent::Text {
                    source: p.source_name.clone(),
                    message: p.message.clone(),
                })
                .await?;
            }

            tx.send(ClientEvent::Packet(packet)).await?;
        }
    }

    async fn send_raw_packet(
        rak_conn: &Arc<Mutex<RakConnection>>,
        packet: &Packet,
        version: ProtocolVersion,
    ) -> Result<(), anyhow::Error> {
        let payload = packet.encode(version)?;
        let mut final_buf = BytesMut::new();
        final_buf.put_u8(0xfe); // game packet header
        put_var_u32(&mut final_buf, (payload.len() + 1) as u32);
        put_var_u32(&mut final_buf, packet.id());
        final_buf.put_slice(&payload);

        let mut guard = rak_conn.lock().await;
        guard.send(final_buf.freeze()).await?;
        Ok(())
    }

    pub async fn send_packet(&mut self, packet: Packet) -> Result<(), anyhow::Error> {
        if let Some(rak_conn) = &self.rak_conn {
            Self::send_raw_packet(rak_conn, &packet, self.options.protocol_version).await?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Client not connected"))
        }
    }

    pub async fn disconnect(&mut self) -> Result<(), anyhow::Error> {
        if let Some(rak_conn) = &self.rak_conn {
            let mut guard = rak_conn.lock().await;
            guard.close().await;
        }
        Ok(())
    }
}
