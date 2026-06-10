import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type { AppConfig, MonitorStatus, LogEntry, ProbeResult, StudioDeviceStatus, StudioKeymapSnapshot } from "./types";

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

// ─── Monitoring ───────────────────────────────────────────────────────────────

export const startMonitoring = () => invoke<void>("start_monitoring");
export const stopMonitoring = () => invoke<void>("stop_monitoring");
export const refreshAiUsage = () => invoke<void>("refresh_ai_usage");

// ─── Events ───────────────────────────────────────────────────────────────────

export const onStatusUpdate = (
  cb: (status: MonitorStatus) => void
): Promise<UnlistenFn> =>
  listen<MonitorStatus>("status-update", (e) => cb(e.payload));

export const onLogAdded = (
  cb: (entry: LogEntry) => void
): Promise<UnlistenFn> =>
  listen<LogEntry>("log-added", (e) => cb(e.payload));
