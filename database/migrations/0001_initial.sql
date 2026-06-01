PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS accounts (
  id TEXT PRIMARY KEY,
  email TEXT NOT NULL,
  gamertag TEXT,
  xuid TEXT,
  microsoft_status TEXT NOT NULL DEFAULT 'missing',
  xbox_status TEXT NOT NULL DEFAULT 'missing',
  xsts_status TEXT NOT NULL DEFAULT 'missing',
  playfab_status TEXT NOT NULL DEFAULT 'missing',
  entitlement_status TEXT NOT NULL DEFAULT 'missing',
  bedrock_auth_status TEXT NOT NULL DEFAULT 'missing',
  bot_status TEXT NOT NULL DEFAULT 'stopped',
  refresh_token_ciphertext TEXT,
  access_token_ciphertext TEXT,
  access_token_expires_at TEXT,
  last_error TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS auth_sessions (
  id TEXT PRIMARY KEY,
  account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
  device_code TEXT NOT NULL,
  user_code TEXT NOT NULL,
  verification_uri TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  interval_seconds INTEGER NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  last_error TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS entitlements (
  account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  has_entitlement INTEGER NOT NULL DEFAULT 0,
  playfab_id TEXT,
  session_ticket_ciphertext TEXT,
  minecraft_token_ciphertext TEXT,
  provisioning_status TEXT NOT NULL DEFAULT 'not_started',
  retry_count INTEGER NOT NULL DEFAULT 0,
  next_retry_at TEXT,
  last_request_id TEXT,
  last_error TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS servers (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  host TEXT NOT NULL,
  port INTEGER NOT NULL DEFAULT 19132,
  protocol_version INTEGER NOT NULL DEFAULT 975,
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  UNIQUE(host, port)
);

CREATE TABLE IF NOT EXISTS bots (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
  status TEXT NOT NULL DEFAULT 'stopped',
  reconnect_enabled INTEGER NOT NULL DEFAULT 1,
  anti_afk_enabled INTEGER NOT NULL DEFAULT 1,
  current_position TEXT,
  inventory_json TEXT NOT NULL DEFAULT '{}',
  capabilities_json TEXT NOT NULL DEFAULT '{}',
  last_join_at TEXT,
  last_leave_at TEXT,
  last_error TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  UNIQUE(account_id, server_id)
);

CREATE TABLE IF NOT EXISTS logs (
  id TEXT PRIMARY KEY,
  account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
  bot_id TEXT REFERENCES bots(id) ON DELETE SET NULL,
  level TEXT NOT NULL,
  category TEXT NOT NULL,
  step TEXT,
  request_id TEXT,
  method TEXT,
  url TEXT,
  status_code INTEGER,
  request_body TEXT,
  response_body TEXT,
  message TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_logs_created_at ON logs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_logs_account_id ON logs(account_id);
CREATE INDEX IF NOT EXISTS idx_bots_status ON bots(status);

