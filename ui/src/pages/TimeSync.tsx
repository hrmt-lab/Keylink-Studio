import { Save } from "lucide-react";
import { Toggle } from "../components/Toggle";
import { ErrorNotice, PageHeader, PrimaryButton } from "../components/Ui";
import { useConfigSection } from "../hooks/useConfigSection";
import { useLang } from "../i18n";
import { formatTzOffset } from "../lib/format";
import type { AppConfig, TimeFormatHint } from "../types";

interface Props {
  config: AppConfig;
  setConfig: (c: AppConfig) => void;
}

// Standard UTC offsets in current worldwide use, in minutes.
const TZ_PRESETS_MIN: number[] = [
  -720, -660, -600, -570, -540, -480, -420, -360, -300, -240, -210, -180,
  -120, -60, 0, 60, 120, 180, 210, 240, 270, 300, 330, 345, 360, 390, 420,
  480, 525, 540, 570, 600, 630, 660, 720, 765, 780, 840,
];

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
  const { draft, setDraft, isDirty, saving, error, save } = useConfigSection({
    config,
    setConfig,
    select: (c) => c.time,
    apply: (c, time) => ({ ...c, time }),
  });

  return (
    <div className="p-6 max-w-2xl mx-auto space-y-5">
      <PageHeader
        title={t("timesync.title")}
        description={t("timesync.subtitle")}
        actions={
          <PrimaryButton
            onClick={save}
            disabled={!isDirty}
            loading={saving}
            icon={<Save size={15} />}
          >
            {t("timesync.save")}
          </PrimaryButton>
        }
      />

      {error && <ErrorNotice message={error} />}

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
            label={t("timesync.enable")}
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
            label={t("timesync.clock_mode")}
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
          <select
            value={draft.tz_offset_min == null ? "" : String(draft.tz_offset_min)}
            onChange={(e) =>
              setDraft({
                ...draft,
                tz_offset_min: e.target.value === "" ? null : Number(e.target.value),
              })
            }
            aria-label={t("timesync.timezone")}
            className="input w-48"
          >
            <option value="">{t("timesync.timezone.auto")}</option>
            {(draft.tz_offset_min == null || TZ_PRESETS_MIN.includes(draft.tz_offset_min)
              ? TZ_PRESETS_MIN
              : [...TZ_PRESETS_MIN, draft.tz_offset_min].sort((a, b) => a - b)
            ).map((min) => (
              <option key={min} value={min}>
                {formatTzOffset(min)}
              </option>
            ))}
          </select>
        </div>
      </div>

    </div>
  );
}
