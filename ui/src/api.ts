import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type { AppConfig, MonitorStatus, LogEntry, ProbeResult } from "./types";

// ─── Config ───────────────────────────────────────────────────────────────────

export const getConfig = () => invoke<AppConfig>("get_config");
export const getConfigPath = () => invoke<string | null>("get_config_path");
export const saveConfig = (config: AppConfig) =>
  invoke<void>("save_config", { config });
export const reloadConfig = () => invoke<AppConfig>("reload_config");

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

// ─── Devices ──────────────────────────────────────────────────────────────────

export const probeDevices = () => invoke<ProbeResult[]>("probe_devices");

// ─── Monitoring ───────────────────────────────────────────────────────────────

export const startMonitoring = () => invoke<void>("start_monitoring");
export const stopMonitoring = () => invoke<void>("stop_monitoring");

// ─── Events ───────────────────────────────────────────────────────────────────

export const onStatusUpdate = (
  cb: (status: MonitorStatus) => void
): Promise<UnlistenFn> =>
  listen<MonitorStatus>("status-update", (e) => cb(e.payload));

export const onLogAdded = (
  cb: (entry: LogEntry) => void
): Promise<UnlistenFn> =>
  listen<LogEntry>("log-added", (e) => cb(e.payload));
