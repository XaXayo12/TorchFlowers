export type AuthStatus =
  | "missing"
  | "device_code_pending"
  | "authenticated"
  | "provisioned"
  | "failed"
  | "expired"
  | "pending";

export interface Account {
  id: string;
  email: string;
  gamertag?: string | null;
  xuid?: string | null;
  microsoft_status: string;
  xbox_status: string;
  xsts_status: string;
  playfab_status: string;
  entitlement_status: string;
  bedrock_auth_status: string;
  bot_status: string;
  last_error?: string | null;
  created_at: string;
  updated_at: string;
}

export interface DeviceAuthSession {
  id: string;
  account_id: string;
  user_code: string;
  verification_uri: string;
  expires_at: string;
  interval_seconds: number;
  status: string;
}

export interface Server {
  id: string;
  name: string;
  host: string;
  port: number;
  protocol_version: number;
  enabled: boolean;
}

export interface Bot {
  id: string;
  account_id: string;
  server_id: string;
  status: string;
  reconnect_enabled: boolean;
  anti_afk_enabled: boolean;
  current_position?: string | null;
  inventory_json: Record<string, unknown>;
  capabilities_json: CapabilityStatus | Record<string, unknown>;
  last_error?: string | null;
}

export interface CapabilityStatus {
  login: boolean;
  spawn: boolean;
  keepalive: boolean;
  chat: boolean;
  forms: boolean;
  inventory_transactions: boolean;
  movement: boolean;
  disconnect_handling: boolean;
  missing_capabilities: string[];
}

export interface LogEntry {
  id: string;
  account_id?: string | null;
  bot_id?: string | null;
  level: string;
  category: string;
  step?: string | null;
  request_id?: string | null;
  method?: string | null;
  url?: string | null;
  status_code?: number | null;
  request_body?: string | null;
  response_body?: string | null;
  message: string;
  metadata_json: Record<string, unknown>;
  created_at: string;
}

export interface ControlSnapshot {
  accounts: Account[];
  servers: Server[];
  bots: Bot[];
  logs: LogEntry[];
}
