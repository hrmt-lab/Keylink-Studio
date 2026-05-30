import { useState } from "react";
import { Save, AlertCircle, RefreshCcw } from "lucide-react";
import { saveConfig, reloadConfig } from "../api";
import { useLang } from "../i18n";
import type { AppConfig } from "../types";

interface Props {
  config: AppConfig;
  setConfig: (c: AppConfig) => void;
}

export default function Settings({ config, setConfig }: Props) {
  const { t } = useLang();
  const [draft, setDraft] = useState(config);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isDirty = JSON.stringify(draft) !== JSON.stringify(config);

  const handleSave = async () => {
    setSaving(true);
    setSaved(false);
    setError(null);
    try {
      await saveConfig(draft);
      setConfig(draft);
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleReload = async () => {
    try {
      const loaded = await reloadConfig();
      setConfig(loaded);
      setDraft(loaded);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="p-6 max-w-2xl mx-auto space-y-5">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold text-gray-800">{t("settings.title")}</h1>
          <p className="text-sm text-gray-500 mt-0.5">{t("settings.subtitle")}</p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={handleReload}
            className="flex items-center gap-2 rounded-lg border border-border bg-white px-4 py-2.5 text-sm font-medium text-gray-600 hover:bg-panel transition-colors"
          >
            <RefreshCcw size={15} />
            {t("settings.reload")}
          </button>
          <button
            onClick={handleSave}
            disabled={saving || !isDirty}
            className="flex items-center gap-2 rounded-lg bg-primary px-4 py-2.5 text-sm font-medium text-white hover:bg-primary-dark disabled:opacity-50 transition-colors"
          >
            {saving ? (
              <div className="h-4 w-4 animate-spin rounded-full border-2 border-white/30 border-t-white" />
            ) : (
              <Save size={15} />
            )}
            {saved ? t("settings.saved") : t("settings.save")}
          </button>
        </div>
      </div>

      {error && (
        <div className="flex items-start gap-2.5 rounded-lg bg-red-50 px-4 py-3 text-sm text-red-700 ring-1 ring-red-200">
          <AlertCircle size={15} className="mt-0.5 flex-shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {/* Polling */}
      <Section title={t("settings.polling.section")}>
        <SettingRow
          label={t("settings.polling.interval")}
          description={t("settings.polling.interval.desc")}
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={50}
              max={10000}
              step={50}
              value={draft.polling.interval_ms}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  polling: {
                    ...draft.polling,
                    interval_ms: Math.max(50, Number(e.target.value)),
                  },
                })
              }
              className="input w-28 text-right"
            />
            <span className="text-sm text-gray-500 w-8">ms</span>
          </div>
        </SettingRow>
      </Section>

      {/* HID */}
      <Section title={t("settings.hid.section")}>
        <SettingRow
          label={t("settings.hid.usage_page")}
          description={t("settings.hid.usage_page.desc")}
        >
          <div className="flex items-center gap-2">
            <span className="text-sm text-gray-400 font-mono">0x</span>
            <input
              className="input w-24 font-mono"
              value={draft.hid.usage_page.toString(16).toUpperCase()}
              onChange={(e) => {
                const v = parseInt(e.target.value, 16);
                if (!isNaN(v)) {
                  setDraft({ ...draft, hid: { ...draft.hid, usage_page: v } });
                }
              }}
              placeholder="FF60"
            />
          </div>
        </SettingRow>

        <SettingRow
          label={t("settings.hid.usage")}
          description={t("settings.hid.usage.desc")}
        >
          <div className="flex items-center gap-2">
            <span className="text-sm text-gray-400 font-mono">0x</span>
            <input
              className="input w-24 font-mono"
              value={draft.hid.usage.toString(16).toUpperCase()}
              onChange={(e) => {
                const v = parseInt(e.target.value, 16);
                if (!isNaN(v)) {
                  setDraft({ ...draft, hid: { ...draft.hid, usage: v } });
                }
              }}
              placeholder="61"
            />
          </div>
        </SettingRow>

        <SettingRow
          label={t("settings.hid.timeout")}
          description={t("settings.hid.timeout.desc")}
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={50}
              max={5000}
              value={draft.hid.hello_timeout_ms}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  hid: {
                    ...draft.hid,
                    hello_timeout_ms: Math.max(50, Number(e.target.value)),
                  },
                })
              }
              className="input w-28 text-right"
            />
            <span className="text-sm text-gray-500 w-8">ms</span>
          </div>
        </SettingRow>
      </Section>

      <div className="rounded-lg bg-background px-4 py-3 text-xs text-gray-400 ring-1 ring-border space-y-1">
        <div>
          {t("settings.note1", { file: "rawhid-host.toml" })}
        </div>
        <div>
          {t("settings.note2", { up: "0xFF60", u: "0x61" })}
        </div>
      </div>
    </div>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-xl bg-white shadow-card ring-1 ring-border overflow-hidden">
      <div className="border-b border-border/60 px-5 py-3">
        <h2 className="text-sm font-semibold text-gray-700">{title}</h2>
      </div>
      <div className="divide-y divide-border/60">{children}</div>
    </div>
  );
}

function SettingRow({
  label,
  description,
  children,
}: {
  label: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between px-5 py-4 gap-4">
      <div className="min-w-0">
        <div className="text-sm font-medium text-gray-800">{label}</div>
        <div className="text-xs text-gray-500 mt-0.5">{description}</div>
      </div>
      <div className="flex-shrink-0">{children}</div>
    </div>
  );
}
