use std::{collections::VecDeque, fmt, future::Future, pin::Pin, sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::sync::Mutex;
use torchflower_auth::AuthConfig;
use torchflower_proto::{Packet, ProtocolVersion};

use crate::{
    config::Config, db::Database, error::EngineError, models::CapabilityStatus,
    native_client::NativeBedrockClient,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ServerAddress {
    pub host: String,
    pub port: u16,
}

impl ServerAddress {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Position {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotation {
    pub yaw: f32,
    pub pitch: f32,
}

impl Rotation {
    pub const fn new(yaw: f32, pitch: f32) -> Self {
        Self { yaw, pitch }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockPosition {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPosition {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InventoryItem {
    pub slot: u32,
    pub item_id: i32,
    pub count: u32,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BotEvent {
    Connected(CapabilityStatus),
    Disconnected { reason: Option<String> },
    Chat { message: String },
    PositionChanged(Position),
    RotationChanged(Rotation),
    InventoryChanged,
    DeathDetected,
    Respawned,
    ScoreboardChanged(Vec<String>),
    ActionbarChanged(String),
    TitleChanged(String),
}

#[derive(Debug, thiserror::Error)]
pub enum BotError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("missing required builder field: {0}")]
    MissingBuilderField(&'static str),
    #[error("bot session is not connected")]
    NotConnected,
    #[error("safe automation policy does not allow {action}")]
    UnsafeAutomationDisabled { action: &'static str },
    #[error("server host is not allowed by automation policy: {0}")]
    HostNotAllowed(String),
}

pub type BotResult<T> = Result<T, BotError>;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Connected,
    Spawned,
    Chat { sender: String, message: String },
    Disconnected { reason: Option<String> },
}

#[derive(Debug)]
pub struct SessionCtx {
    server: ServerAddress,
    protocol_version: ProtocolVersion,
    outbound_chat: Vec<String>,
    disconnected: bool,
}

impl SessionCtx {
    pub fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    pub fn server(&self) -> &ServerAddress {
        &self.server
    }

    pub async fn send_chat(&mut self, message: impl Into<String>) -> BotResult<()> {
        self.outbound_chat.push(message.into());
        Ok(())
    }

    pub async fn disconnect(&mut self) -> BotResult<()> {
        self.disconnected = true;
        Ok(())
    }
}

#[async_trait]
pub trait Session: Send + Sync {
    async fn on_connect(&mut self, _ctx: &mut SessionCtx) {}
    async fn on_packet(&mut self, _ctx: &mut SessionCtx, _packet: Packet) {}
    async fn on_disconnect(&mut self, _ctx: &mut SessionCtx, _reason: Option<String>) {}
    async fn on_form_request(
        &mut self,
        _ctx: &mut SessionCtx,
        _form: torchflower_proto::ModalFormRequest,
    ) {
    }
}

#[derive(Debug, Clone)]
pub struct BotBuilder {
    server: Option<ServerAddress>,
    protocol_version: ProtocolVersion,
    auth: Option<AuthConfig>,
}

impl BotBuilder {
    pub fn new() -> Self {
        Self {
            server: None,
            protocol_version: ProtocolVersion::default(),
            auth: None,
        }
    }

    pub fn address(mut self, host: impl Into<String>, port: u16) -> Self {
        self.server = Some(ServerAddress::new(host, port));
        self
    }

    pub fn protocol_version(mut self, version: ProtocolVersion) -> Self {
        self.protocol_version = version;
        self
    }

    pub fn auth(mut self, auth: AuthConfig) -> Self {
        self.auth = Some(auth);
        self
    }

    pub async fn build(self) -> BotResult<Bot> {
        Ok(Bot {
            server: self
                .server
                .ok_or(BotError::MissingBuilderField("address"))?,
            protocol_version: self.protocol_version,
            auth: self.auth.ok_or(BotError::MissingBuilderField("auth"))?,
        })
    }
}

impl Default for BotBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct Bot {
    server: ServerAddress,
    protocol_version: ProtocolVersion,
    auth: AuthConfig,
}

impl Bot {
    pub async fn run<F>(self, mut handler: F) -> BotResult<()>
    where
        F: for<'a> FnMut(
            &'a mut SessionCtx,
            Event,
        ) -> Pin<Box<dyn Future<Output = BotResult<()>> + Send + 'a>>,
    {
        let mut ctx = SessionCtx {
            server: self.server,
            protocol_version: self.protocol_version,
            outbound_chat: Vec::new(),
            disconnected: false,
        };
        let _auth = self.auth;
        handler(&mut ctx, Event::Connected).await?;
        handler(&mut ctx, Event::Spawned).await?;
        if !ctx.disconnected {
            handler(&mut ctx, Event::Disconnected { reason: None }).await?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct AutomationPolicy {
    pub allow_gameplay_actions: bool,
    pub allowed_hosts: Vec<String>,
}

impl AutomationPolicy {
    pub fn allow_for_hosts(hosts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            allow_gameplay_actions: true,
            allowed_hosts: hosts.into_iter().map(Into::into).collect(),
        }
    }

    pub fn permits_host(&self, host: &str) -> bool {
        let host = host.trim().to_ascii_lowercase();
        self.allowed_hosts.iter().any(|allowed| {
            let allowed = allowed.trim().to_ascii_lowercase();
            allowed == "*" || allowed == host
        })
    }

    fn require_gameplay_action(&self, host: &str, action: &'static str) -> BotResult<()> {
        if !self.allow_gameplay_actions {
            return Err(BotError::UnsafeAutomationDisabled { action });
        }
        if !self.permits_host(host) {
            return Err(BotError::HostNotAllowed(host.to_string()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BotAction {
    Chat(String),
    MoveTo(Position),
    Look(Rotation),
    Jump,
    Sneak(bool),
    Attack,
    Interact(BlockPosition),
    BreakBlock(BlockPosition),
    PlaceBlock(BlockPosition),
    UseItem,
    OpenInventory,
    ClickSlot(u32),
    Respawn,
}

#[derive(Debug, Default)]
pub struct ActionScheduler {
    pending: VecDeque<BotAction>,
}

impl ActionScheduler {
    pub fn schedule(&mut self, action: BotAction) {
        self.pending.push_back(action);
    }

    pub fn pop_next(&mut self) -> Option<BotAction> {
        self.pending.pop_front()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct InventoryTracker {
    items: Vec<InventoryItem>,
}

impl InventoryTracker {
    pub fn items(&self) -> &[InventoryItem] {
        &self.items
    }

    pub fn replace_items(&mut self, items: Vec<InventoryItem>) {
        self.items = items;
    }

    pub fn selected_placeable(&self) -> Option<&InventoryItem> {
        self.items
            .iter()
            .find(|item| item.count > 0 && item.item_id > 0)
    }
}

#[derive(Debug, Default)]
pub struct ServerStateTracker {
    position: Option<Position>,
    rotation: Option<Rotation>,
    dead: bool,
    scoreboard: Vec<String>,
    actionbar: Option<String>,
    title: Option<String>,
}

impl ServerStateTracker {
    pub fn position(&self) -> Option<Position> {
        self.position
    }

    pub fn rotation(&self) -> Option<Rotation> {
        self.rotation
    }

    pub fn is_dead(&self) -> bool {
        self.dead
    }

    pub fn scoreboard(&self) -> &[String] {
        &self.scoreboard
    }

    pub fn actionbar(&self) -> Option<&str> {
        self.actionbar.as_deref()
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    fn set_position(&mut self, position: Position) {
        self.position = Some(position);
    }

    fn set_rotation(&mut self, rotation: Rotation) {
        self.rotation = Some(rotation);
    }
}

#[derive(Debug, Default)]
pub struct MovementController {
    target: Option<Position>,
    rotation: Option<Rotation>,
    sneaking: bool,
}

impl MovementController {
    pub fn target(&self) -> Option<Position> {
        self.target
    }

    pub fn rotation(&self) -> Option<Rotation> {
        self.rotation
    }

    pub fn sneaking(&self) -> bool {
        self.sneaking
    }

    fn move_to(&mut self, position: Position) {
        self.target = Some(position);
    }

    fn look(&mut self, rotation: Rotation) {
        self.rotation = Some(rotation);
    }

    fn sneak(&mut self, enabled: bool) {
        self.sneaking = enabled;
    }
}

#[derive(Debug, Default)]
pub struct KeepAliveController {
    interval: Duration,
}

impl KeepAliveController {
    pub fn with_interval(interval: Duration) -> Self {
        Self { interval }
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }
}

#[derive(Debug)]
pub struct BlockInteractionController {
    policy: AutomationPolicy,
    server: ServerAddress,
}

impl BlockInteractionController {
    fn new(policy: AutomationPolicy, server: ServerAddress) -> Self {
        Self { policy, server }
    }

    pub fn can_modify_blocks(&self) -> bool {
        self.policy.allow_gameplay_actions && self.policy.permits_host(&self.server.host)
    }
}

#[derive(Default)]
pub struct BotSessionBuilder {
    config: Option<Config>,
    db: Option<Database>,
    account_id: Option<String>,
    server: Option<ServerAddress>,
    automation_policy: AutomationPolicy,
}

impl fmt::Debug for BotSessionBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BotSessionBuilder")
            .field("config", &self.config.as_ref().map(|_| "<configured>"))
            .field("db", &self.db.as_ref().map(|_| "<configured>"))
            .field("account_id", &self.account_id)
            .field("server", &self.server)
            .field("automation_policy", &self.automation_policy)
            .finish()
    }
}

impl BotSessionBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn config(mut self, config: Config) -> Self {
        self.config = Some(config);
        self
    }

    pub fn database(mut self, db: Database) -> Self {
        self.db = Some(db);
        self
    }

    pub fn account(mut self, account_id: impl Into<String>) -> Self {
        self.account_id = Some(account_id.into());
        self
    }

    pub fn account_id(self, account_id: impl Into<String>) -> Self {
        self.account(account_id)
    }

    pub fn server(mut self, server: ServerAddress) -> Self {
        self.server = Some(server);
        self
    }

    pub fn automation_policy(mut self, policy: AutomationPolicy) -> Self {
        self.automation_policy = policy;
        self
    }

    pub async fn build(self) -> BotResult<BotSession> {
        let config = self.config.ok_or(BotError::MissingBuilderField("config"))?;
        let db = self.db.ok_or(BotError::MissingBuilderField("database"))?;
        let account_id = self
            .account_id
            .ok_or(BotError::MissingBuilderField("account"))?;
        let server = self.server.ok_or(BotError::MissingBuilderField("server"))?;
        Ok(BotSession::new(
            config,
            db,
            account_id,
            server,
            self.automation_policy,
        ))
    }
}

pub struct BotSession {
    _config: Config,
    db: Database,
    account_id: String,
    server: ServerAddress,
    policy: AutomationPolicy,
    connected: bool,
    last_status: Option<CapabilityStatus>,
    state: Arc<Mutex<ServerStateTracker>>,
    inventory: Arc<Mutex<InventoryTracker>>,
    movement: Arc<Mutex<MovementController>>,
    scheduler: Arc<Mutex<ActionScheduler>>,
    keepalive: KeepAliveController,
    blocks: BlockInteractionController,
}

impl BotSession {
    pub fn builder() -> BotSessionBuilder {
        BotSessionBuilder::new()
    }

    fn new(
        config: Config,
        db: Database,
        account_id: String,
        server: ServerAddress,
        policy: AutomationPolicy,
    ) -> Self {
        Self {
            blocks: BlockInteractionController::new(policy.clone(), server.clone()),
            _config: config,
            db,
            account_id,
            server,
            policy,
            connected: false,
            last_status: None,
            state: Arc::new(Mutex::new(ServerStateTracker::default())),
            inventory: Arc::new(Mutex::new(InventoryTracker::default())),
            movement: Arc::new(Mutex::new(MovementController::default())),
            scheduler: Arc::new(Mutex::new(ActionScheduler::default())),
            keepalive: KeepAliveController::with_interval(Duration::from_secs(30)),
        }
    }

    /// Validates that the configured Bedrock server responds on the native RakNet ping path.
    pub async fn connect(&mut self) -> BotResult<CapabilityStatus> {
        let status = self.validate_for(Duration::from_secs(30), false).await?;
        self.connected = status.success;
        self.last_status = Some(status.clone());
        Ok(status)
    }

    /// Runs the native Bedrock server reachability validation through the public session wrapper.
    pub async fn validate_for(
        &self,
        duration: Duration,
        _run_gameplay_validation: bool,
    ) -> BotResult<CapabilityStatus> {
        self.db.get_account(&self.account_id).await?;
        Ok(NativeBedrockClient::default()
            .validate_ping(&self.server.host, self.server.port, duration)
            .await?)
    }

    /// Marks the high-level session disconnected and emits no packets if no persistent transport is active.
    pub async fn disconnect(&mut self) -> BotResult<()> {
        self.connected = false;
        Ok(())
    }

    /// Queues a chat message for a persistent session implementation.
    pub async fn chat(&self, message: impl Into<String>) -> BotResult<()> {
        self.require_connected()?;
        self.scheduler
            .lock()
            .await
            .schedule(BotAction::Chat(message.into()));
        Ok(())
    }

    /// Queues a movement target.
    pub async fn move_to(&self, position: Position) -> BotResult<()> {
        self.require_connected()?;
        self.movement.lock().await.move_to(position);
        self.state.lock().await.set_position(position);
        self.scheduler
            .lock()
            .await
            .schedule(BotAction::MoveTo(position));
        Ok(())
    }

    /// Queues a look/rotation update.
    pub async fn look(&self, rotation: Rotation) -> BotResult<()> {
        self.require_connected()?;
        self.movement.lock().await.look(rotation);
        self.state.lock().await.set_rotation(rotation);
        self.scheduler
            .lock()
            .await
            .schedule(BotAction::Look(rotation));
        Ok(())
    }

    /// Queues a jump action.
    pub async fn jump(&self) -> BotResult<()> {
        self.require_connected()?;
        self.scheduler.lock().await.schedule(BotAction::Jump);
        Ok(())
    }

    /// Queues a sneak toggle.
    pub async fn sneak(&self, enabled: bool) -> BotResult<()> {
        self.require_connected()?;
        self.movement.lock().await.sneak(enabled);
        self.scheduler
            .lock()
            .await
            .schedule(BotAction::Sneak(enabled));
        Ok(())
    }

    /// Queues an attack action when gameplay automation is explicitly allowed.
    pub async fn attack(&self) -> BotResult<()> {
        self.require_gameplay_allowed("attack")?;
        self.scheduler.lock().await.schedule(BotAction::Attack);
        Ok(())
    }

    /// Queues an interaction against a block position when gameplay automation is explicitly allowed.
    pub async fn interact(&self, position: BlockPosition) -> BotResult<()> {
        self.require_gameplay_allowed("interact")?;
        self.scheduler
            .lock()
            .await
            .schedule(BotAction::Interact(position));
        Ok(())
    }

    /// Queues a block break when gameplay automation is explicitly allowed.
    pub async fn break_block(&self, position: BlockPosition) -> BotResult<()> {
        self.require_gameplay_allowed("break_block")?;
        self.scheduler
            .lock()
            .await
            .schedule(BotAction::BreakBlock(position));
        Ok(())
    }

    /// Queues a block placement when gameplay automation is explicitly allowed.
    pub async fn place_block(&self, position: BlockPosition) -> BotResult<()> {
        self.require_gameplay_allowed("place_block")?;
        self.scheduler
            .lock()
            .await
            .schedule(BotAction::PlaceBlock(position));
        Ok(())
    }

    /// Queues a held-item use when gameplay automation is explicitly allowed.
    pub async fn use_item(&self) -> BotResult<()> {
        self.require_gameplay_allowed("use_item")?;
        self.scheduler.lock().await.schedule(BotAction::UseItem);
        Ok(())
    }

    /// Queues an inventory-open action when gameplay automation is explicitly allowed.
    pub async fn open_inventory(&self) -> BotResult<()> {
        self.require_gameplay_allowed("open_inventory")?;
        self.scheduler
            .lock()
            .await
            .schedule(BotAction::OpenInventory);
        Ok(())
    }

    /// Queues an inventory slot click when gameplay automation is explicitly allowed.
    pub async fn click_slot(&self, slot: u32) -> BotResult<()> {
        self.require_gameplay_allowed("click_slot")?;
        self.scheduler
            .lock()
            .await
            .schedule(BotAction::ClickSlot(slot));
        Ok(())
    }

    /// Queues a respawn action.
    pub async fn respawn(&self) -> BotResult<()> {
        self.require_connected()?;
        self.scheduler.lock().await.schedule(BotAction::Respawn);
        Ok(())
    }

    pub async fn is_dead(&self) -> bool {
        self.state.lock().await.is_dead()
    }

    pub async fn scoreboard(&self) -> Vec<String> {
        self.state.lock().await.scoreboard().to_vec()
    }

    pub async fn actionbar(&self) -> Option<String> {
        self.state.lock().await.actionbar().map(ToOwned::to_owned)
    }

    pub async fn title(&self) -> Option<String> {
        self.state.lock().await.title().map(ToOwned::to_owned)
    }

    pub fn last_status(&self) -> Option<&CapabilityStatus> {
        self.last_status.as_ref()
    }

    pub fn movement_controller(&self) -> Arc<Mutex<MovementController>> {
        self.movement.clone()
    }

    pub fn inventory_tracker(&self) -> Arc<Mutex<InventoryTracker>> {
        self.inventory.clone()
    }

    pub fn server_state_tracker(&self) -> Arc<Mutex<ServerStateTracker>> {
        self.state.clone()
    }

    pub fn action_scheduler(&self) -> Arc<Mutex<ActionScheduler>> {
        self.scheduler.clone()
    }

    pub fn keepalive_controller(&self) -> &KeepAliveController {
        &self.keepalive
    }

    pub fn block_interaction_controller(&self) -> &BlockInteractionController {
        &self.blocks
    }

    fn require_connected(&self) -> BotResult<()> {
        self.connected.then_some(()).ok_or(BotError::NotConnected)
    }

    fn require_gameplay_allowed(&self, action: &'static str) -> BotResult<()> {
        self.require_connected()?;
        self.policy
            .require_gameplay_action(&self.server.host, action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MicrosoftAuthFlow;
    use torchflower_net::{
        native::NativePingServer,
        protocol::mcpe::motd::{Gamemode, Motd},
    };

    #[test]
    fn automation_policy_requires_allowed_host() {
        let policy = AutomationPolicy::allow_for_hosts(["example.org"]);
        assert!(policy
            .require_gameplay_action("example.org", "break_block")
            .is_ok());
        assert!(matches!(
            policy.require_gameplay_action("other.org", "break_block"),
            Err(BotError::HostNotAllowed(_))
        ));
    }

    #[test]
    fn scheduler_preserves_action_order() {
        let mut scheduler = ActionScheduler::default();
        scheduler.schedule(BotAction::Jump);
        scheduler.schedule(BotAction::Chat("hello".to_string()));
        assert_eq!(scheduler.pop_next(), Some(BotAction::Jump));
        assert_eq!(
            scheduler.pop_next(),
            Some(BotAction::Chat("hello".to_string()))
        );
        assert!(scheduler.is_empty());
    }

    #[test]
    fn inventory_tracker_selects_non_air_items() {
        let mut tracker = InventoryTracker::default();
        tracker.replace_items(vec![
            InventoryItem {
                slot: 0,
                item_id: 0,
                count: 0,
                display_name: None,
            },
            InventoryItem {
                slot: 1,
                item_id: 5,
                count: 2,
                display_name: Some("planks".to_string()),
            },
        ]);
        assert_eq!(tracker.selected_placeable().map(|item| item.slot), Some(1));
    }

    #[tokio::test]
    async fn fake_lifecycle_schedules_public_actions() {
        let db = crate::db::Database::connect("sqlite::memory:")
            .await
            .unwrap();
        db.migrate().await.unwrap();
        let config = Config {
            microsoft_client_id: "client".to_string(),
            microsoft_auth_flow: MicrosoftAuthFlow::Live,
            token_encryption_key: [3u8; 32],
            database_url: "sqlite::memory:".to_string(),
            rust_engine_bind: "127.0.0.1:0".to_string(),
            api_key: Some("key".to_string()),
            dev_allow_unauth_api: false,
            cors_allowed_origins: vec!["http://localhost:3000".to_string()],
            allowed_server_hosts: vec!["example.org".to_string()],
            dangerous_log_auth_bodies: false,
        };
        let mut session = BotSession::new(
            config,
            db,
            "account".to_string(),
            ServerAddress::new("example.org", 19132),
            AutomationPolicy::allow_for_hosts(["example.org"]),
        );
        session.connected = true;

        session.chat("hello").await.unwrap();
        session
            .move_to(Position::new(1.0, 64.0, 1.0))
            .await
            .unwrap();
        session.look(Rotation::new(90.0, 0.0)).await.unwrap();
        session
            .break_block(BlockPosition::new(1, 63, 1))
            .await
            .unwrap();

        let scheduler = session.action_scheduler();
        let mut scheduler = scheduler.lock().await;
        assert_eq!(scheduler.len(), 4);
        assert_eq!(
            scheduler.pop_next(),
            Some(BotAction::Chat("hello".to_string()))
        );
        assert_eq!(
            scheduler.pop_next(),
            Some(BotAction::MoveTo(Position::new(1.0, 64.0, 1.0)))
        );
    }

    #[tokio::test]
    async fn bot_session_validation_uses_native_ping_layer() {
        let db = crate::db::Database::connect("sqlite::memory:")
            .await
            .unwrap();
        db.migrate().await.unwrap();
        let account_id = db.upsert_account_email("native@example.com").await.unwrap();
        let motd = Motd {
            edition: "MCPE".to_string(),
            name: "TorchFlower Native Session Test".to_string(),
            sub_name: "native".to_string(),
            protocol: 975,
            version: "1.21.130".to_string(),
            player_count: 1,
            player_max: 20,
            gamemode: Gamemode::Survival,
            server_guid: 7007,
            port: Some("19132".to_string()),
            ipv6_port: Some("19133".to_string()),
            nintendo_limited: Some(false),
        };
        let server = NativePingServer::bind("127.0.0.1:0".parse().unwrap(), motd)
            .await
            .unwrap();
        let addr = server.local_addr().unwrap();
        let server_task = tokio::spawn(async move { server.serve_once().await.unwrap() });
        let config = Config {
            microsoft_client_id: "client".to_string(),
            microsoft_auth_flow: MicrosoftAuthFlow::Live,
            token_encryption_key: [3u8; 32],
            database_url: "sqlite::memory:".to_string(),
            rust_engine_bind: "127.0.0.1:0".to_string(),
            api_key: Some("key".to_string()),
            dev_allow_unauth_api: false,
            cors_allowed_origins: vec!["http://localhost:3000".to_string()],
            allowed_server_hosts: vec!["127.0.0.1".to_string()],
            dangerous_log_auth_bodies: false,
        };
        let session = BotSession::new(
            config,
            db,
            account_id,
            ServerAddress::new("127.0.0.1", addr.port()),
            AutomationPolicy::allow_for_hosts(["127.0.0.1"]),
        );

        let status = session
            .validate_for(Duration::from_secs(30), false)
            .await
            .unwrap();
        let _ = server_task.await.unwrap();

        assert!(status.success);
        assert!(status.keepalive);
        assert!(!status.login);
        assert!(status
            .optional_capabilities_missing
            .iter()
            .any(|item| item == "server_name=TorchFlower Native Session Test"));
    }
}
