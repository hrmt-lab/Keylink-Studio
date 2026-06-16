import { useMemo, useState } from "react";
import {
  Play,
  Square,
  List,
  Clock,
  Activity,
  AlertCircle,
  Info,
  AlertTriangle,
} from "lucide-react";
import { startMonitoring, stopMonitoring, saveConfig } from "../api";
import { Toggle } from "../components/Toggle";
import { RollingNumber } from "../components/RollingNumber";
import { ErrorNotice, SavedIndicator } from "../components/Ui";
import { aiStatusKey, formatClockTime, formatUsedBp, usageBarColor } from "../lib/format";
import { useLang, type TranslationKey } from "../i18n";
import type {
  AppConfig,
  MonitorStatus,
  LogEntry,
  AiUsageProviderStatus,
  AiUsageStatusKind,
} from "../types";

interface Props {
  config: AppConfig;
  setConfig: (c: AppConfig) => void;
  status: MonitorStatus;
  logs: LogEntry[];
}

/** Case-insensitive serial match between a connected device and battery data. */
function serialsMatch(a: string | null, b: string | null): boolean {
  if (!a || !b) return false;
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

function compareDeviceName(a: string, b: string): number {
  return a.localeCompare(b, undefined, { sensitivity: "base", numeric: true });
}

export default function Dashboard({ config, setConfig, status, logs }: Props) {
  const { t, lang } = useLang();
  const [actionLoading, setActionLoading] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [featureSaving, setFeatureSaving] = useState(false);
  const [featureError, setFeatureError] = useState<string | null>(null);
  const [justSaved, setJustSaved] = useState(false);

  const saveFeatureConfig = async (updated: AppConfig) => {
    const previous = config;
    setActionError(null);
    setFeatureError(null);
    setJustSaved(false);
    setConfig(updated);
    setFeatureSaving(true);
    try {
      await saveConfig(updated);
      setJustSaved(true);
      setTimeout(() => setJustSaved(false), 2000);
    } catch {
      setConfig(previous);
      setFeatureError(t("dashboard.save_failed"));
    } finally {
      setFeatureSaving(false);
    }
  };

  const toggleLayerSwitch = async (enabled: boolean) => {
    await saveFeatureConfig({ ...config, layer_switch: { ...config.layer_switch, enabled } });
  };

  const toggleTimeSync = async (enabled: boolean) => {
    await saveFeatureConfig({ ...config, time: { ...config.time, enabled } });
  };

  const toggleAiUsage = async (enabled: boolean) => {
    await saveFeatureConfig({ ...config, ai_usage: { ...config.ai_usage, enabled } });
  };

  const toggleAiUsageProvider = async (provider: "codex" | "claude_code", enabled: boolean) => {
    await saveFeatureConfig({
      ...config,
      ai_usage: {
        ...config.ai_usage,
        [provider]: { ...config.ai_usage[provider], enabled },
      },
    });
  };

  const handleToggle = async () => {
    setActionLoading(true);
    setActionError(null);
    setFeatureError(null);
    try {
      if (status.running) {
        await stopMonitoring();
      } else {
        await startMonitoring();
      }
    } catch (e) {
      setActionError(String(e));
    } finally {
      setActionLoading(false);
    }
  };

  const reversedLogs = [...logs].reverse();
  // Layer rules are per-device only; summarize across all device configs.
  const allDeviceRules = Object.values(config.layer_switch.devices ?? {}).flatMap(
    (device) => device.rules
  );
  const connectedDevices = useMemo(
    () =>
      status.connected_device_names
        .map((name, index) => ({
          name,
          device: status.host_link_devices[index] ?? null,
        }))
        .sort((a, b) => compareDeviceName(a.name, b.name)),
    [status.connected_device_names, status.host_link_devices]
  );

  return (
    <div className="p-6 max-w-4xl mx-auto space-y-5">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-medium text-ink">{t("dashboard.title")}</h1>
          <p className="text-sm text-muted mt-0.5">{t("dashboard.subtitle")}</p>
        </div>
        <div className="flex items-center gap-3">
          {justSaved && <SavedIndicator label={t("common.saved")} />}
          <button
          onClick={handleToggle}
          disabled={actionLoading}
          className="btn-neu flex items-center gap-2 rounded-full px-5 py-2.5 text-sm font-medium text-ink disabled:opacity-60"
        >
          {actionLoading ? (
            <div className="h-4 w-4 animate-spin rounded-full border-2 border-border border-t-accent" />
          ) : status.running ? (
            <Square size={15} className="text-accent" />
          ) : (
            <Play size={15} />
          )}
          {status.running ? t("dashboard.stop") : t("dashboard.start")}
          </button>
        </div>
      </div>

      {actionError && <ErrorNotice message={t("dashboard.error.title")} details={actionError} />}
      {featureError && <ErrorNotice message={featureError} />}

      {/* Error Banner */}
      {status.last_error && (
        <div className="flex items-start gap-2.5 rounded-lg bg-red-50 px-4 py-3 text-sm text-red-700 ring-1 ring-red-200">
          <AlertCircle size={16} className="mt-0.5 flex-shrink-0" />
          <div>
            <div className="font-medium">{t("dashboard.error.title")}</div>
            <div className="mt-0.5 text-red-600">{status.last_error}</div>
          </div>
        </div>
      )}

      {/* Feature Summary Cards */}
      <div className="grid grid-cols-2 gap-4">

        {/* Status */}
        <div className="overflow-hidden rounded-card bg-surface">
          <div className="flex items-center justify-between border-b border-background px-5 py-3.5">
            <div className="flex items-center gap-2">
              <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-plate text-ink">
                <Activity size={14} />
              </div>
              <span className="text-sm font-medium text-ink">{t("dashboard.status.label")}</span>
            </div>
            <span
              className={`rounded-full px-2.5 py-1 text-xs font-medium ${
                status.running
                  ? "bg-accent-soft text-accent-deep"
                  : "bg-plate text-muted"
              }`}
            >
              {status.running ? t("dashboard.status.running") : t("dashboard.status.stopped")}
            </span>
          </div>

          <div className="px-5 py-4 space-y-3">
            <FeatureRow
              label={t("dashboard.devices.label")}
              value={
                t("dashboard.devices.unit")
                  ? status.connected_devices + " " + t("dashboard.devices.unit")
                  : String(status.connected_devices)
              }
              mono
            />

            {connectedDevices.length > 0 ? (
              <div className="space-y-1 border-t border-background pt-2.5">
                {connectedDevices.slice(0, 2).map(({ name, device }) => {
                  const battery = device
                    ? status.device_battery.find(
                        (b) =>
                          serialsMatch(b.serial_number, device.serial_number) ||
                          (b.product !== null && b.product === device.product)
                      ) ?? null
                    : null;
                  return (
                    <div key={device?.device_uid_hash ?? device?.path ?? name} className="flex items-center gap-1.5 text-[11px] text-muted">
                      <span className="h-1.5 w-1.5 flex-shrink-0 rounded-full bg-accent" />
                      <span className="min-w-0 truncate">{name}</span>
                      {battery && (
                        <span className="ml-auto flex flex-shrink-0 items-center gap-1.5">
                          {battery.sources.map((source) => (
                            <span key={source.source} className="font-mono">
                              {t(`battery.source.${source.source}` as Parameters<typeof t>[0])}{" "}
                              <RollingNumber
                                value={source.level}
                                format={(v) => `${Math.round(v)}%`}
                              />
                            </span>
                          ))}
                        </span>
                      )}
                    </div>
                  );
                })}
                {connectedDevices.length > 2 && (
                  <div className="text-[11px] text-disabled">
                    {t("dashboard.feature.rules_others", { n: connectedDevices.length - 2 })}
                  </div>
                )}
              </div>
            ) : (
              <div className="border-t border-background pt-2.5 text-[11px] text-disabled">
                {t("dashboard.devices.none")}
              </div>
            )}
          </div>
        </div>

        {/* レイヤー切替 */}
        <div className="rounded-card bg-surface">
          <div className="flex items-center justify-between border-b border-background px-5 py-3.5">
            <div className="flex items-center gap-2">
              <div className={`flex h-7 w-7 items-center justify-center rounded-lg bg-plate ${
                config.layer_switch.enabled ? "text-ink" : "text-disabled"
              }`}>
                <List size={14} />
              </div>
              <span className="text-sm font-medium text-ink">{t("dashboard.layer_switch")}</span>
            </div>
            <Toggle
              checked={config.layer_switch.enabled}
              onChange={toggleLayerSwitch}
              disabled={featureSaving}
              label={t("dashboard.layer_switch")}
            />
          </div>

          <div className="px-5 py-4 space-y-2.5">
            {config.layer_switch.enabled ? (
              <>
                <FeatureRow
                  label={t("dashboard.feature.rules_count")}
                  value={
                    allDeviceRules.length > 0
                      ? `${allDeviceRules.length}`
                      : t("dashboard.feature.rules_unset")
                  }
                  warn={allDeviceRules.length === 0}
                  mono={allDeviceRules.length > 0}
                />
                <FeatureRow
                  label={t("dashboard.feature.polling")}
                  value={`${config.polling.interval_ms} ms`}
                  mono
                />
                {allDeviceRules.length > 0 && (
                  <div className="pt-1 space-y-1">
                    {allDeviceRules.slice(0, 3).map((r, i) => (
                      <div key={i} className="flex items-center gap-2 text-xs text-muted">
                        <span className="inline-flex h-4 w-5 items-center justify-center rounded bg-accent-soft font-mono text-[10px] font-medium text-accent">
                          L{r.layer}
                        </span>
                        <span className="truncate">{r.name}</span>
                      </div>
                    ))}
                    {allDeviceRules.length > 3 && (
                      <p className="text-[11px] text-disabled">
                        {t("dashboard.feature.rules_others", { n: allDeviceRules.length - 3 })}
                      </p>
                    )}
                  </div>
                )}
              </>
            ) : (
              <p className="text-sm text-faint">
                {t("dashboard.layer_switch.disabled")}
              </p>
            )}
          </div>
        </div>

        {/* 時刻同期 */}
        <div className="rounded-card bg-surface">
          <div className="flex items-center justify-between border-b border-background px-5 py-3.5">
            <div className="flex items-center gap-2">
              <div className={`flex h-7 w-7 items-center justify-center rounded-lg bg-plate ${
                config.time.enabled ? "text-ink" : "text-disabled"
              }`}>
                <Clock size={14} />
              </div>
              <span className="text-sm font-medium text-ink">{t("dashboard.timesync")}</span>
            </div>
            <Toggle
              checked={config.time.enabled}
              onChange={toggleTimeSync}
              disabled={featureSaving}
              label={t("dashboard.timesync")}
            />
          </div>

          <div className="px-5 py-4 space-y-2.5">
            {config.time.enabled ? (
              <>
                <FeatureRow
                  label={t("dashboard.feature.format")}
                  value={formatHintLabel(config.time.format_hint, t)}
                />
                <FeatureRow
                  label={t("dashboard.feature.clock_mode")}
                  value={config.time.clock_mode === "24h"
                    ? t("dashboard.feature.clock_24h")
                    : t("dashboard.feature.clock_12h")}
                />
                <FeatureRow
                  label={t("dashboard.feature.periodic_sync")}
                  value={config.time.periodic_sync_sec === 0
                    ? t("dashboard.feature.periodic_sync.change")
                    : t("dashboard.feature.periodic_sync.seconds", { n: config.time.periodic_sync_sec })}
                  mono={config.time.periodic_sync_sec !== 0}
                />
                <FeatureRow
                  label={t("dashboard.feature.timezone")}
                  value={tzLabel(config.time.tz_offset_min, t)}
                  mono={config.time.tz_offset_min !== null}
                />
              </>
            ) : (
              <p className="text-sm text-faint">
                {t("dashboard.timesync.disabled")}
              </p>
            )}
          </div>
        </div>

        {/* AI Usage */}
        <div className="rounded-card bg-surface">
          <div className="flex items-center justify-between border-b border-background px-5 py-3.5">
            <div className="flex items-center gap-2">
              <div className={`flex h-7 w-7 items-center justify-center rounded-lg bg-plate ${
                config.ai_usage.enabled ? "text-ink" : "text-disabled"
              }`}>
                <Activity size={14} />
              </div>
              <span className="text-sm font-medium text-ink">{t("dashboard.ai_usage")}</span>
            </div>
            <Toggle
              checked={config.ai_usage.enabled}
              onChange={toggleAiUsage}
              disabled={featureSaving}
              label={t("dashboard.ai_usage")}
            />
          </div>

          <div className="px-5 py-4 space-y-2.5">
            {config.ai_usage.enabled ? (
              <>
                <div className="grid grid-cols-2 gap-2">
                  <ProviderToggle
                    label="Codex"
                    checked={config.ai_usage.codex.enabled}
                    disabled={featureSaving}
                    onChange={(enabled) => toggleAiUsageProvider("codex", enabled)}
                  />
                  <ProviderToggle
                    label="Claude Code"
                    checked={config.ai_usage.claude_code.enabled}
                    disabled={featureSaving}
                    onChange={(enabled) => toggleAiUsageProvider("claude_code", enabled)}
                  />
                </div>
                {status.ai_usage.length === 0 ? (
                  <p className="text-sm text-faint">{t("dashboard.ai_usage.waiting")}</p>
                ) : (
                  <div className="space-y-2 pt-1">
                    {status.ai_usage.map((provider) => (
                      <AiUsageSummary key={provider.provider} provider={provider} />
                    ))}
                  </div>
                )}
              </>
            ) : (
              <p className="text-sm text-faint">
                {t("dashboard.ai_usage.disabled")}
              </p>
            )}
          </div>
        </div>

      </div>

      {/* Activity Log */}
      <div className="rounded-card bg-surface">
        <div className="flex items-center justify-between border-b border-background px-5 py-3.5">
          <h2 className="text-sm font-medium text-ink">{t("dashboard.log.title")}</h2>
          <span className="text-xs text-faint font-mono">
            {t("dashboard.log.count", { n: logs.length })}
          </span>
        </div>
        <div className="max-h-60 overflow-y-auto">
          {reversedLogs.length === 0 ? (
            <div className="px-5 py-8 text-center text-sm text-faint">
              {t("dashboard.log.empty")}
            </div>
          ) : (
            <ul className="divide-y divide-background">
              {reversedLogs.slice(0, 50).map((entry) => (
                <li
                  key={entry.id}
                  className="flex items-start gap-3 px-5 py-2.5"
                >
                  <LogIcon level={entry.level} />
                  <div className="min-w-0 flex-1">
                    <span className="text-sm text-ink">{entry.message}</span>
                  </div>
                  <span className="flex-shrink-0 text-[11px] text-faint font-mono">
                    {formatClockTime(entry.timestamp_ms, lang)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

function AiUsageSummary({ provider }: { provider: AiUsageProviderStatus }) {
  const { t } = useLang();
  const color = provider.status === "ok"
    ? "text-accent-deep"
    : provider.status === "stale"
      ? "text-amber-600"
      : "text-muted";
  return (
    <div className="rounded-lg bg-plate px-3 py-2 text-xs">
      <div className="mb-1.5 flex items-center justify-between gap-3">
        <div className="font-medium text-ink">{providerLabel(provider.provider)}</div>
        <div className={`font-medium ${color}`}>{aiStatusLabel(provider.status, t)}</div>
      </div>
      <div className="grid grid-cols-2 gap-2">
        <MiniUsage label={t("ai_usage.window.5h")} valid={provider.five_hour_valid} bp={provider.five_hour_used_bp} />
        <MiniUsage label={t("ai_usage.window.7d")} valid={provider.seven_day_valid} bp={provider.seven_day_used_bp} />
      </div>
    </div>
  );
}

function MiniUsage({ label, valid, bp }: { label: string; valid: boolean; bp: number | null }) {
  return (
    <div>
      <div className="flex items-center justify-between text-[11px] text-muted">
        <span>{label}</span>
        <span className="font-mono">
          {valid && bp !== null ? (
            <RollingNumber value={bp} format={formatUsedBp} />
          ) : (
            "--"
          )}
        </span>
      </div>
      <div className="mt-1 h-1.5 rounded-full bg-plate shadow-neu-groove">
        <div
          className={`gauge-fill h-1.5 rounded-full ${usageBarColor(bp ?? 0, valid)}`}
          style={{ width: valid && bp !== null ? `${Math.min(bp / 100, 100)}%` : "0%" }}
        />
      </div>
    </div>
  );
}

function ProviderToggle({
  label,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onChange: (enabled: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between rounded-lg bg-plate px-3 py-2">
      <span className="text-xs font-medium text-ink">{label}</span>
      <Toggle checked={checked} onChange={onChange} disabled={disabled} label={label} />
    </div>
  );
}

function providerLabel(provider: string) {
  if (provider === "claude_code") return "Claude Code";
  if (provider === "codex") return "Codex";
  return provider;
}

function aiStatusLabel(status: AiUsageStatusKind, t: TFn) {
  return t(aiStatusKey(status));
}

function FeatureRow({ label, value, warn = false, mono = false }: {
  label: string;
  value: string;
  warn?: boolean;
  mono?: boolean;
}) {
  return (
    <div className="flex items-center justify-between text-sm">
      <span className="text-muted">{label}</span>
      <span
        className={`font-medium ${warn ? "text-amber-600" : "text-ink"} ${mono ? "font-mono" : ""}`}
      >
        {value}
      </span>
    </div>
  );
}

type TFn = (key: TranslationKey, params?: Record<string, string | number>) => string;

function formatHintLabel(hint: string, t: TFn): string {
  const key = `format.${hint}` as TranslationKey;
  return t(key);
}

function tzLabel(offsetMin: number | null, t: TFn): string {
  if (offsetMin === null) return t("dashboard.tz.auto");
  const sign = offsetMin >= 0 ? "+" : "-";
  const abs = Math.abs(offsetMin);
  const h = Math.floor(abs / 60);
  const m = abs % 60;
  return m === 0 ? `UTC${sign}${h}` : `UTC${sign}${h}:${String(m).padStart(2, "0")}`;
}

function LogIcon({ level }: { level: string }) {
  if (level === "error") return <AlertCircle size={14} className="mt-0.5 flex-shrink-0 text-red-400" />;
  if (level === "warn")  return <AlertTriangle size={14} className="mt-0.5 flex-shrink-0 text-amber-400" />;
  return <Info size={14} className="mt-0.5 flex-shrink-0 text-faint" />;
}
