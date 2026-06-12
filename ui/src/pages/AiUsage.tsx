import { useEffect, useState } from "react";
import {
  AlertCircle,
  ChevronRight,
  RefreshCcw,
  RotateCcw,
  Save,
} from "lucide-react";
import claudeCodeIcon from "../assets/claude_code_icon_transparent.png";
import codexIcon from "../assets/codex_icon_transparent.png";
import { refreshAiUsage } from "../api";
import { Toggle } from "../components/Toggle";
import {
  ErrorNotice,
  Notice,
  PageHeader,
  PrimaryButton,
  SecondaryButton,
  SettingRow,
} from "../components/Ui";
import { useConfigSection } from "../hooks/useConfigSection";
import { aiStatusKey, formatUnixShort, formatUsedBp, usageBarColor, usageTextColor } from "../lib/format";
import { useLang, type TranslationKey } from "../i18n";
import type { AiUsageProviderStatus, AiUsageStatusKind, AppConfig, MonitorStatus } from "../types";

interface Props {
  config: AppConfig;
  setConfig: (c: AppConfig) => void;
  status: MonitorStatus;
}

type ProviderName = "codex" | "claude_code";

export default function AiUsage({ config, setConfig, status }: Props) {
  const { t } = useLang();
  const {
    draft,
    setDraft,
    isDirty,
    saving,
    error,
    setError,
    save,
  } = useConfigSection({
    config,
    setConfig,
    select: (c) => c,
    apply: (_c, d) => d,
  });
  const [refreshing, setRefreshing] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  // The backend pushes a status-update when the worker finishes (even while
  // monitoring is stopped), so the metrics refresh on their own. We only show a
  // transient acknowledgement here.
  useEffect(() => {
    if (!message) return;
    const timer = setTimeout(() => setMessage(null), 4000);
    return () => clearTimeout(timer);
  }, [message]);

  const updateAiUsage = (ai_usage: AppConfig["ai_usage"]) =>
    setDraft((prev) => ({ ...prev, ai_usage }));

  const handleSave = async () => {
    setMessage(null);
    await save();
  };

  const handleRefresh = async () => {
    setRefreshing(true);
    setError(null);
    setMessage(null);
    try {
      await refreshAiUsage();
      setMessage(t("ai_usage.refresh.done"));
    } catch (e) {
      const code = String(e);
      setError(
        code === "not_running"
          ? t("ai_usage.refresh.not_running")
          : code === "refresh_in_progress"
            ? t("ai_usage.refresh.in_progress")
            : t("ai_usage.refresh.failed")
      );
    } finally {
      setRefreshing(false);
    }
  };

  const statusFor = (provider: ProviderName) =>
    status.ai_usage.find((item) => item.provider === provider) ?? null;

  return (
    <div className="mx-auto max-w-2xl space-y-5 p-6">
      <PageHeader
        title={t("ai_usage.title")}
        description={t("ai_usage.subtitle")}
        actions={
          <>
            <SecondaryButton
              onClick={handleRefresh}
              loading={refreshing}
              icon={<RefreshCcw size={15} />}
            >
              {refreshing ? t("ai_usage.refreshing") : t("ai_usage.refresh")}
            </SecondaryButton>
            <PrimaryButton
              onClick={handleSave}
              disabled={!isDirty}
              loading={saving}
              icon={<Save size={15} />}
            >
              {t("ai_usage.save")}
            </PrimaryButton>
          </>
        }
      />

      {message && <Notice tone="info">{message}</Notice>}
      {error && <ErrorNotice message={error} />}

      <BasicSettings
        draft={draft}
        updateAiUsage={updateAiUsage}
      />

      <ProviderCard
        provider="codex"
        status={statusFor("codex")}
        draft={draft}
        updateAiUsage={updateAiUsage}
      />
      <ProviderCard
        provider="claude_code"
        status={statusFor("claude_code")}
        draft={draft}
        updateAiUsage={updateAiUsage}
      />
    </div>
  );
}

function BasicSettings({
  draft,
  updateAiUsage,
}: {
  draft: AppConfig;
  updateAiUsage: (ai_usage: AppConfig["ai_usage"]) => void;
}) {
  const { t } = useLang();
  return (
    <div className="overflow-hidden rounded-xl bg-white shadow-card ring-1 ring-border">
      <div className="divide-y divide-border/60">
        <SettingRow label={t("ai_usage.enabled")} description={t("ai_usage.enabled.desc")}>
          <Toggle
            checked={draft.ai_usage.enabled}
            onChange={(enabled) => updateAiUsage({ ...draft.ai_usage, enabled })}
            label={t("ai_usage.enabled")}
          />
        </SettingRow>
        <NumberRow
          label={t("ai_usage.poll_interval")}
          description={t("ai_usage.poll_interval.desc")}
          value={draft.ai_usage.poll_interval_sec}
          min={1}
          unit="sec"
          onChange={(poll_interval_sec) =>
            updateAiUsage({ ...draft.ai_usage, poll_interval_sec })
          }
        />
        <NumberRow
          label={t("ai_usage.stale_after")}
          description={t("ai_usage.stale_after.desc")}
          value={draft.ai_usage.stale_after_sec}
          min={1}
          unit="sec"
          onChange={(stale_after_sec) => updateAiUsage({ ...draft.ai_usage, stale_after_sec })}
        />
      </div>
    </div>
  );
}

function ProviderCard({
  provider,
  status,
  draft,
  updateAiUsage,
}: {
  provider: ProviderName;
  status: AiUsageProviderStatus | null;
  draft: AppConfig;
  updateAiUsage: (ai_usage: AppConfig["ai_usage"]) => void;
}) {
  const { t } = useLang();
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const isCodex = provider === "codex";
  const name = isCodex ? "Codex" : "Claude Code";
  const accent = isCodex ? "primary" : "amber";
  const providerConfig = isCodex ? draft.ai_usage.codex : draft.ai_usage.claude_code;

  return (
    <div
      className={`overflow-hidden rounded-xl bg-white shadow-card ring-1 ${
        isCodex ? "ring-primary/20" : "ring-amber-500/20"
      }`}
    >
      <div
        className={`flex items-center justify-between px-5 py-3.5 ${
          isCodex ? "border-b border-primary/10 bg-primary/5" : "border-b border-amber-500/10 bg-amber-500/5"
        }`}
      >
        <div className="flex items-center gap-2.5">
          <ProviderIcon provider={provider} />
          <span className="text-sm font-semibold text-gray-800">{name}</span>
          <StatusBadge status={status?.status ?? "no_data"} />
        </div>
        <div className="flex items-center gap-2.5">
          <span className="text-xs text-gray-400">
            {t("ai_usage.updated.short", { time: formatUnixShort(status?.updated_unix ?? null) })}
          </span>
          <Toggle
            checked={providerConfig.enabled}
            label={name}
            onChange={(enabled) =>
              updateAiUsage(
                isCodex
                  ? {
                      ...draft.ai_usage,
                      codex: { ...draft.ai_usage.codex, enabled },
                    }
                  : {
                      ...draft.ai_usage,
                      claude_code: { ...draft.ai_usage.claude_code, enabled },
                    }
              )
            }
          />
        </div>
      </div>

      <div className="px-5 pb-1 pt-3">
        <p className="text-[11px] text-gray-400">
          {isCodex ? t("ai_usage.codex.note.short") : t("ai_usage.claude.note.short")}
        </p>
      </div>

      <div className="space-y-3.5 px-5 py-3">
        <UsageMetric
          label={t("ai_usage.window.5h.used")}
          status={status}
          window="five_hour"
          accent={accent}
        />
        <UsageMetric
          label={t("ai_usage.window.7d.used")}
          status={status}
          window="seven_day"
          accent={accent}
        />
        {status?.error_present && (
          <div className="flex items-start gap-2 rounded-lg bg-amber-50 px-3 py-2.5 text-[11px] text-amber-800 ring-1 ring-amber-200">
            <AlertCircle size={12} className="mt-0.5 flex-shrink-0 text-amber-600" />
            <span>
              {t("ai_usage.error.fixed", {
                code: errorCodeLabel(status.last_error_code, t),
              })}
            </span>
          </div>
        )}
        {status?.estimated && (
          <div className="rounded-lg bg-amber-50 px-3 py-2 text-[11px] text-amber-700 ring-1 ring-amber-100">
            {t("ai_usage.estimate.note")}
          </div>
        )}
      </div>

      <div className="border-t border-panel">
        <button
          className="flex w-full items-center gap-2 px-5 py-3 text-xs font-medium text-gray-500 hover:bg-gray-50"
          onClick={() => setAdvancedOpen((open) => !open)}
        >
          <ChevronRight
            size={13}
            className={`transition-transform ${advancedOpen ? "rotate-90" : ""}`}
          />
          {t("ai_usage.advanced")}
        </button>
        {advancedOpen && (
          <div className="divide-y divide-border/60 border-t border-panel">
            {isCodex ? (
              <CodexAdvanced draft={draft} updateAiUsage={updateAiUsage} />
            ) : (
              <ClaudeAdvanced draft={draft} updateAiUsage={updateAiUsage} />
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function CodexAdvanced({
  draft,
  updateAiUsage,
}: {
  draft: AppConfig;
  updateAiUsage: (ai_usage: AppConfig["ai_usage"]) => void;
}) {
  const { t } = useLang();
  return (
    <>
      <PathRow
        label={t("ai_usage.codex.sessions_dir")}
        description={t("ai_usage.path.default.desc")}
        example="C:\\Users\\<user>\\.codex\\sessions"
        value={draft.ai_usage.codex.sessions_dir}
        onChange={(sessions_dir) =>
          updateAiUsage({
            ...draft.ai_usage,
            codex: { ...draft.ai_usage.codex, sessions_dir },
          })
        }
        onReset={() =>
          updateAiUsage({
            ...draft.ai_usage,
            codex: { ...draft.ai_usage.codex, sessions_dir: null },
          })
        }
      />
      <SettingRow
        compact
        label={t("ai_usage.codex.auto_detect")}
        description={t("ai_usage.codex.auto_detect.desc")}
      >
        <Toggle
          checked={draft.ai_usage.codex.sessions_auto_detect}
          onChange={(sessions_auto_detect) =>
            updateAiUsage({
              ...draft.ai_usage,
              codex: { ...draft.ai_usage.codex, sessions_auto_detect },
            })
          }
        />
      </SettingRow>
      <SettingRow
        compact
        label={t("ai_usage.codex.include_wsl")}
        description={t("ai_usage.codex.include_wsl.desc")}
      >
        <Toggle
          checked={draft.ai_usage.codex.include_wsl_sessions}
          onChange={(include_wsl_sessions) =>
            updateAiUsage({
              ...draft.ai_usage,
              codex: { ...draft.ai_usage.codex, include_wsl_sessions },
            })
          }
        />
      </SettingRow>
      <TextAreaRow
        label={t("ai_usage.codex.extra_paths")}
        description={t("ai_usage.codex.extra_paths.desc")}
        value={draft.ai_usage.codex.extra_sessions_paths.join("\n")}
        placeholder="\\\\wsl.localhost\\Ubuntu\\home\\<user>\\.codex\\sessions"
        onChange={(value) =>
          updateAiUsage({
            ...draft.ai_usage,
            codex: {
              ...draft.ai_usage.codex,
              extra_sessions_paths: value
                .split("\n")
                .map((line) => line.trim())
                .filter(Boolean),
            },
          })
        }
      />
      <SettingRow
        compact
        label={t("ai_usage.codex.history_fallback")}
        description={t("ai_usage.codex.history_fallback.desc")}
      >
        <Toggle
          checked={draft.ai_usage.codex.history_fallback_enabled}
          onChange={(history_fallback_enabled) =>
            updateAiUsage({
              ...draft.ai_usage,
              codex: { ...draft.ai_usage.codex, history_fallback_enabled },
            })
          }
        />
      </SettingRow>
      <SettingRow
        compact
        label={t("ai_usage.codex.allow_baseline")}
        description={t("ai_usage.codex.baseline.desc")}
      >
        <Toggle
          checked={draft.ai_usage.codex.allow_activity_baseline}
          onChange={(allow_activity_baseline) =>
            updateAiUsage({
              ...draft.ai_usage,
              codex: { ...draft.ai_usage.codex, allow_activity_baseline },
            })
          }
        />
      </SettingRow>
      <NumberRow
        compact
        label={t("ai_usage.codex.baseline_5h")}
        description={t("ai_usage.codex.baseline.desc")}
        value={draft.ai_usage.codex.activity_five_hour_token_baseline}
        min={0}
        unit="tokens"
        onChange={(activity_five_hour_token_baseline) =>
          updateAiUsage({
            ...draft.ai_usage,
            codex: { ...draft.ai_usage.codex, activity_five_hour_token_baseline },
          })
        }
      />
      <NumberRow
        compact
        label={t("ai_usage.codex.baseline_7d")}
        description={t("ai_usage.codex.baseline.desc")}
        value={draft.ai_usage.codex.activity_seven_day_token_baseline}
        min={0}
        unit="tokens"
        onChange={(activity_seven_day_token_baseline) =>
          updateAiUsage({
            ...draft.ai_usage,
            codex: { ...draft.ai_usage.codex, activity_seven_day_token_baseline },
          })
        }
      />
    </>
  );
}

function ClaudeAdvanced({
  draft,
  updateAiUsage,
}: {
  draft: AppConfig;
  updateAiUsage: (ai_usage: AppConfig["ai_usage"]) => void;
}) {
  const { t } = useLang();
  return (
    <>
      <PathRow
        label={t("ai_usage.claude.credentials_path")}
        description={t("ai_usage.path.default.desc")}
        example="C:\\Users\\<user>\\.claude\\.credentials.json"
        value={draft.ai_usage.claude_code.credentials_path}
        onChange={(credentials_path) =>
          updateAiUsage({
            ...draft.ai_usage,
            claude_code: { ...draft.ai_usage.claude_code, credentials_path },
          })
        }
        onReset={() =>
          updateAiUsage({
            ...draft.ai_usage,
            claude_code: { ...draft.ai_usage.claude_code, credentials_path: null },
          })
        }
      />
      <SettingRow
        compact
        label={t("ai_usage.claude.auto_detect")}
        description={t("ai_usage.claude.auto_detect.desc")}
      >
        <Toggle
          checked={draft.ai_usage.claude_code.credentials_auto_detect}
          onChange={(credentials_auto_detect) =>
            updateAiUsage({
              ...draft.ai_usage,
              claude_code: { ...draft.ai_usage.claude_code, credentials_auto_detect },
            })
          }
        />
      </SettingRow>
      <SettingRow
        compact
        label={t("ai_usage.claude.include_wsl")}
        description={t("ai_usage.claude.include_wsl.desc")}
      >
        <Toggle
          checked={draft.ai_usage.claude_code.include_wsl_credentials}
          onChange={(include_wsl_credentials) =>
            updateAiUsage({
              ...draft.ai_usage,
              claude_code: { ...draft.ai_usage.claude_code, include_wsl_credentials },
            })
          }
        />
      </SettingRow>
      <TextAreaRow
        label={t("ai_usage.claude.extra_paths")}
        description={t("ai_usage.claude.extra_paths.desc")}
        value={draft.ai_usage.claude_code.extra_credentials_paths.join("\n")}
        placeholder="C:\\Users\\<user>\\.claude-alt\\.credentials.json"
        onChange={(value) =>
          updateAiUsage({
            ...draft.ai_usage,
            claude_code: {
              ...draft.ai_usage.claude_code,
              extra_credentials_paths: value
                .split("\n")
                .map((line) => line.trim())
                .filter(Boolean),
            },
          })
        }
      />
      <NumberRow
        compact
        label={t("ai_usage.claude.api_timeout")}
        description={t("ai_usage.claude.api_timeout.desc")}
        value={draft.ai_usage.claude_code.api_timeout_sec}
        min={1}
        unit="sec"
        onChange={(api_timeout_sec) =>
          updateAiUsage({
            ...draft.ai_usage,
            claude_code: { ...draft.ai_usage.claude_code, api_timeout_sec },
          })
        }
      />
    </>
  );
}

function TextAreaRow({
  label,
  description,
  value,
  placeholder,
  onChange,
}: {
  label: string;
  description: string;
  value: string;
  placeholder: string;
  onChange: (value: string) => void;
}) {
  return (
    <SettingRow compact align="start" label={label} description={description}>
      <textarea
        className="h-20 w-56 resize-none rounded-lg border border-border bg-background px-2.5 py-1.5 font-mono text-xs text-gray-700 placeholder:text-gray-300"
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
    </SettingRow>
  );
}
function UsageMetric({
  label,
  status,
  window,
  accent,
}: {
  label: string;
  status: AiUsageProviderStatus | null;
  window: "five_hour" | "seven_day";
  accent: "primary" | "amber";
}) {
  const { t } = useLang();
  const valid = window === "five_hour" ? status?.five_hour_valid : status?.seven_day_valid;
  const bp = window === "five_hour" ? status?.five_hour_used_bp : status?.seven_day_used_bp;
  const reset = window === "five_hour" ? status?.five_hour_reset_unix : status?.seven_day_reset_unix;
  const hasData = Boolean(valid && bp !== null && bp !== undefined);
  const pct = hasData ? Math.min(bp! / 100, 100) : 0;
  const barColor = usageBarColor(bp ?? 0, Boolean(valid), accent);
  return (
    <div>
      <div className="mb-1.5 flex items-center justify-between">
        <span className="text-xs font-medium text-gray-600">{label}</span>
        {hasData ? (
          <div className="flex items-center gap-2">
            <span className="text-xs font-semibold" style={{ color: usageTextColor(bp!, accent) }}>
              {formatUsedBp(bp!)}
            </span>
          </div>
        ) : (
          <span className="text-xs text-gray-400">-- {t("ai_usage.no_data")}</span>
        )}
      </div>
      <div className="h-2 overflow-hidden rounded-full bg-panel">
        <div className={`h-full rounded-full transition-all duration-500 ${barColor}`} style={{ width: `${pct}%` }} />
      </div>
      <div className="mt-1 flex justify-between gap-3">
        <span className="text-[11px] text-gray-400">{t("ai_usage.source.label")}: {status ? sourceLabel(status, t) : t("ai_usage.source.none")}</span>
        <span className="text-[11px] text-gray-400">
          {t("ai_usage.reset.label")}:{" "}
          <span className="font-mono">
            {status?.quota_source ? formatUnixShort(reset ?? null) : "-"}
          </span>
        </span>
      </div>
    </div>
  );
}

function ProviderIcon({ provider, small = false }: { provider: ProviderName; small?: boolean }) {
  const isCodex = provider === "codex";
  const size = small ? "h-6 w-6" : "h-7 w-7";
  const icon = isCodex ? codexIcon : claudeCodeIcon;
  const label = isCodex ? "Codex" : "Claude Code";
  return (
    <div className={`flex ${size} items-center justify-center`}>
      <img
        src={icon}
        alt={label}
        className="h-full w-full object-contain"
        draggable={false}
      />
    </div>
  );
}

function NumberRow({
  label,
  description,
  value,
  min,
  unit,
  onChange,
  compact = false,
}: {
  label: string;
  description: string;
  value: number;
  min: number;
  unit: string;
  onChange: (value: number) => void;
  compact?: boolean;
}) {
  return (
    <SettingRow compact={compact} label={label} description={description}>
      <div className="flex items-center gap-2">
        <input
          type="number"
          min={min}
          value={value}
          onChange={(e) => onChange(Math.max(min, Number(e.target.value)))}
          className={`${compact ? "w-20 px-2.5 py-1.5 text-xs" : "w-20 px-3 py-1.5 text-sm"} rounded-lg border border-border bg-background text-right font-mono text-gray-800`}
        />
        <span className="text-xs text-gray-400">{unit}</span>
      </div>
    </SettingRow>
  );
}

function PathRow({
  label,
  description,
  example,
  value,
  onChange,
  onReset,
}: {
  label: string;
  description: string;
  example: string;
  value: string | null;
  onChange: (value: string | null) => void;
  onReset: () => void;
}) {
  const { t } = useLang();
  return (
    <SettingRow compact align="start" label={label} description={`${description} / ${t("ai_usage.example", { path: example })}`}>
      <div className="flex flex-shrink-0 items-center gap-2">
        <input
          className="w-40 rounded-lg border border-border bg-background px-2.5 py-1.5 text-right font-mono text-xs text-gray-700 placeholder:text-gray-300"
          value={value ?? ""}
          placeholder={t("ai_usage.default")}
          onChange={(e) => onChange(e.target.value.trim() === "" ? null : e.target.value)}
        />
        <button
          type="button"
          onClick={onReset}
          className="rounded-lg p-1.5 text-gray-400 hover:bg-panel hover:text-primary"
          title={t("ai_usage.reset_default")}
        >
          <RotateCcw size={12} />
        </button>
      </div>
    </SettingRow>
  );
}

function StatusBadge({ status }: { status: AiUsageStatusKind }) {
  const { t } = useLang();
  const color =
    status === "ok"
      ? "bg-emerald-100 text-emerald-700"
      : status === "stale"
        ? "bg-amber-100 text-amber-700"
        : "bg-gray-100 text-gray-600";
  const dot =
    status === "ok" ? "bg-emerald-400" : status === "stale" ? "bg-amber-400" : "bg-gray-400";
  return (
    <span className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium ${color}`}>
      <span className={`h-1.5 w-1.5 rounded-full ${dot}`} />
      {t(aiStatusKey(status))}
    </span>
  );
}

type TFn = (key: TranslationKey, params?: Record<string, string | number>) => string;

function sourceLabel(status: AiUsageProviderStatus, t: TFn) {
  if (status.provider === "codex" && status.quota_source) return t("ai_usage.source.codex_rate_limits");
  if (status.provider === "codex" && status.local_history_source) return t("ai_usage.source.codex_history");
  if (status.provider === "claude_code" && status.quota_source) {
    const credentialSource = status.credential_source
      ? ` / ${t(`ai_usage.credential_source.${status.credential_source}` as TranslationKey)}`
      : "";
    return `${t("ai_usage.source.claude_oauth")}${credentialSource}`;
  }
  return t("ai_usage.source.none");
}

function errorCodeLabel(code: number | null, t: TFn) {
  if (code === null) return t("ai_usage.error.unknown");
  return t(`ai_usage.error.${code}` as TranslationKey);
}
