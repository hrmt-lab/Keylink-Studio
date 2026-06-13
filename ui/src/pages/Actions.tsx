import { useEffect, useState } from "react";
import { Plus, Trash2, Zap } from "lucide-react";
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
];

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
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const capableDevices = status.host_link_devices.filter(isHostActionDevice);
  const deviceTargets = buildDeviceTargets(config, capableDevices);
  const selectedDeviceConfig = targetKey ? config.actions.devices[targetKey] : undefined;
  const bindings = selectedDeviceConfig?.bindings ?? [];

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

  const addBinding = async () => {
    if (!targetKey) return;
    setError(null);
    if (bindings.some((b) => b.action_id === actionId)) {
      setError(t("actions.duplicate"));
      return;
    }
    const trimmedPath = path.trim();
    if (kind === "launch" && !trimmedPath) {
      setError(t("actions.path_required"));
      return;
    }
    const binding: ActionBinding = {
      action_id: actionId,
      action: kind,
      path: kind === "launch" ? trimmedPath : null,
    };
    await persist(updateBindingsForTarget([...bindings, binding]));
    setPath("");
  };

  const deleteBinding = async (idx: number) => {
    await persist(updateBindingsForTarget(bindings.filter((_, i) => i !== idx)));
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
                  onChange={(e) => setActionId(Math.max(0, Math.min(255, Number(e.target.value))))}
                  className="input !w-24 !bg-surface font-mono"
                />
              </div>
              <div>
                <label className="mb-1 block text-xs font-medium text-muted">
                  {t("actions.action")}
                </label>
                <select
                  value={kind}
                  onChange={(e) => setKind(e.target.value as HostActionKind)}
                  className="input !w-auto min-w-44 cursor-pointer !bg-surface"
                >
                  {ACTION_KINDS.map((item) => (
                    <option key={item} value={item}>{actionKindLabel(item, t)}</option>
                  ))}
                </select>
              </div>
              {kind === "launch" && (
                <div className="min-w-0 flex-1">
                  <label className="mb-1 block text-xs font-medium text-muted">
                    {t("actions.path")}
                  </label>
                  <input
                    type="text"
                    value={path}
                    onChange={(e) => setPath(e.target.value)}
                    placeholder={t("actions.path_placeholder")}
                    className="input w-full font-mono text-sm !bg-surface"
                  />
                </div>
              )}
              <button
                onClick={addBinding}
                disabled={saving || !targetKey}
                className="btn-neu flex flex-shrink-0 items-center gap-2 rounded-full px-5 py-2 text-sm font-medium text-ink disabled:opacity-60"
              >
                {saving
                  ? <div className="h-4 w-4 animate-spin rounded-full border-2 border-border border-t-accent" />
                  : <Plus size={15} />
                }
                {t("actions.add")}
              </button>
            </div>
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
              {bindings.map((binding, idx) => (
                <div
                  key={idx}
                  className="row-lift flex items-center gap-3 rounded-card bg-surface px-4 py-3"
                >
                  <span className="inline-flex h-7 w-12 flex-shrink-0 items-center justify-center rounded-lg bg-plate font-mono text-xs font-medium text-ink">
                    {binding.action_id}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="text-sm font-medium text-ink">
                      {actionKindLabel(binding.action, t)}
                    </div>
                    {binding.path && (
                      <div className="truncate font-mono text-[11px] text-faint">
                        {binding.path}
                      </div>
                    )}
                  </div>
                  <button
                    onClick={() => deleteBinding(idx)}
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
