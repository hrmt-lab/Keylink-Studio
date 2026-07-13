import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppConfig,
  DiscardChangesDto,
  ResetToKeymapDto,
  EditBehavior,
  EncoderBindingsDto,
  EncoderInfoDto,
  ComboInfoDto,
  ComboItemDto,
  ComboItemInputDto,
  KeyPressEvent,
  KeyCatalogEntry,
  KeyStatsSummary,
  LogEntry,
  MonitorStatus,
  ProbeResult,
  SaveOrDiscardResultDto,
  StudioResyncEditStateDto,
  StatsPeriod,
  StudioDeviceStatus,
  StudioBindingLabelPatch,
  StudioKeymapSnapshot,
  StudioRawBinding,
  RestoreReport,
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
export const studioExportKeymap = (deviceId: string, path: string, hostLinkUid: string | null) =>
  invoke<void>("studio_export_keymap", { deviceId, path, hostLinkUid });
export const studioPreviewKeymapRestore = (deviceId: string, path: string, hostLinkUid: string | null) =>
  invoke<RestoreReport>("studio_preview_keymap_restore", { deviceId, path, hostLinkUid });
export const studioApplyKeymapRestore = (deviceId: string, path: string, hostLinkUid: string | null) =>
  invoke<[StudioKeymapSnapshot, RestoreReport]>("studio_apply_keymap_restore", { deviceId, path, hostLinkUid });
export const studioKeyCatalog = () =>
  invoke<KeyCatalogEntry[]>("studio_key_catalog");
export const resolveStudioBehaviorLabels = (deviceId: string, rawBindings: StudioRawBinding[]) =>
  invoke<StudioBindingLabelPatch[]>("resolve_studio_behavior_labels", { deviceId, rawBindings });
export const studioBeginEdit = (
  deviceId: string,
  forceDiscard: boolean,
  labelPatches: StudioBindingLabelPatch[] = []
) =>
  invoke<StudioKeymapSnapshot>("studio_begin_edit", { deviceId, forceDiscard, labelPatches });
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
export const studioAddLayer = (deviceId: string, name: string) =>
  invoke<StudioKeymapSnapshot>("studio_add_layer", { deviceId, name });
export const studioRenameLayer = (deviceId: string, layerId: number, name: string) =>
  invoke<StudioKeymapSnapshot>("studio_rename_layer", { deviceId, layerId, name });
export const studioRemoveLayer = (deviceId: string, layerIndex: number) =>
  invoke<StudioKeymapSnapshot>("studio_remove_layer", { deviceId, layerIndex });
export const studioSaveChanges = (deviceId: string, hostLinkUid: string | null, comboHostLinkUid: string | null) =>
  invoke<SaveOrDiscardResultDto>("studio_save_changes", { deviceId, hostLinkUid, comboHostLinkUid });
export const studioDiscardChanges = (deviceId: string, hostLinkUid: string | null, comboHostLinkUid: string | null) =>
  invoke<DiscardChangesDto>("studio_discard_changes", { deviceId, hostLinkUid, comboHostLinkUid });
export const studioResetToKeymap = (deviceId: string, hostLinkUid: string | null, comboHostLinkUid: string | null) =>
  invoke<ResetToKeymapDto>("studio_reset_to_keymap", { deviceId, hostLinkUid, comboHostLinkUid });
export const studioHasUnsaved = (deviceId: string) =>
  invoke<boolean>("studio_has_unsaved", { deviceId });
export const studioResyncEditState = (deviceId: string) =>
  invoke<StudioResyncEditStateDto>("studio_resync_edit_state", { deviceId });
export const studioEndEdit = (deviceId: string) =>
  invoke<void>("studio_end_edit", { deviceId });
export const studioAbortEdit = (deviceId: string) =>
  invoke<void>("studio_abort_edit", { deviceId });

// ─── Encoder Config RPC ───────────────────────────────────────────────────────

export const readEncoderInfo = (hostLinkUid: string) =>
  invoke<EncoderInfoDto>("read_encoder_info", { hostLinkUid });
export const readEncoderBindings = (
  deviceId: string,
  hostLinkUid: string,
  layerId: number,
  encoderId: number
) =>
  invoke<EncoderBindingsDto>("read_encoder_bindings", {
    deviceId,
    hostLinkUid,
    layerId,
    encoderId,
  });
export const readEncoderLayerBindings = (
  deviceId: string,
  hostLinkUid: string,
  layerId: number,
  encoderCount: number
) =>
  invoke<EncoderBindingsDto[]>("read_encoder_layer_bindings", {
    deviceId,
    hostLinkUid,
    layerId,
    encoderCount,
  });
// Either side may be null to keep the current runtime override binding for that
// direction. Both sides are required for the first edit of a `source=keymap`
// encoder (the backend rejects a partial initial edit).
export const studioSetEncoderBindings = (
  deviceId: string,
  hostLinkUid: string,
  layerId: number,
  encoderId: number,
  cw: EditBehavior | null,
  ccw: EditBehavior | null
) =>
  invoke<EncoderBindingsDto>("studio_set_encoder_bindings", {
    deviceId,
    hostLinkUid,
    layerId,
    encoderId,
    cw,
    ccw,
  });
export const studioEncoderHasUnsaved = (hostLinkUid: string) =>
  invoke<boolean>("studio_encoder_has_unsaved", { hostLinkUid });
export const studioEncoderSave = (hostLinkUid: string) =>
  invoke<void>("studio_encoder_save", { hostLinkUid });
export const studioEncoderDiscard = (hostLinkUid: string) =>
  invoke<void>("studio_encoder_discard", { hostLinkUid });
export const studioEncoderClearOverride = (
  hostLinkUid: string,
  layerId: number,
  encoderId: number
) =>
  invoke<void>("studio_encoder_clear_override", { hostLinkUid, layerId, encoderId });

// Combo Config RPC
export const readComboInfo = (hostLinkUid: string) =>
  invoke<ComboInfoDto>("read_combo_info", { hostLinkUid });
export const readCombo = (deviceId: string, hostLinkUid: string, slot: number) =>
  invoke<ComboItemDto>("read_combo", { deviceId, hostLinkUid, slot });
export const studioSetCombo = (deviceId: string, hostLinkUid: string, item: ComboItemInputDto) =>
  invoke<ComboItemDto>("studio_set_combo", { deviceId, hostLinkUid, item });
export const studioComboHasUnsaved = (hostLinkUid: string) =>
  invoke<boolean>("studio_combo_has_unsaved", { hostLinkUid });
export const studioComboSave = (hostLinkUid: string) =>
  invoke<void>("studio_combo_save", { hostLinkUid });
export const studioComboDiscard = (hostLinkUid: string) =>
  invoke<void>("studio_combo_discard", { hostLinkUid });
export const studioComboDelete = (hostLinkUid: string, slot: number) =>
  invoke<void>("studio_combo_delete", { hostLinkUid, slot });
export const studioComboResetToKeymap = (hostLinkUid: string) =>
  invoke<void>("studio_combo_reset_to_keymap", { hostLinkUid });

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
