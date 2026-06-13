import { useEffect, useState } from "react";
import { Check, Pencil, Plus, Trash2, X, Zap } from "lucide-react";
import { saveConfig } from "../api";
import { Toggle } from "../components/Toggle";
import { useLang, type TranslationKey } from "../i18n";
import type {
  ActionBinding,
  AppConfig,
  DeviceInfo,
  HostActionKind,
} from "../types";

interface Props {
  config: AppConfig;
  setConfig: (c: AppConfig) => void;
  status: { running: boolean; host_link_devices: DeviceInfo[] };
}

const HOST_ACTION_CAPABILITY = 1 << 5;

// start_monitoring is accepted by the config but is a no-op when triggered
// over HID (monitoring is already running), so it is not offered here.
const ACTION_KINDS: HostActionKind[] = [
  "show_window",
  "stop_monitoring",
  "refresh_ai_usage",
  "launch",
  "open_folder",
];

// Kinds that require a filesystem path argument.
function needsPath(kind: HostActionKind) {
  return kind === "launch" || kind === "open_folder";
}

function isHostActionDevice(device: DeviceInfo) {
  return device.device_uid_hash !== null && (device.capabilities & HOST_ACTION_CAPABILITY) !== 0;
}

function buildDeviceTargets(config: AppConfig, capableDevices: DeviceInfo[]) {
  const targets = new Map<string, { key: string; label: string; connected: boolean }>();
  for (const [key, deviceConfig] of Object.entries(config.actions.devices ?? {})) {
    targets.set(key, { key, label: deviceConfig.display_name || key, connected: false });
  }
  for (const device of capableDevices) {
    if (!device.device_uid_hash) continue;
    const label = device.product || device.serial_number || device.device_uid_hash;
    targets.set(device.device_uid_hash, { key: device.device_uid_hash, label, connected: true });
  }
  return Array.from(targets.values()).sort((a, b) => a.label.localeCompare(b.label));
}

export default function Actions({ config, setConfig, status }: Props) {
  const { t } = useLang();
  const [targetKey, setTargetKey] = useState("");
  const [actionId, setActionId] = useState(1);
  const [kind, setKind] = useState<HostActionKind>("show_window");
  const [path, setPath] = useState("");
  const [preferTab, setPreferTab] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // null = add mode; otherwise the action_id of the binding being edited.
  const [editingActionId, setEditingActionId] = useState<number | null>(null);

  const capableDevices = status.host_link_devices.filter(isHostActionDevice);
  const deviceTargets = buildDeviceTargets(config, capableDevices);
  const selectedDeviceConfig = targetKey ? config.actions.devices[targetKey] : undefined;
  const bindings = selectedDeviceConfig?.bindings ?? [];

  // Leaving a device cancels any in-progress edit (the binding was its own) and
  // defaults the action_id to the new device's first unused id.
  useEffect(() => {
    setPath("");
    setPreferTab(false);
    setEditingActionId(null);
    setActionId(firstUnusedId(bindings));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [targetKey]);

  // Keep a valid device selected; fall back to the first known device.
  useEffect(() => {
    if (deviceTargets.length === 0) {
      if (targetKey !== "") setTargetKey("");
      return;
    }
    if (!deviceTargets.some((target) => target.key === targetKey)) {
      setTargetKey(deviceTargets[0].key);
    }
  }, [deviceTargets, targetKey]);

  const updateBindingsForTarget = (nextBindings: ActionBinding[]): AppConfig => {
    const existing = config.actions.devices[targetKey] ?? {
      display_name: deviceTargets.find((target) => target.key === targetKey)?.label ?? null,
      enabled: true,
      bindings: [],
    };
    return {
      ...config,
      actions: {
        ...config.actions,
        devices: {
          ...config.actions.devices,
          [targetKey]: { ...existing, bindings: nextBindings },
        },
      },
    };
  };

  const persist = async (updated: AppConfig) => {
    setSaving(true);
    setError(null);
    try {
      await saveConfig(updated);
      setConfig(updated);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  // Smallest action_id (1..255) not yet used; defaulted in the form when adding.
  const firstUnusedId = (list: ActionBinding[]) => {
    const used = new Set(list.map((b) => b.action_id));
    let id = 1;
    while (id < 256 && used.has(id)) id++;
    return Math.min(id, 255);
  };

  const resetForm = (nextId?: number) => {
    setPath("");
    setPreferTab(false);
    setEditingActionId(null);
    if (nextId !== undefined) setActionId(nextId);
  };

  // Bindings are stored sorted by action_id so the list renders in ID order and
  // the index-based delete below stays aligned with what the user sees.
  const sortById = (list: ActionBinding[]) =>
    [...list].sort((a, b) => a.action_id - b.action_id);

  const submitBinding = async () => {
    if (!targetKey) return;
    setError(null);
    // Allow the binding being edited to keep its own ID; reject any other clash.
    const clashes = bindings.some(
      (b) => b.action_id === actionId && b.action_id !== editingActionId
    );
    if (clashes) {
      setError(t("actions.duplicate"));
      return;
    }
    const trimmedPath = path.trim();
    if (needsPath(kind) && !trimmedPath) {
      setError(t(kind === "open_folder" ? "actions.folder_required" : "actions.path_required"));
      return;
    }
    const binding: ActionBinding = {
      action_id: actionId,
      action: kind,
      path: needsPath(kind) ? trimmedPath : null,
      prefer_tab: kind === "open_folder" ? preferTab : false,
    };
    const nextBindings =
      editingActionId === null
        ? [...bindings, binding]
        : bindings.map((b) => (b.action_id === editingActionId ? binding : b));
    await persist(updateBindingsForTarget(sortById(nextBindings)));
    resetForm(firstUnusedId(nextBindings));
  };

  const startEdit = (binding: ActionBinding) => {
    setActionId(binding.action_id);
    setKind(binding.action);
    setPath(binding.path ?? "");
    setPreferTab(binding.prefer_tab);
    setEditingActionId(binding.action_id);
    setError(null);
  };

  const deleteBinding = async (id: number) => {
    const remaining = bindings.filter((b) => b.action_id !== id);
    if (id === editingActionId) resetForm(firstUnusedId(remaining));
    await persist(updateBindingsForTarget(remaining));
  };

  const toggleEnabled = async (enabled: boolean) => {
    await persist({ ...config, actions: { ...config.actions, enabled } });
  };

  // Remove the device-specific config section entirely. Without a section
  // the device's actions are only logged.
  const deleteDeviceConfig = async () => {
    if (!targetKey || !selectedDeviceConfig) return;
    const devices = { ...config.actions.devices };
    delete devices[targetKey];
    await persist({ ...config, actions: { ...config.actions, devices } });
  };

  return (
    <div className="flex h-full flex-col overflow-hidden">

      {/* ── Header ── */}
      <div className="flex items-center justify-between border-b border-border bg-surface px-6 py-4 flex-shrink-0">
        <div>
          <h1 className="text-xl font-medium text-ink">{t("actions.title")}</h1>
          <p className="mt-0.5 text-sm text-muted">{t("actions.subtitle")}</p>
        </div>
        <div className="flex items-center gap-3">
          {deviceTargets.length === 0 ? (
            <span className="text-sm text-faint">{t("actions.no_devices")}</span>
          ) : (
            <>
              <select
                value={targetKey}
                onChange={(e) => { setTargetKey(e.target.value); setError(null); }}
                className="input !w-auto min-w-48 text-sm"
                title={t("actions.device_target")}
              >
                {deviceTargets.map((device) => (
                  <option key={device.key} value={device.key}>
                    {device.label}
                    {device.connected ? "" : ` ${t("actions.target_disconnected")}`}
                  </option>
                ))}
              </select>
              {selectedDeviceConfig && (
                <button
                  onClick={deleteDeviceConfig}
                  disabled={saving}
                  title={t("actions.delete_device_config")}
                  aria-label={t("actions.delete_device_config")}
                  className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg border border-border bg-surface text-faint hover:border-red-300 hover:text-red-500 disabled:opacity-50"
                >
                  <Trash2 size={13} />
                </button>
              )}
            </>
          )}
          <span className="text-sm text-muted">{t("actions.toggle_label")}</span>
          <Toggle
            checked={config.actions.enabled}
            onChange={toggleEnabled}
            disabled={saving}
            label={t("actions.toggle_label")}
          />
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl space-y-4 p-6">

          {!config.actions.enabled && (
            <div className="rounded-lg bg-amber-50 px-4 py-3 text-sm text-amber-700 ring-1 ring-amber-200">
              {t("actions.disabled_hint")}
            </div>
          )}

          {/* Binding builder */}
          <div className="rounded-card border border-border p-5">
            <div className="flex flex-wrap items-end gap-4">
              <div>
                <label className="mb-1 block text-xs font-medium text-muted">
                  {t("actions.action_id")}
                </label>
                <input
                  type="number"
                  min={0}
                  max={255}
                  value={actionId}
                  onChange={(e) => {
                    setActionId(Math.max(0, Math.min(255, Number(e.target.value))));
                    setError(null);
                  }}
                  className="input !w-24 !bg-surface font-mono"
                />
              </div>
              <div>
                <label className="mb-1 block text-xs font-medium text-muted">
                  {t("actions.action")}
                </label>
                <select
                  value={kind}
                  onChange={(e) => {
                    setKind(e.target.value as HostActionKind);
                    setError(null);
                  }}
                  className="input !w-auto min-w-44 cursor-pointer !bg-surface"
                >
                  {ACTION_KINDS.map((item) => (
                    <option key={item} value={item}>{actionKindLabel(item, t)}</option>
                  ))}
                </select>
              </div>
              {needsPath(kind) && (
                <div className="min-w-0 flex-1">
                  <label className="mb-1 block text-xs font-medium text-muted">
                    {t(kind === "open_folder" ? "actions.folder" : "actions.path")}
                  </label>
                  <input
                    type="text"
                    value={path}
                    onChange={(e) => {
                      setPath(e.target.value);
                      setError(null);
                    }}
                    placeholder={t(kind === "open_folder" ? "actions.folder_placeholder" : "actions.path_placeholder")}
                    className="input w-full font-mono text-sm !bg-surface"
                  />
                </div>
              )}
              <button
                onClick={submitBinding}
                disabled={saving || !targetKey}
                className="btn-neu flex flex-shrink-0 items-center gap-2 rounded-full px-5 py-2 text-sm font-medium text-ink disabled:opacity-60"
              >
                {saving
                  ? <div className="h-4 w-4 animate-spin rounded-full border-2 border-border border-t-accent" />
                  : editingActionId === null ? <Plus size={15} /> : <Check size={15} />
                }
                {editingActionId === null ? t("actions.add") : t("actions.update")}
              </button>
            </div>
            {editingActionId !== null && (
              <div className="mt-2 flex justify-end">
                <button
                  onClick={() => resetForm(firstUnusedId(bindings))}
                  disabled={saving}
                  className="flex flex-shrink-0 items-center gap-2 rounded-full border border-border px-4 py-2 text-sm font-medium text-muted hover:text-ink disabled:opacity-60"
                >
                  <X size={15} />
                  {t("actions.cancel_edit")}
                </button>
              </div>
            )}
            {kind === "open_folder" && (
              <div className="mt-4 flex items-center gap-3">
                <Toggle
                  checked={preferTab}
                  onChange={setPreferTab}
                  disabled={saving}
                  label={t("actions.prefer_tab")}
                />
                <div className="min-w-0">
                  <div className="text-sm text-ink">{t("actions.prefer_tab")}</div>
                  <div className="text-xs text-faint">{t("actions.prefer_tab.desc")}</div>
                </div>
              </div>
            )}
            {error && <p className="mt-3 text-xs text-red-500">{error}</p>}
          </div>

          {/* Binding list */}
          {bindings.length === 0 ? (
            <div className="flex flex-col items-center justify-center gap-2 py-16 text-center">
              <Zap size={32} className="text-disabled" />
              <p className="text-sm text-faint">{t("actions.empty.title")}</p>
              <p className="text-xs text-disabled">{t("actions.empty.hint")}</p>
            </div>
          ) : (
            <div className="space-y-2">
              <p className="text-xs font-medium uppercase tracking-wide text-faint">
                {t("actions.count", { n: bindings.length })}
              </p>
              {sortById(bindings).map((binding) => (
                <div
                  key={binding.action_id}
                  className={`row-lift flex items-center gap-3 rounded-card bg-surface px-4 py-3 ${
                    binding.action_id === editingActionId ? "ring-2 ring-accent" : ""
                  }`}
                >
                  <span className="inline-flex h-7 w-12 flex-shrink-0 items-center justify-center rounded-lg bg-plate font-mono text-xs font-medium text-ink">
                    {binding.action_id}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 text-sm font-medium text-ink">
                      {actionKindLabel(binding.action, t)}
                      {binding.action === "open_folder" && binding.prefer_tab && (
                        <span className="rounded bg-plate px-1.5 py-0.5 text-[10px] font-normal text-muted">
                          {t("actions.prefer_tab")}
                        </span>
                      )}
                    </div>
                    {binding.path && (
                      <div className="truncate font-mono text-[11px] text-faint">
                        {binding.path}
                      </div>
                    )}
                  </div>
                  {needsPath(binding.action) && (
                    <button
                      onClick={() => startEdit(binding)}
                      disabled={saving}
                      title={t("actions.edit")}
                      aria-label={t("actions.edit")}
                      className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg text-disabled hover:bg-plate hover:text-ink disabled:opacity-50"
                    >
                      <Pencil size={14} />
                    </button>
                  )}
                  <button
                    onClick={() => deleteBinding(binding.action_id)}
                    disabled={saving}
                    title={t("common.delete")}
                    aria-label={t("common.delete")}
                    className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg text-disabled hover:bg-red-50 hover:text-red-500 disabled:opacity-50"
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              ))}
            </div>
          )}

          <p className="text-xs text-faint">{t("actions.firmware_hint")}</p>
        </div>
      </div>
    </div>
  );
}

function actionKindLabel(
  kind: HostActionKind,
  t: (key: TranslationKey, params?: Record<string, string | number>) => string
) {
  return t(`actions.kind.${kind}` as TranslationKey);
}
