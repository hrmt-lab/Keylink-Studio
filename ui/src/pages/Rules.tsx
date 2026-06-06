import { useState, useEffect, useCallback } from "react";
import { RefreshCw, Search, Trash2, Check, Plus } from "lucide-react";
import { saveConfig, getRunningApps, type RunningApp } from "../api";
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
  const [loadingApps, setLoadingApps] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<RunningApp | null>(null);
  const [layer, setLayer] = useState(1);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [justAdded, setJustAdded] = useState<string | null>(null);
  const [targetKey, setTargetKey] = useState("global");

  const appLayerDevices = status.host_link_devices.filter(isAppLayerDevice);
  const deviceTargets = buildDeviceTargets(config, appLayerDevices);
  const selectedDeviceConfig = targetKey === "global" ? null : config.layer_switch.devices[targetKey];
  const rules = targetKey === "global" ? config.layer_switch.rules : selectedDeviceConfig?.rules ?? [];
  const maxLayer = 31; // ZMK max layers (0-31)

  const loadApps = useCallback(async () => {
    setLoadingApps(true);
    try {
      setApps(await getRunningApps());
    } finally {
      setLoadingApps(false);
    }
  }, []);

  useEffect(() => { loadApps(); }, [loadApps]);

  useEffect(() => {
    if (targetKey !== "global" && !deviceTargets.some((target) => target.key === targetKey)) {
      setTargetKey("global");
    }
  }, [deviceTargets, targetKey]);

  const filtered = apps.filter((a) => {
    if (!query) return true;
    const q = query.toLowerCase();
    return a.display_name.toLowerCase().includes(q) || a.exe.toLowerCase().includes(q);
  });


  const updateRulesForTarget = (nextRules: RuleConfig[]): AppConfig => {
    if (targetKey === "global") {
      return {
        ...config,
        layer_switch: { ...config.layer_switch, rules: nextRules },
      };
    }
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
    if (!selected) return;
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

  return (
    <div className="flex h-full flex-col overflow-hidden">

      {/* ── Header ── */}
      <div className="flex items-center justify-between border-b border-border/60 bg-white px-6 py-4 flex-shrink-0">
        <div>
          <h1 className="text-xl font-semibold text-gray-800">{t("rules.title")}</h1>
          <p className="mt-0.5 text-sm text-gray-500">{t("rules.subtitle")}</p>
        </div>
        <div className="flex items-center gap-3">
          <select
            value={targetKey}
            onChange={(e) => { setTargetKey(e.target.value); setSelected(null); setError(null); }}
            className="input min-w-48 text-sm"
            title={t("rules.device_target")}
          >
            <option value="global">{t("rules.global_fallback")}</option>
            {deviceTargets.map((device) => (
              <option key={device.key} value={device.key}>
                {device.label}
              </option>
            ))}
          </select>
          <span className="text-sm text-gray-600">{t("rules.toggle_label")}</span>
          <Toggle checked={config.layer_switch.enabled} onChange={toggleEnabled} disabled={saving} />
        </div>
      </div>

      <div className="flex flex-1 overflow-hidden">

        {/* ══ Left: App Picker ══ */}
        <div className="flex w-64 flex-shrink-0 flex-col border-r border-border/60 bg-background">

          {/* Search + Refresh */}
          <div className="flex items-center gap-2 border-b border-border/60 px-3 py-2.5">
            <div className="relative flex-1">
              <Search size={13} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-400" />
              <input
                className="w-full rounded-lg border border-border bg-white py-1.5 pl-7 pr-2 text-sm placeholder-gray-400 focus:border-primary focus:outline-none"
                placeholder={t("rules.search_placeholder")}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
            <button
              onClick={loadApps}
              disabled={loadingApps}
              title="更新"
              className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg border border-border bg-white text-gray-500 hover:border-primary hover:text-primary disabled:opacity-50"
            >
              <RefreshCw size={13} className={loadingApps ? "animate-spin" : ""} />
            </button>
          </div>

          {/* App list */}
          <div className="flex-1 overflow-y-auto p-2">
            {loadingApps && apps.length === 0 ? (
              <div className="flex h-32 items-center justify-center">
                <div className="h-5 w-5 animate-spin rounded-full border-2 border-border border-t-primary" />
              </div>
            ) : filtered.length === 0 ? (
              <p className="mt-8 text-center text-xs text-gray-400">{t("rules.not_found")}</p>
            ) : (
              <ul className="space-y-0.5">
                {filtered.map((app) => {
                  const isSelected = selected?.exe === app.exe;
                  const existingRule = rules.find((r) => r.exe === app.exe);
                  return (
                    <li key={app.exe}>
                      <button
                        onClick={() => { setSelected(isSelected ? null : app); setError(null); }}
                        className={`group flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left transition-all ${
                          isSelected
                            ? "bg-primary text-white"
                            : "hover:bg-white hover:shadow-sm text-gray-700"
                        }`}
                      >
                        <AppAvatar name={app.display_name} selected={isSelected} />
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1">
                            <span className={`truncate text-sm font-medium ${isSelected ? "text-white" : "text-gray-800"}`}>
                              {app.display_name}
                            </span>
                            {justAdded === app.exe && (
                              <Check size={11} className="flex-shrink-0 text-emerald-400" />
                            )}
                          </div>
                          <span className={`block truncate font-mono text-[11px] ${isSelected ? "text-white/60" : "text-gray-400"}`}>
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
          <div className="flex-shrink-0 border-b border-border/60 bg-white px-5 py-4 min-h-[80px]">
            {selected ? (
              <div className="flex items-center gap-4 flex-wrap">
                {/* App */}
                <div className="flex items-center gap-3 min-w-0 flex-shrink-0">
                  <AppAvatar name={selected.display_name} size="lg" />
                  <div className="min-w-0">
                    <div className="font-semibold text-gray-800 truncate max-w-[160px]">
                      {selected.display_name}
                    </div>
                    <div className="font-mono text-[11px] text-gray-400 truncate max-w-[160px]">
                      {selected.exe}
                    </div>
                  </div>
                </div>

                <div className="h-10 w-px bg-border flex-shrink-0" />

                {/* Layer dropdown */}
                <div className="flex items-center gap-2 flex-shrink-0">
                  <span className="text-sm text-gray-600 whitespace-nowrap">{t("rules.layer")}</span>
                  <select
                    value={layer}
                    onChange={(e) => setLayer(Number(e.target.value))}
                    className="input w-20 cursor-pointer"
                  >
                    {Array.from({ length: maxLayer + 1 }, (_, i) => (
                      <option key={i} value={i}>L{i}</option>
                    ))}
                  </select>
                </div>

                {/* Add button */}
                <button
                  onClick={addRule}
                  disabled={saving}
                  className="flex flex-shrink-0 items-center gap-2 rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-white hover:bg-primary-dark disabled:opacity-60 transition-colors"
                >
                  {saving
                    ? <div className="h-4 w-4 animate-spin rounded-full border-2 border-white/30 border-t-white" />
                    : <Plus size={15} />
                  }
                  {t("rules.add")}
                </button>

                {error && (
                  <span className="text-xs text-red-500">{error}</span>
                )}
              </div>
            ) : (
              <p className="flex h-full items-center text-sm text-gray-400">
                {t("rules.select_hint")}
              </p>
            )}
          </div>

          {/* Rule list */}
          <div className="flex-1 overflow-y-auto p-4">
            {rules.length === 0 ? (
              <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
                <span className="text-5xl opacity-10">⌨</span>
                <p className="text-sm text-gray-400">{t("rules.empty.title")}</p>
                <p className="text-xs text-gray-300">{t("rules.empty.hint")}</p>
              </div>
            ) : (
              <div className="space-y-2">
                <p className="mb-3 text-xs font-medium uppercase tracking-wide text-gray-400">
                  {targetKey === "global" ? t("rules.count", { n: rules.length }) : t("rules.device_count", { n: rules.length })}
                </p>
                {rules.map((rule, idx) => (
                  <RuleCard key={idx} rule={rule} onDelete={() => deleteRule(idx)} disabled={saving} />
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
  const targets = new Map<string, { key: string; label: string }>();
  for (const [key, deviceConfig] of Object.entries(config.layer_switch.devices ?? {})) {
    targets.set(key, { key, label: deviceConfig.display_name || key });
  }
  for (const device of appLayerDevices) {
    if (!device.device_uid_hash) continue;
    const label = device.product || device.serial_number || device.device_uid_hash;
    targets.set(device.device_uid_hash, { key: device.device_uid_hash, label });
  }
  return Array.from(targets.values()).sort((a, b) => a.label.localeCompare(b.label));
}
const AVATAR_COLORS = [
  "bg-blue-500","bg-violet-500","bg-emerald-500","bg-amber-500","bg-rose-500",
  "bg-cyan-500","bg-orange-500","bg-pink-500","bg-teal-500","bg-indigo-500",
];

function avatarColor(name: string) {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = ((h * 31) + name.charCodeAt(i)) | 0;
  return AVATAR_COLORS[Math.abs(h) % AVATAR_COLORS.length];
}

function AppAvatar({ name, selected = false, size = "sm" }: {
  name: string; selected?: boolean; size?: "sm" | "lg";
}) {
  return (
    <div className={`flex flex-shrink-0 items-center justify-center rounded-lg font-bold text-white ${
      size === "lg" ? "h-10 w-10 text-base" : "h-8 w-8 text-sm"
    } ${selected ? "opacity-90" : ""} ${avatarColor(name)}`}>
      {(name[0] ?? "?").toUpperCase()}
    </div>
  );
}

// ─── Rule Card ────────────────────────────────────────────────────────────────

function RuleCard({ rule, onDelete, disabled }: {
  rule: RuleConfig; onDelete: () => void; disabled: boolean;
}) {
  const matchDesc = rule.exe ?? rule.title ?? rule.path ?? "—";
  const matchKind = rule.exe ? "exe" : rule.title ? "title" : "path";

  return (
    <div className="group flex items-center gap-4 rounded-xl bg-white px-4 py-3 shadow-card ring-1 ring-border hover:shadow-card-hover transition-all">
      <LayerBadge layer={rule.layer} />
      <div className="min-w-0 flex-1">
        <div className="font-medium text-gray-800">{rule.name}</div>
        <div className="mt-0.5 flex items-center gap-1.5">
          <span className="rounded bg-background px-1.5 py-0.5 font-mono text-[10px] text-gray-400 ring-1 ring-border">
            {matchKind}
          </span>
          <span className="truncate font-mono text-[11px] text-gray-500">{matchDesc}</span>
        </div>
      </div>
      <button
        onClick={onDelete}
        disabled={disabled}
        className="flex-shrink-0 rounded-lg p-1.5 text-gray-300 opacity-0 transition-all group-hover:opacity-100 hover:bg-red-50 hover:text-red-400 disabled:opacity-30"
        title="削除"
      >
        <Trash2 size={14} />
      </button>
    </div>
  );
}
