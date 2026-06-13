import { useState, useEffect, useCallback } from "react";
import { RefreshCw, Search, Trash2, Check, Plus } from "lucide-react";
import { saveConfig, getRunningApps, getAppIcons, type RunningApp } from "../api";
import { Toggle } from "../components/Toggle";
import { LayerBadge } from "../components/LayerBadge";
import { useLang } from "../i18n";
import type { AppConfig, DeviceInfo, RuleConfig } from "../types";

interface Props {
  config: AppConfig;
  setConfig: (c: AppConfig) => void;
  status: { running: boolean; host_link_devices: DeviceInfo[] };
}

// Match priority: exe → path → title
// exe is always available from running apps, so we always use it.
function buildRule(app: RunningApp, layer: number): RuleConfig {
  return {
    name: app.display_name || app.exe.replace(/\.exe$/i, ""),
    layer,
    exe: app.exe || null,
    path: null,
    title: null,
  };
}

export default function Rules({ config, setConfig, status }: Props) {
  const { t } = useLang();
  const [apps, setApps] = useState<RunningApp[]>([]);
  const [iconByExe, setIconByExe] = useState<Record<string, string>>({});
  const [loadingApps, setLoadingApps] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<RunningApp | null>(null);
  const [layer, setLayer] = useState(1);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [justAdded, setJustAdded] = useState<string | null>(null);
  const [targetKey, setTargetKey] = useState("");

  const appLayerDevices = status.host_link_devices.filter(isAppLayerDevice);
  const deviceTargets = buildDeviceTargets(config, appLayerDevices);
  const selectedDeviceConfig = targetKey ? config.layer_switch.devices[targetKey] : undefined;
  const rules = selectedDeviceConfig?.rules ?? [];
  const maxLayer = 31; // ZMK max layers (0-31)

  const loadApps = useCallback(async () => {
    setLoadingApps(true);
    try {
      const list = await getRunningApps();
      setApps(list);
      try {
        const paths = list.map((a) => a.path).filter((p): p is string => Boolean(p));
        const iconsByPath = await getAppIcons(paths);
        const byExe: Record<string, string> = {};
        for (const app of list) {
          if (app.path && iconsByPath[app.path]) byExe[app.exe] = iconsByPath[app.path];
        }
        setIconByExe(byExe);
      } catch {
        // Icons are best-effort; fall back to initials.
      }
    } finally {
      setLoadingApps(false);
    }
  }, []);

  useEffect(() => { loadApps(); }, [loadApps]);

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

  const filtered = apps.filter((a) => {
    if (!query) return true;
    const q = query.toLowerCase();
    return a.display_name.toLowerCase().includes(q) || a.exe.toLowerCase().includes(q);
  });


  const updateRulesForTarget = (nextRules: RuleConfig[]): AppConfig => {
    const existing = config.layer_switch.devices[targetKey] ?? {
      display_name: deviceTargets.find((target) => target.key === targetKey)?.label ?? null,
      enabled: true,
      rules: [],
      unmatched_action: null,
    };
    return {
      ...config,
      layer_switch: {
        ...config.layer_switch,
        devices: {
          ...config.layer_switch.devices,
          [targetKey]: { ...existing, rules: nextRules },
        },
      },
    };
  };
  const addRule = async () => {
    if (!selected || !targetKey) return;
    setError(null);
    const newRule = buildRule(selected, layer);
    if (rules.some((r) => r.exe === newRule.exe && r.layer === newRule.layer)) {
      setError(t("rules.duplicate"));
      return;
    }
    const updated = updateRulesForTarget([...rules, newRule]);
    setSaving(true);
    try {
      await saveConfig(updated);
      setConfig(updated);
      setJustAdded(selected.exe);
      setTimeout(() => setJustAdded(null), 2000);
      setSelected(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const deleteRule = async (idx: number) => {
    const updated = updateRulesForTarget(rules.filter((_, i) => i !== idx));
    setSaving(true);
    try {
      await saveConfig(updated);
      setConfig(updated);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const toggleEnabled = async (enabled: boolean) => {
    const updated = { ...config, layer_switch: { ...config.layer_switch, enabled } };
    await saveConfig(updated);
    setConfig(updated);
  };

  // Remove the device-specific config section entirely. Without a section the
  // device is no longer layer-managed (there is no global fallback).
  const deleteDeviceConfig = async () => {
    if (!targetKey || !selectedDeviceConfig) return;
    const devices = { ...config.layer_switch.devices };
    delete devices[targetKey];
    const updated = { ...config, layer_switch: { ...config.layer_switch, devices } };
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

  return (
    <div className="flex h-full flex-col overflow-hidden">

      {/* ── Header ── */}
      <div className="flex items-center justify-between border-b border-border bg-surface px-6 py-4 flex-shrink-0">
        <div>
          <h1 className="text-xl font-medium text-ink">{t("rules.title")}</h1>
          <p className="mt-0.5 text-sm text-muted">{t("rules.subtitle")}</p>
        </div>
        <div className="flex items-center gap-3">
          {deviceTargets.length === 0 ? (
            <span className="text-sm text-faint">{t("rules.no_devices")}</span>
          ) : (
            <>
              <select
                value={targetKey}
                onChange={(e) => { setTargetKey(e.target.value); setSelected(null); setError(null); }}
                className="input !w-auto min-w-48 text-sm"
                title={t("rules.device_target")}
              >
                {deviceTargets.map((device) => (
                  <option key={device.key} value={device.key}>
                    {device.label}
                    {device.connected ? "" : ` ${t("rules.target_disconnected")}`}
                  </option>
                ))}
              </select>
              {selectedDeviceConfig && (
                <button
                  onClick={deleteDeviceConfig}
                  disabled={saving}
                  title={t("rules.delete_device_config")}
                  aria-label={t("rules.delete_device_config")}
                  className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg border border-border bg-surface text-faint hover:border-red-300 hover:text-red-500 disabled:opacity-50"
                >
                  <Trash2 size={13} />
                </button>
              )}
            </>
          )}
          <span className="text-sm text-muted">{t("rules.toggle_label")}</span>
          <Toggle checked={config.layer_switch.enabled} onChange={toggleEnabled} disabled={saving} label={t("rules.toggle_label")} />
        </div>
      </div>

      <div className="flex flex-1 overflow-hidden">

        {/* ══ Left: App Picker ══ */}
        <div className="flex w-64 flex-shrink-0 flex-col border-r border-border bg-background">

          {/* Search + Refresh */}
          <div className="flex items-center gap-2 border-b border-border px-3 py-2.5">
            <div className="relative flex-1">
              <Search size={13} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-faint" />
              <input
                className="w-full rounded-lg border border-border bg-surface py-1.5 pl-7 pr-2 text-sm text-ink placeholder-faint focus:border-accent focus:outline-none"
                placeholder={t("rules.search_placeholder")}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
            <button
              onClick={loadApps}
              disabled={loadingApps}
              title={t("common.refresh")}
              aria-label={t("common.refresh")}
              className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg border border-border bg-surface text-muted hover:text-ink disabled:opacity-50"
            >
              <RefreshCw size={13} className={loadingApps ? "animate-spin" : ""} />
            </button>
          </div>

          {/* App list */}
          <div className="flex-1 overflow-y-auto p-2">
            {loadingApps && apps.length === 0 ? (
              <div className="flex h-32 items-center justify-center">
                <div className="h-5 w-5 animate-spin rounded-full border-2 border-border border-t-accent" />
              </div>
            ) : filtered.length === 0 ? (
              <p className="mt-8 text-center text-xs text-faint">{t("rules.not_found")}</p>
            ) : (
              <ul className="space-y-0.5">
                {filtered.map((app) => {
                  const isSelected = selected?.exe === app.exe;
                  const existingRule = rules.find((r) => r.exe === app.exe);
                  return (
                    <li key={app.exe}>
                      <button
                        onClick={() => { setSelected(isSelected ? null : app); setError(null); }}
                        className={`group flex w-full items-center gap-2.5 rounded-pill px-3 py-2 text-left ${
                          isSelected
                            ? "bg-surface shadow-neu-sel"
                            : "row-lift bg-transparent hover:bg-surface text-ink"
                        }`}
                      >
                        <AppAvatar name={app.display_name} icon={iconByExe[app.exe]} selected={isSelected} />
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1">
                            <span className={`truncate text-sm font-medium ${isSelected ? "text-accent-deep" : "text-ink"}`}>
                              {app.display_name}
                            </span>
                            {justAdded === app.exe && (
                              <Check size={11} className="flex-shrink-0 text-accent-deep" />
                            )}
                          </div>
                          <span className={`block truncate font-mono text-[11px] ${isSelected ? "text-accent-deep/60" : "text-faint"}`}>
                            {app.exe}
                          </span>
                        </div>
                        {existingRule && !isSelected && (
                          <LayerBadge layer={existingRule.layer} size="sm" />
                        )}
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        </div>

        {/* ══ Right ══ */}
        <div className="flex flex-1 flex-col overflow-hidden min-w-0">

          {/* Rule Builder */}
          <div className="flex-shrink-0 border-b border-border px-5 py-4 min-h-[80px]">
            {selected ? (
              <div className="flex items-center gap-4 flex-wrap">
                {/* App */}
                <div className="flex items-center gap-3 min-w-0 flex-shrink-0">
                  <AppAvatar name={selected.display_name} icon={iconByExe[selected.exe]} size="lg" />
                  <div className="min-w-0">
                    <div className="font-medium text-ink truncate max-w-[160px]">
                      {selected.display_name}
                    </div>
                    <div className="font-mono text-[11px] text-faint truncate max-w-[160px]">
                      {selected.exe}
                    </div>
                  </div>
                </div>

                <div className="h-10 w-px bg-border flex-shrink-0" />

                {/* Layer dropdown */}
                <div className="flex items-center gap-2 flex-shrink-0">
                  <span className="text-sm text-muted whitespace-nowrap">{t("rules.layer")}</span>
                  <select
                    value={layer}
                    onChange={(e) => setLayer(Number(e.target.value))}
                    className="input !w-20 cursor-pointer !bg-surface font-mono"
                  >
                    {Array.from({ length: maxLayer + 1 }, (_, i) => (
                      <option key={i} value={i}>L{i}</option>
                    ))}
                  </select>
                </div>

                {/* Add button */}
                <button
                  onClick={addRule}
                  disabled={saving || !targetKey}
                  className="btn-neu flex flex-shrink-0 items-center gap-2 rounded-full px-5 py-2 text-sm font-medium text-ink disabled:opacity-60"
                >
                  {saving
                    ? <div className="h-4 w-4 animate-spin rounded-full border-2 border-border border-t-accent" />
                    : <Plus size={15} />
                  }
                  {t("rules.add")}
                </button>

                {error && (
                  <span className="text-xs text-red-500">{error}</span>
                )}
              </div>
            ) : (
              <p className="flex h-full items-center text-sm text-faint">
                {t("rules.select_hint")}
              </p>
            )}
          </div>

          {/* Rule list */}
          <div className="flex-1 overflow-y-auto p-4">
            {rules.length === 0 ? (
              <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
                <span className="text-5xl opacity-10">⌨</span>
                <p className="text-sm text-faint">{t("rules.empty.title")}</p>
                <p className="text-xs text-disabled">{t("rules.empty.hint")}</p>
              </div>
            ) : (
              <div className="space-y-2">
                <p className="mb-3 text-xs font-medium uppercase tracking-wide text-faint">
                  {t("rules.count", { n: rules.length })}
                </p>
                {rules.map((rule, idx) => (
                  <RuleCard
                    key={idx}
                    rule={rule}
                    icon={rule.exe ? iconByExe[rule.exe] : undefined}
                    onDelete={() => deleteRule(idx)}
                    disabled={saving}
                  />
                ))}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── Avatar ───────────────────────────────────────────────────────────────────

const APP_LAYER_CAPABILITY = 1 << 0;

function isAppLayerDevice(device: DeviceInfo) {
  return device.device_uid_hash !== null && (device.capabilities & APP_LAYER_CAPABILITY) !== 0;
}

function buildDeviceTargets(config: AppConfig, appLayerDevices: DeviceInfo[]) {
  const targets = new Map<string, { key: string; label: string; connected: boolean }>();
  for (const [key, deviceConfig] of Object.entries(config.layer_switch.devices ?? {})) {
    targets.set(key, { key, label: deviceConfig.display_name || key, connected: false });
  }
  for (const device of appLayerDevices) {
    if (!device.device_uid_hash) continue;
    const label = device.product || device.serial_number || device.device_uid_hash;
    targets.set(device.device_uid_hash, { key: device.device_uid_hash, label, connected: true });
  }
  return Array.from(targets.values()).sort((a, b) => a.label.localeCompare(b.label));
}

function AppAvatar({ name, icon, selected = false, size = "sm" }: {
  name: string; icon?: string; selected?: boolean; size?: "sm" | "lg";
}) {
  const sizeClass = size === "lg" ? "h-10 w-10" : "h-8 w-8";
  if (icon) {
    return (
      <div className={`flex flex-shrink-0 items-center justify-center overflow-hidden rounded-lg bg-white/60 ${sizeClass}`}>
        <img src={icon} alt="" aria-hidden="true" className="h-full w-full object-contain" draggable={false} />
      </div>
    );
  }
  return (
    <div className={`flex flex-shrink-0 items-center justify-center rounded-lg font-medium ${
      size === "lg" ? "text-base" : "text-sm"
    } ${sizeClass} ${selected ? "bg-accent-soft text-accent-deep" : "bg-plate text-muted"}`}>
      {(name[0] ?? "?").toUpperCase()}
    </div>
  );
}

// ─── Rule Card ────────────────────────────────────────────────────────────────

function RuleCard({ rule, icon, onDelete, disabled }: {
  rule: RuleConfig; icon?: string; onDelete: () => void; disabled: boolean;
}) {
  const { t } = useLang();
  const matchDesc = rule.exe ?? rule.title ?? rule.path ?? "—";
  const matchKind = rule.exe ? "exe" : rule.title ? "title" : "path";

  return (
    <div className="group row-lift flex items-center gap-4 rounded-card bg-surface px-4 py-3">
      <LayerBadge layer={rule.layer} />
      {icon && (
        <img src={icon} alt="" aria-hidden="true" className="h-7 w-7 flex-shrink-0 object-contain" draggable={false} />
      )}
      <div className="min-w-0 flex-1">
        <div className="font-medium text-ink">{rule.name}</div>
        <div className="mt-0.5 flex items-center gap-1.5">
          <span className="rounded bg-plate px-1.5 py-0.5 font-mono text-[10px] text-muted">
            {matchKind}
          </span>
          <span className="truncate font-mono text-[11px] text-muted">{matchDesc}</span>
        </div>
      </div>
      <button
        onClick={onDelete}
        disabled={disabled}
        className="flex-shrink-0 rounded-lg p-1.5 text-disabled opacity-0 transition-all group-hover:opacity-100 hover:bg-red-50 hover:text-red-400 disabled:opacity-30"
        title={t("common.delete")}
        aria-label={t("common.delete")}
      >
        <Trash2 size={14} />
      </button>
    </div>
  );
}
