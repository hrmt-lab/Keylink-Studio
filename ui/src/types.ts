// 笏笏笏 Config Types 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

export interface PollingConfig {
  interval_ms: number;
}

export interface HidConfig {
  usage_page: number;
  usage: number;
  hello_timeout_ms: number;
  rescan_interval_sec: number;
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

export interface StudioConfig {
  probe_timeout_ms: number;
  keymap_read_timeout_ms: number;
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
  sessions_auto_detect: boolean;
  include_wsl_sessions: boolean;
  extra_sessions_paths: string[];
  history_fallback_enabled: boolean;
  allow_activity_baseline: boolean;
  activity_five_hour_token_baseline: number;
  activity_seven_day_token_baseline: number;
}

export interface ClaudeCodeAiUsageConfig {
  enabled: boolean;
  credentials_path: string | null;
  credentials_auto_detect: boolean;
  include_wsl_credentials: boolean;
  extra_credentials_paths: string[];
  api_timeout_sec: number;
}

export type UnmatchedAction = "clear_managed" | "keep";

export interface DeviceLayerSwitchConfig {
  display_name: string | null;
  enabled: boolean;
  rules: RuleConfig[];
  unmatched_action: UnmatchedAction | null;
}

export interface LayerSwitchConfig {
  enabled: boolean;
  unmatched_action: UnmatchedAction;
  devices: Record<string, DeviceLayerSwitchConfig>;
}

export interface AppBehaviorConfig {
  start_monitoring_on_launch: boolean;
}

export type HostActionKind =
  | "show_window"
  | "start_monitoring"
  | "stop_monitoring"
  | "refresh_ai_usage"
  | "launch"
  | "open_folder";

export interface ActionBinding {
  action_id: number;
  action: HostActionKind;
  /** Filesystem path: executable for "launch", folder for "open_folder"; null otherwise. */
  path: string | null;
  /** "open_folder" only: prefer reusing an existing Explorer window's tab (best-effort). */
  prefer_tab: boolean;
}

export interface DeviceActionsConfig {
  display_name: string | null;
  enabled: boolean;
  bindings: ActionBinding[];
}

export interface ActionsConfig {
  enabled: boolean;
  devices: Record<string, DeviceActionsConfig>;
}

export interface AppConfig {
  app: AppBehaviorConfig;
  polling: PollingConfig;
  hid: HidConfig;
  layer_switch: LayerSwitchConfig;
  time: TimeConfig;
  ai_usage: AiUsageConfig;
  studio: StudioConfig;
  actions: ActionsConfig;
}

// 笏笏笏 Runtime Types 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

export interface DeviceBatterySource {
  /** 0 = self/dongle, 1 = left, 2 = right, 3 = aux. */
  source: number;
  /** 0..100, or null when unknown / disconnected. */
  level: number | null;
}

export interface DeviceBatteryStatus {
  device_key: string;
  serial_number: string | null;
  product: string | null;
  sources: DeviceBatterySource[];
  updated_unix: number;
}

export interface DeviceLayerState {
  device_key: string;
  serial_number: string | null;
  product: string | null;
  active_layer: number;
  layer_mask: number;
}

export interface PositionCount {
  position: number;
  count: number;
}

export interface KeyStatsSummary {
  device_key: string;
  total: number;
  per_position: PositionCount[];
  days_covered: number;
}

export type StatsPeriod = "today" | "last7days" | "all";

export interface MonitorStatus {
  running: boolean;
  connected_devices: number;
  connected_device_names: string[];
  host_link_devices: DeviceInfo[];
  current_layer: number | null;
  current_rule: string | null;
  last_error: string | null;
  ai_usage: AiUsageProviderStatus[];
  device_battery: DeviceBatteryStatus[];
  device_layers: DeviceLayerState[];
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

export type AiUsageCredentialSourceKind =
  | "explicit_path"
  | "windows_default"
  | "wsl"
  | "extra_path";

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
  credential_source: AiUsageCredentialSourceKind | null;
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
  capabilities: number;
  device_uid_hash: string | null;
}


export type StudioRpcStatus = "ok" | "failed" | "timeout" | "unavailable";
export type StudioLockState = "locked" | "unlocked" | "unknown";
export type KeymapViewerStatus = "available" | "locked" | "unsupported" | "failed";
export type StudioErrorCode =
  | "none"
  | "no_serial_ports"
  | "open_failed"
  | "rpc_timeout"
  | "rpc_failed"
  | "protocol_mismatch"
  | "locked"
  | "device_not_found"
  | "keymap_read_failed";

export interface StudioDeviceStatus {
  id: string;
  connection_type: string;
  port_name: string;
  display_name: string;
  vid: number | null;
  pid: number | null;
  serial_number: string | null;
  manufacturer: string | null;
  product: string | null;
  transport_detected: boolean;
  rpc_status: StudioRpcStatus;
  lock_state: StudioLockState;
  keymap_viewer_status: KeymapViewerStatus;
  error_code: StudioErrorCode;
}

export type StudioLayoutSource = "studio_physical_layout" | "grid_fallback";

export interface StudioKeymapSnapshot {
  device_id: string;
  device_name: string;
  connection_type: string;
  lock_state: StudioLockState;
  physical_layouts: StudioPhysicalLayout[];
  selected_physical_layout_index: number | null;
  selected_physical_layout_name: string | null;
  layout_source: StudioLayoutSource;
  selected_layout_keys: StudioPhysicalKey[];
  layers: StudioLayer[];
  updated_ms: number;
}

export interface StudioPhysicalLayout {
  index: number;
  name: string;
  keys: StudioPhysicalKey[];
}

export interface StudioPhysicalKey {
  position: number;
  x: number;
  y: number;
  width: number;
  height: number;
  r: number;
  rx: number;
  ry: number;
}

export interface StudioLayer {
  index: number;
  id: number;
  name: string;
  bindings: StudioBinding[];
}

export interface StudioBinding {
  position: number;
  binding_label: string;
  primary_label: string;
  secondary_label: string;
  full_label: string;
  behavior: string;
  params: number[];
  raw: StudioRawBinding;
}

export interface StudioRawBinding {
  behavior_id: number;
  param1: number;
  param2: number;
}
export interface ProbeResult {
  device: DeviceInfo;
  verified: boolean;
  error: string | null;
}

// 笏笏笏 Page Types 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

export type Page = "dashboard" | "rules" | "actions" | "timesync" | "ai_usage" | "keymap_viewer" | "devices" | "settings";
