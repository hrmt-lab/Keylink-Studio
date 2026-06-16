import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppConfig,
  EditBehavior,
  KeyPressEvent,
  KeyCatalogEntry,
  KeyStatsSummary,
  LogEntry,
  MonitorStatus,
  ProbeResult,
  StatsPeriod,
  StudioDeviceStatus,
  StudioKeymapSnapshot,
} from "./types";

export interface ConfigLocationResult {
  path: string;
  revealed: boolean;
}

// ─── Config ───────────────────────────────────────────────────────────────────

export const getConfig = () => invoke<AppConfig>("get_config");
export const getConfigPath = () => invoke<string | null>("get_config_path");
export const saveConfig = (config: AppConfig) =>
  invoke<void>("save_config", { config });
export const reloadConfig = () => invoke<AppConfig>("reload_config");
export const showConfigFileLocation = () =>
  invoke<ConfigLocationResult>("show_config_file_location");

// ─── Status & Log ─────────────────────────────────────────────────────────────

export const getStatus = () => invoke<MonitorStatus>("get_status");
export const getLogEntries = () => invoke<LogEntry[]>("get_log_entries");

// ─── Running Apps ─────────────────────────────────────────────────────────────

export interface RunningApp {
  exe: string;
  path: string | null;
  display_name: string;
  titles: string[];
}

export const getRunningApps = () => invoke<RunningApp[]>("get_running_apps");

export const getAppIcons = (paths: string[]) =>
  invoke<Record<string, string>>("get_app_icons", { paths });

// ─── Startup ──────────────────────────────────────────────────────────────────

export const getLaunchAtLogin = () => invoke<boolean>("get_launch_at_login");
export const setLaunchAtLogin = (enabled: boolean) =>
  invoke<void>("set_launch_at_login", { enabled });

// ─── Devices ──────────────────────────────────────────────────────────────────

export const probeDevices = () => invoke<ProbeResult[]>("probe_devices");
export const probeStudioDevices = () => invoke<StudioDeviceStatus[]>("probe_studio_devices");
export const readStudioKeymap = (deviceId: string) =>
  invoke<StudioKeymapSnapshot>("read_studio_keymap", { deviceId });
export const studioKeyCatalog = () =>
  invoke<KeyCatalogEntry[]>("studio_key_catalog");
export const studioBeginEdit = (deviceId: string, forceDiscard: boolean) =>
  invoke<StudioKeymapSnapshot>("studio_begin_edit", { deviceId, forceDiscard });
export const studioSetKey = (
  deviceId: string,
  layerId: number,
  position: number,
  behavior: EditBehavior
) =>
  invoke<StudioKeymapSnapshot>("studio_set_key", {
    deviceId,
    layerId,
    position,
    behavior,
  });
export const studioSaveChanges = (deviceId: string) =>
  invoke<void>("studio_save_changes", { deviceId });
export const studioDiscardChanges = (deviceId: string) =>
  invoke<StudioKeymapSnapshot>("studio_discard_changes", { deviceId });
export const studioHasUnsaved = (deviceId: string) =>
  invoke<boolean>("studio_has_unsaved", { deviceId });
export const studioEndEdit = (deviceId: string) =>
  invoke<void>("studio_end_edit", { deviceId });

// ─── Monitoring ───────────────────────────────────────────────────────────────

export const startMonitoring = () => invoke<void>("start_monitoring");
export const stopMonitoring = () => invoke<void>("stop_monitoring");
export const refreshAiUsage = () => invoke<void>("refresh_ai_usage");

// ─── Key Stats ────────────────────────────────────────────────────────────────

export const getKeyStats = (deviceUid: string, period: StatsPeriod) =>
  invoke<KeyStatsSummary>("get_key_stats", { deviceUid, period });
export const listKeyStatsDevices = () =>
  invoke<string[]>("list_key_stats_devices");

// ─── Events ───────────────────────────────────────────────────────────────────

export const onStatusUpdate = (
  cb: (status: MonitorStatus) => void
): Promise<UnlistenFn> =>
  listen<MonitorStatus>("status-update", (e) => cb(e.payload));

export const onLogAdded = (
  cb: (entry: LogEntry) => void
): Promise<UnlistenFn> =>
  listen<LogEntry>("log-added", (e) => cb(e.payload));

export const onKeyStatsUpdated = (
  cb: (deviceKey: string) => void
): Promise<UnlistenFn> =>
  listen<string>("key-stats-updated", (e) => cb(e.payload));

export const onKeyPressEvent = (
  cb: (event: KeyPressEvent) => void
): Promise<UnlistenFn> =>
  listen<KeyPressEvent>("key-press-event", (e) => cb(e.payload));
