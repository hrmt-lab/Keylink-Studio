import { useState } from "react";
import { Save, AlertCircle } from "lucide-react";
import { saveConfig } from "../api";
import { Toggle } from "../components/Toggle";
import { useLang } from "../i18n";
import type { AppConfig, TimeFormatHint } from "../types";

interface Props {
  config: AppConfig;
  setConfig: (c: AppConfig) => void;
}

const FORMAT_OPTIONS: { value: TimeFormatHint; labelKey: string; example: string }[] = [
  { value: "time_hm",      labelKey: "format.time_hm",      example: "14:30" },
  { value: "time_hms",     labelKey: "format.time_hms",     example: "14:30:05" },
  { value: "date_ymd",     labelKey: "format.date_ymd",     example: "2025-01-15" },
  { value: "date_md",      labelKey: "format.date_md",      example: "01-15" },
  { value: "datetime_hm",  labelKey: "format.datetime_hm",  example: "2025-01-15 14:30" },
  { value: "weekday_hm",   labelKey: "format.weekday_hm",   example: "Wed 14:30" },
];

export default function TimeSync({ config, setConfig }: Props) {
  const { t } = useLang();
  const [draft, setDraft] = useState(config.time);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isDirty = JSON.stringify(draft) !== JSON.stringify(config.time);

  const handleSave = async () => {
    setSaving(true);
    setSaved(false);
    setError(null);
    try {
      const updated = { ...config, time: draft };
      await saveConfig(updated);
      setConfig(updated);
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="p-6 max-w-2xl mx-auto space-y-5">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold text-gray-800">{t("timesync.title")}</h1>
          <p className="text-sm text-gray-500 mt-0.5">{t("timesync.subtitle")}</p>
        </div>
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
          {saved ? t("timesync.saved") : t("timesync.save")}
        </button>
      </div>

      {error && (
        <div className="flex items-start gap-2.5 rounded-lg bg-red-50 px-4 py-3 text-sm text-red-700 ring-1 ring-red-200">
          <AlertCircle size={15} className="mt-0.5 flex-shrink-0" />
          <span>{error}</span>
        </div>
      )}

      <div className="rounded-xl bg-white shadow-card ring-1 ring-border divide-y divide-border/60">
        {/* Enable */}
        <div className="flex items-center justify-between px-5 py-4">
          <div>
            <div className="text-sm font-medium text-gray-800">{t("timesync.enable")}</div>
            <div className="text-xs text-gray-500 mt-0.5">{t("timesync.enable.desc")}</div>
          </div>
          <Toggle
            checked={draft.enabled}
            onChange={(v) => setDraft({ ...draft, enabled: v })}
          />
        </div>

        {/* Format */}
        <div className="px-5 py-4">
          <div className="text-sm font-medium text-gray-800 mb-3">{t("timesync.format")}</div>
          <div className="grid grid-cols-2 gap-2">
            {FORMAT_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setDraft({ ...draft, format_hint: opt.value })}
                className={`flex items-center justify-between rounded-lg border px-3 py-2.5 text-sm text-left transition-all ${
                  draft.format_hint === opt.value
                    ? "border-primary bg-primary/5 text-primary ring-1 ring-primary/30"
                    : "border-border text-gray-600 hover:border-secondary hover:bg-background"
                }`}
              >
                <span>{t(opt.labelKey as Parameters<typeof t>[0])}</span>
                <span className="font-mono text-xs text-gray-400">{opt.example}</span>
              </button>
            ))}
          </div>
        </div>

        {/* Clock mode */}
        <div className="flex items-center justify-between px-5 py-4">
          <div>
            <div className="text-sm font-medium text-gray-800">{t("timesync.clock_mode")}</div>
            <div className="text-xs text-gray-500 mt-0.5">{t("timesync.clock_mode.desc")}</div>
          </div>
          <Toggle
            checked={draft.clock_mode === "12h"}
            onChange={(v) =>
              setDraft({ ...draft, clock_mode: v ? "12h" : "24h" })
            }
          />
        </div>

        {/* Sync interval */}
        <div className="flex items-center justify-between px-5 py-4">
          <div>
            <div className="text-sm font-medium text-gray-800">{t("timesync.sync_interval")}</div>
            <div className="text-xs text-gray-500 mt-0.5">{t("timesync.sync_interval.desc")}</div>
          </div>
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={0}
              max={86400}
              value={draft.periodic_sync_sec}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  periodic_sync_sec: Math.max(0, Number(e.target.value)),
                })
              }
              className="input w-24 text-right"
            />
            <span className="text-sm text-gray-500">{t("timesync.sync_interval.unit")}</span>
          </div>
        </div>

        {/* Timezone */}
        <div className="flex items-center justify-between px-5 py-4">
          <div>
            <div className="text-sm font-medium text-gray-800">{t("timesync.timezone")}</div>
            <div className="text-xs text-gray-500 mt-0.5">{t("timesync.timezone.desc")}</div>
          </div>
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={-1440}
              max={1440}
              value={draft.tz_offset_min ?? ""}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  tz_offset_min: e.target.value === "" ? null : Number(e.target.value),
                })
              }
              placeholder={t("timesync.timezone.auto")}
              className="input w-24 text-right"
            />
            <span className="text-sm text-gray-500">{t("timesync.timezone.unit")}</span>
          </div>
        </div>
      </div>

    </div>
  );
}
