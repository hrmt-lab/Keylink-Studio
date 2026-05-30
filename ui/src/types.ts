// ─── Config Types ─────────────────────────────────────────────────────────────

export interface PollingConfig {
  interval_ms: number;
}

export interface HidConfig {
  usage_page: number;
  usage: number;
  hello_timeout_ms: number;
}

export interface RuleConfig {
  name: string;
  layer: number;
  path: string | null;
  exe: string | null;
  title: string | null;
}

export type TimeFormatHint =
  | "time_hm"
  | "time_hms"
  | "date_ymd"
  | "date_md"
  | "datetime_hm"
  | "weekday_hm";

export type ClockMode = "24h" | "12h";

export interface TimeConfig {
  enabled: boolean;
  format_hint: TimeFormatHint;
  clock_mode: ClockMode;
  periodic_sync_sec: number;
  tz_offset_min: number | null;
}

export interface AiUsageConfig {
  enabled: boolean;
  poll_interval_sec: number;
  stale_after_sec: number;
  codex: CodexAiUsageConfig;
  claude_code: ClaudeCodeAiUsageConfig;
}

export interface CodexAiUsageConfig {
  enabled: boolean;
  sessions_dir: string | null;
  history_fallback_enabled: boolean;
  allow_activity_baseline: boolean;
  activity_five_hour_token_baseline: number;
  activity_seven_day_token_baseline: number;
}

export interface ClaudeCodeAiUsageConfig {
  enabled: boolean;
  credentials_path: string | null;
  api_timeout_sec: number;
}

export interface LayerSwitchConfig {
  enabled: boolean;
  rules: RuleConfig[];
}

export interface AppConfig {
  polling: PollingConfig;
  hid: HidConfig;
  layer_switch: LayerSwitchConfig;
  time: TimeConfig;
  ai_usage: AiUsageConfig;
}

// ─── Runtime Types ────────────────────────────────────────────────────────────

export interface MonitorStatus {
  running: boolean;
  connected_devices: number;
  connected_device_names: string[];
  current_layer: number | null;
  current_rule: string | null;
  last_error: string | null;
  ai_usage: AiUsageProviderStatus[];
}

export type AiUsageStatusKind =
  | "disabled"
  | "ok"
  | "stale"
  | "no_data"
  | "missing_credentials"
  | "expired_credentials"
  | "auth_failed"
  | "rate_limited"
  | "fetch_failed"
  | "parse_failed"
  | "missing_limit";

export type AiUsageSourceKind = "none" | "quota" | "local_history";

export interface AiUsageProviderStatus {
  provider: string;
  status: AiUsageStatusKind;
  source: AiUsageSourceKind;
  updated_unix: number | null;
  stale: boolean;
  last_error_code: number | null;
  five_hour_used_bp: number | null;
  seven_day_used_bp: number | null;
  five_hour_reset_unix: number | null;
  seven_day_reset_unix: number | null;
  five_hour_valid: boolean;
  seven_day_valid: boolean;
  estimated: boolean;
  quota_source: boolean;
  local_history_source: boolean;
  fallback_limit: boolean;
  error_present: boolean;
}

export interface LogEntry {
  id: number;
  timestamp_ms: number;
  level: "info" | "warn" | "error";
  message: string;
}

export interface DeviceInfo {
  path: string;
  vendor_id: number;
  product_id: number;
  usage_page: number;
  usage: number;
  manufacturer: string | null;
  product: string | null;
  serial_number: string | null;
}

export interface ProbeResult {
  device: DeviceInfo;
  verified: boolean;
  error: string | null;
}

// ─── Page Types ───────────────────────────────────────────────────────────────

export type Page = "dashboard" | "rules" | "timesync" | "ai_usage" | "devices" | "settings";
