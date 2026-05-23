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

export interface LayerSwitchConfig {
  enabled: boolean;
  rules: RuleConfig[];
}

export interface AppConfig {
  polling: PollingConfig;
  hid: HidConfig;
  layer_switch: LayerSwitchConfig;
  time: TimeConfig;
}

// ─── Runtime Types ────────────────────────────────────────────────────────────

export interface MonitorStatus {
  running: boolean;
  connected_devices: number;
  connected_device_names: string[];
  current_layer: number | null;
  current_rule: string | null;
  last_error: string | null;
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
  hello_ok: boolean;
  error: string | null;
}

// ─── Page Types ───────────────────────────────────────────────────────────────

export type Page = "dashboard" | "rules" | "timesync" | "devices" | "settings";
