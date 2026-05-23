import { useState } from "react";
import {
  Play,
  Square,
  Keyboard,
  List,
  Clock,
  AlertCircle,
  Info,
  AlertTriangle,
} from "lucide-react";
import { startMonitoring, stopMonitoring, saveConfig } from "../api";
import { Toggle } from "../components/Toggle";
import { useLang, type TranslationKey } from "../i18n";
import type { AppConfig, MonitorStatus, LogEntry } from "../types";

interface Props {
  config: AppConfig;
  setConfig: (c: AppConfig) => void;
  status: MonitorStatus;
  logs: LogEntry[];
}

function formatTime(ms: number) {
  const d = new Date(ms);
  return d.toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export default function Dashboard({ config, setConfig, status, logs }: Props) {
  const { t } = useLang();
  const [actionLoading, setActionLoading] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [featureSaving, setFeatureSaving] = useState(false);

  const toggleLayerSwitch = async (enabled: boolean) => {
    const updated = { ...config, layer_switch: { ...config.layer_switch, enabled } };
    setFeatureSaving(true);
    try { await saveConfig(updated); setConfig(updated); } finally { setFeatureSaving(false); }
  };

  const toggleTimeSync = async (enabled: boolean) => {
    const updated = { ...config, time: { ...config.time, enabled } };
    setFeatureSaving(true);
    try { await saveConfig(updated); setConfig(updated); } finally { setFeatureSaving(false); }
  };

  const handleToggle = async () => {
    setActionLoading(true);
    setActionError(null);
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

  return (
    <div className="p-6 max-w-4xl mx-auto space-y-5">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold text-gray-800">{t("dashboard.title")}</h1>
          <p className="text-sm text-gray-500 mt-0.5">{t("dashboard.subtitle")}</p>
        </div>
        <button
          onClick={handleToggle}
          disabled={actionLoading}
          className={`flex items-center gap-2 rounded-lg px-5 py-2.5 text-sm font-semibold text-white shadow-sm transition-all disabled:opacity-60 ${
            status.running
              ? "bg-rose-500 hover:bg-rose-600"
              : "bg-primary hover:bg-primary-dark"
          }`}
        >
          {actionLoading ? (
            <div className="h-4 w-4 animate-spin rounded-full border-2 border-white/30 border-t-white" />
          ) : status.running ? (
            <Square size={15} />
          ) : (
            <Play size={15} />
          )}
          {status.running ? t("dashboard.stop") : t("dashboard.start")}
        </button>
      </div>

      {actionError && (
        <div className="flex items-start gap-2.5 rounded-lg bg-red-50 px-4 py-3 text-sm text-red-700 ring-1 ring-red-200">
          <AlertCircle size={16} className="mt-0.5 flex-shrink-0" />
          <span>{actionError}</span>
        </div>
      )}

      {/* Status Cards */}
      <div className="grid grid-cols-2 gap-4">
        {/* Status */}
        <div className="rounded-xl bg-white px-5 py-4 shadow-card ring-1 ring-border">
          <div className="text-xs font-medium text-gray-400 uppercase tracking-wide">
            {t("dashboard.status.label")}
          </div>
          <div className="mt-3 flex items-center gap-2.5">
            <span
              className={`h-2.5 w-2.5 rounded-full flex-shrink-0 ${
                status.running
                  ? "bg-emerald-400 shadow-[0_0_6px_rgba(52,211,153,0.5)]"
                  : "bg-gray-300"
              }`}
            />
            <span
              className={`text-lg font-semibold ${
                status.running ? "text-emerald-600" : "text-gray-400"
              }`}
            >
              {status.running ? t("dashboard.status.running") : t("dashboard.status.stopped")}
            </span>
          </div>
        </div>

        {/* Devices */}
        <div className="rounded-xl bg-white px-5 py-4 shadow-card ring-1 ring-border">
          <div className="text-xs font-medium text-gray-400 uppercase tracking-wide">
            {t("dashboard.devices.label")}
          </div>
          <div className="mt-3 flex items-center gap-2">
            <Keyboard size={20} className="text-primary flex-shrink-0" />
            <span className="text-lg font-semibold text-gray-800">
              {status.connected_devices}
            </span>
            {t("dashboard.devices.unit") && (
              <span className="text-sm text-gray-400">{t("dashboard.devices.unit")}</span>
            )}
          </div>
          {status.connected_device_names.length > 0 && (
            <div className="mt-1.5 space-y-0.5">
              {status.connected_device_names.map((name, i) => (
                <div key={i} className="text-[11px] text-gray-400 truncate flex items-center gap-1">
                  <span className="inline-block h-1 w-1 rounded-full bg-gray-300 flex-shrink-0" />
                  {name}
                </div>
              ))}
            </div>
          )}
          {status.connected_devices === 0 && (
            <div className="mt-1 text-[11px] text-gray-300">{t("dashboard.devices.none")}</div>
          )}
        </div>
      </div>

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

        {/* レイヤー切替 */}
        <div className={`rounded-xl bg-white shadow-card ring-1 transition-all ${
          config.layer_switch.enabled ? "ring-primary/20" : "ring-border"
        }`}>
          <div className={`flex items-center justify-between rounded-t-xl px-5 py-3.5 ${
            config.layer_switch.enabled ? "bg-primary/5" : "bg-gray-50"
          }`}>
            <div className="flex items-center gap-2">
              <div className={`flex h-7 w-7 items-center justify-center rounded-lg ${
                config.layer_switch.enabled ? "bg-primary text-white" : "bg-gray-200 text-gray-400"
              }`}>
                <List size={14} />
              </div>
              <span className="text-sm font-semibold text-gray-800">{t("dashboard.layer_switch")}</span>
            </div>
            <Toggle
              checked={config.layer_switch.enabled}
              onChange={toggleLayerSwitch}
              disabled={featureSaving}
            />
          </div>

          <div className="px-5 py-4 space-y-2.5">
            {config.layer_switch.enabled ? (
              <>
                <FeatureRow
                  label={t("dashboard.feature.rules_count")}
                  value={
                    config.layer_switch.rules.length > 0
                      ? `${config.layer_switch.rules.length}`
                      : t("dashboard.feature.rules_unset")
                  }
                  warn={config.layer_switch.rules.length === 0}
                />
                <FeatureRow
                  label={t("dashboard.feature.polling")}
                  value={`${config.polling.interval_ms} ms`}
                />
                {config.layer_switch.rules.length > 0 && (
                  <div className="pt-1 space-y-1">
                    {config.layer_switch.rules.slice(0, 3).map((r, i) => (
                      <div key={i} className="flex items-center gap-2 text-xs text-gray-500">
                        <span className="inline-flex h-4 w-5 items-center justify-center rounded bg-primary/10 font-mono text-[10px] font-semibold text-primary">
                          L{r.layer}
                        </span>
                        <span className="truncate">{r.name}</span>
                      </div>
                    ))}
                    {config.layer_switch.rules.length > 3 && (
                      <p className="text-[11px] text-gray-400">
                        {t("dashboard.feature.rules_others", { n: config.layer_switch.rules.length - 3 })}
                      </p>
                    )}
                  </div>
                )}
              </>
            ) : (
              <p className="text-sm text-gray-400">
                {t("dashboard.layer_switch.disabled")}
              </p>
            )}
          </div>
        </div>

        {/* 時刻同期 */}
        <div className={`rounded-xl bg-white shadow-card ring-1 transition-all ${
          config.time.enabled ? "ring-primary/20" : "ring-border"
        }`}>
          <div className={`flex items-center justify-between rounded-t-xl px-5 py-3.5 ${
            config.time.enabled ? "bg-primary/5" : "bg-gray-50"
          }`}>
            <div className="flex items-center gap-2">
              <div className={`flex h-7 w-7 items-center justify-center rounded-lg ${
                config.time.enabled ? "bg-primary text-white" : "bg-gray-200 text-gray-400"
              }`}>
                <Clock size={14} />
              </div>
              <span className="text-sm font-semibold text-gray-800">{t("dashboard.timesync")}</span>
            </div>
            <Toggle
              checked={config.time.enabled}
              onChange={toggleTimeSync}
              disabled={featureSaving}
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
                />
                <FeatureRow
                  label={t("dashboard.feature.timezone")}
                  value={tzLabel(config.time.tz_offset_min, t)}
                />
              </>
            ) : (
              <p className="text-sm text-gray-400">
                {t("dashboard.timesync.disabled")}
              </p>
            )}
          </div>
        </div>

      </div>

      {/* Activity Log */}
      <div className="rounded-xl bg-white shadow-card ring-1 ring-border">
        <div className="flex items-center justify-between border-b border-border px-5 py-3.5">
          <h2 className="text-sm font-semibold text-gray-700">{t("dashboard.log.title")}</h2>
          <span className="text-xs text-gray-400">
            {t("dashboard.log.count", { n: logs.length })}
          </span>
        </div>
        <div className="max-h-60 overflow-y-auto">
          {reversedLogs.length === 0 ? (
            <div className="px-5 py-8 text-center text-sm text-gray-400">
              {t("dashboard.log.empty")}
            </div>
          ) : (
            <ul className="divide-y divide-border/50">
              {reversedLogs.slice(0, 50).map((entry) => (
                <li
                  key={entry.id}
                  className="flex items-start gap-3 px-5 py-2.5 hover:bg-background/50"
                >
                  <LogIcon level={entry.level} />
                  <div className="min-w-0 flex-1">
                    <span className="text-sm text-gray-700">{entry.message}</span>
                  </div>
                  <span className="flex-shrink-0 text-[11px] text-gray-400 font-mono">
                    {formatTime(entry.timestamp_ms)}
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

function FeatureRow({ label, value, warn = false }: { label: string; value: string; warn?: boolean }) {
  return (
    <div className="flex items-center justify-between text-sm">
      <span className="text-gray-400">{label}</span>
      <span className={`font-medium ${warn ? "text-amber-600" : "text-gray-700"}`}>{value}</span>
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
  return <Info size={14} className="mt-0.5 flex-shrink-0 text-primary/60" />;
}
