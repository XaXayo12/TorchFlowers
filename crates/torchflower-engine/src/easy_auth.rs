/// Simple, zero-config Microsoft device-code authentication for TorchFlower.
///
/// This module is gated behind the `easy-auth` feature flag. It drives the
/// full Bedrock provisioning flow — device code → Microsoft OAuth → Xbox →
/// XSTS → PlayFab → Minecraft entitlement — inside a single async call with
/// no HTTP server, no database, and no config file required.
///
/// Tokens are cached on disk in `~/.torchflower/tokens/` so subsequent runs
/// authenticate silently using the stored refresh token.
///
/// # Example
///
/// ```rust,no_run
/// use torchflower_engine::easy_auth::DeviceCodeLogin;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let session = DeviceCodeLogin::new()
///         .start("your@email.com")
///         .await?;
///
///     println!("Logged in as account {}", session.account_id);
///     Ok(())
/// }
/// ```
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::{
    auth::{
        entitlement::EntitlementProvisioner, microsoft::MicrosoftAuth, ProvisionedBedrockSession,
    },
    config::{Config, BEDROCK_PROTOCOL_LIVE_CLIENT_ID},
    db::Database,
    error::EngineResult,
};

// ---------------------------------------------------------------------------
// Persisted token cache (stored on disk, one file per account e-mail)
// ---------------------------------------------------------------------------

/// Minimal set of tokens persisted to disk so subsequent runs skip the
/// interactive device-code prompt and use the stored refresh token instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedTokens {
    account_id: String,
    access_token: String,
    refresh_token: String,
    /// Seconds-since-Unix-epoch at which the access token expires.
    expires_at_secs: i64,
}

// ---------------------------------------------------------------------------
// Public result type
// ---------------------------------------------------------------------------

/// A fully provisioned Bedrock session returned by [`DeviceCodeLogin::start`].
pub struct EasyAuthSession {
    /// The internal account UUID assigned by the engine database.
    pub account_id: String,
    /// The full Bedrock provisioning result, ready for use with the bot.
    pub provisioned: ProvisionedBedrockSession,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Zero-config Microsoft device-code login builder.
///
/// Internally spins up an in-process SQLite database and drives the full
/// Bedrock provisioning chain. The resulting [`EasyAuthSession`] can be used
/// directly without any external engine process.
///
/// ```rust,no_run
/// # use torchflower_engine::easy_auth::DeviceCodeLogin;
/// # #[tokio::main] async fn main() -> anyhow::Result<()> {
/// let session = DeviceCodeLogin::new()
///     .start("your@email.com")
///     .await?;
/// println!("account_id = {}", session.account_id);
/// # Ok(()) }
/// ```
pub struct DeviceCodeLogin {
    /// Directory where the ephemeral SQLite database is stored.
    /// Defaults to `~/.torchflower/db`.
    db_dir: Option<PathBuf>,
    /// Directory where per-account token cache files are stored.
    /// Defaults to `~/.torchflower/tokens`.
    token_cache_dir: Option<PathBuf>,
    /// Microsoft client id. Defaults to the public Bedrock/Live client id.
    client_id: Option<String>,
    /// Polling interval while waiting for the user to complete browser login.
    /// Defaults to 5 seconds (matches Microsoft's recommended minimum).
    poll_interval: Duration,
    /// Whether to suppress all stdout output. Defaults to `false`.
    silent: bool,
}

impl Default for DeviceCodeLogin {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceCodeLogin {
    /// Create a new builder with default settings.
    pub fn new() -> Self {
        Self {
            db_dir: None,
            token_cache_dir: None,
            client_id: None,
            poll_interval: Duration::from_secs(5),
            silent: false,
        }
    }

    /// Override the directory used for the internal SQLite database.
    pub fn db_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.db_dir = Some(path.into());
        self
    }

    /// Override the directory used for token cache files.
    pub fn token_cache_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.token_cache_dir = Some(path.into());
        self
    }

    /// Override the Microsoft OAuth client id (advanced use only).
    pub fn client_id(mut self, id: impl Into<String>) -> Self {
        self.client_id = Some(id.into());
        self
    }

    /// Override the poll interval. Must be >= 5 s to respect Microsoft's rate
    /// limits.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval.max(Duration::from_secs(5));
        self
    }

    /// Suppress all stdout progress output.
    pub fn silent(mut self) -> Self {
        self.silent = true;
        self
    }

    /// Run the interactive login flow for `email`.
    ///
    /// - If a valid cached token exists on disk the interactive prompt is
    ///   skipped and provisioning runs silently with the stored refresh token.
    /// - Otherwise prints a URL + code to stdout and polls until the user
    ///   completes browser login.
    pub async fn start(self, email: &str) -> anyhow::Result<EasyAuthSession> {
        let home = home_dir()?;

        let db_dir = self
            .db_dir
            .unwrap_or_else(|| home.join(".torchflower").join("db"));
        let token_dir = self
            .token_cache_dir
            .unwrap_or_else(|| home.join(".torchflower").join("tokens"));

        std::fs::create_dir_all(&db_dir)?;
        std::fs::create_dir_all(&token_dir)?;

        let client_id = self
            .client_id
            .unwrap_or_else(|| BEDROCK_PROTOCOL_LIVE_CLIENT_ID.to_string());

        // Build a minimal in-process Config — no API key, no server required.
        let config = Config::easy_auth(client_id.clone())?;

        let db_path = db_dir.join("easy_auth.sqlite");
        let database_url = format!("sqlite://{}", db_path.display());
        let db = Database::connect(&database_url).await?;
        db.migrate().await?;

        let microsoft = MicrosoftAuth::new(&config, db.clone());
        let provisioner = EntitlementProvisioner::new(&config, db.clone());

        // ----------------------------------------------------------------
        // Fast path: check the on-disk token cache first
        // ----------------------------------------------------------------
        let cache_path = token_cache_path(&token_dir, email);
        if let Some(cached) = load_cached_tokens(&cache_path) {
            if !self.silent {
                println!("[torchflower] Cached tokens found — skipping browser login.");
            }
            // Try to provision with the cached refresh token. On failure
            // (e.g. refresh token expired) fall through to the interactive flow.
            match try_provision_from_cache(&config, &db, &provisioner, &cached).await {
                Ok(provisioned) => {
                    if !self.silent {
                        println!("[torchflower] Session provisioned from cache.");
                    }
                    return Ok(EasyAuthSession {
                        account_id: cached.account_id,
                        provisioned,
                    });
                }
                Err(err) => {
                    if !self.silent {
                        println!(
                            "[torchflower] Cached token refresh failed ({err}), \
                             falling back to interactive login."
                        );
                    }
                    let _ = std::fs::remove_file(&cache_path);
                }
            }
        }

        // ----------------------------------------------------------------
        // Interactive path: device-code flow
        // ----------------------------------------------------------------
        let session = microsoft.start_device_auth(email).await?;

        if !self.silent {
            println!();
            println!("[torchflower] === Microsoft Device Login ===");
            println!("[torchflower]  1. Open : {}", session.verification_uri);
            println!("[torchflower]  2. Enter: {}", session.user_code);
            println!();
            println!("[torchflower] Waiting for you to complete login in the browser...");
        }

        let tokens = loop {
            tokio::time::sleep(self.poll_interval).await;
            match microsoft.poll_device_auth(&session.id).await? {
                Some(t) => break t,
                None => {
                    if !self.silent {
                        print!(".");
                        std::io::stdout().flush().ok();
                    }
                }
            }
        };

        if !self.silent {
            println!();
            println!("[torchflower] Microsoft login successful — provisioning Bedrock session...");
        }

        provisioner
            .save_microsoft_tokens(&session.account_id, &tokens)
            .await?;

        // Persist refresh token to disk for next run.
        let expires_at_secs = chrono::Utc::now().timestamp() + tokens.expires_in.max(3600) - 60;
        let cached = CachedTokens {
            account_id: session.account_id.clone(),
            access_token: tokens.access_token.clone(),
            refresh_token: tokens.refresh_token.clone(),
            expires_at_secs,
        };
        save_cached_tokens(&cache_path, &cached);

        let provisioned = provisioner.provision(&session.account_id).await?;

        if !self.silent {
            println!("[torchflower] Session provisioned successfully.");
        }

        Ok(EasyAuthSession {
            account_id: session.account_id,
            provisioned,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn home_dir() -> anyhow::Result<PathBuf> {
    // std::env::home_dir is deprecated but still correct on Windows/Linux/mac.
    #[allow(deprecated)]
    std::env::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))
}

fn token_cache_path(dir: &Path, email: &str) -> PathBuf {
    // Use a simple hex-encoded hash so special chars in e-mail don't cause path issues.
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    email.to_ascii_lowercase().hash(&mut hasher);
    let hash = hasher.finish();
    dir.join(format!("{hash:016x}.json"))
}

fn load_cached_tokens(path: &Path) -> Option<CachedTokens> {
    let data = std::fs::read_to_string(path).ok()?;
    let cached: CachedTokens = serde_json::from_str(&data).ok()?;
    // Reject tokens that are already expired.
    if cached.expires_at_secs <= chrono::Utc::now().timestamp() {
        return None;
    }
    Some(cached)
}

fn save_cached_tokens(path: &Path, cached: &CachedTokens) {
    if let Ok(json) = serde_json::to_string_pretty(cached) {
        let _ = std::fs::write(path, json);
    }
}

/// Attempt to re-use cached Microsoft tokens by saving them and immediately
/// running `provision`. This refreshes if needed because `TokenManager` will
/// detect expiry and use the refresh token automatically.
async fn try_provision_from_cache(
    _config: &Config,
    _db: &Database,
    provisioner: &EntitlementProvisioner,
    cached: &CachedTokens,
) -> EngineResult<ProvisionedBedrockSession> {
    // Re-hydrate a fake MicrosoftTokenResponse so we can call save_microsoft_tokens.
    let tokens = crate::auth::microsoft::MicrosoftTokenResponse {
        token_type: "Bearer".to_string(),
        scope: String::new(),
        expires_in: (cached.expires_at_secs - chrono::Utc::now().timestamp()).max(0),
        access_token: cached.access_token.clone(),
        refresh_token: cached.refresh_token.clone(),
    };
    provisioner
        .save_microsoft_tokens(&cached.account_id, &tokens)
        .await?;
    provisioner.provision(&cached.account_id).await
}
