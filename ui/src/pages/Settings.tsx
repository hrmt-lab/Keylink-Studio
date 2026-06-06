import { useState } from "react";
import { Save, RefreshCcw } from "lucide-react";
import { saveConfig, reloadConfig } from "../api";
import { ErrorNotice, PageHeader, PrimaryButton, SecondaryButton, SectionCard, SettingRow } from "../components/Ui";
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
  const [error, setError] = useState<string | null>(null);

  const isDirty = JSON.stringify(draft) !== JSON.stringify(config);

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      await saveConfig(draft);
      setConfig(draft);
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
      <PageHeader
        title={t("settings.title")}
        description={t("settings.subtitle")}
        actions={
          <>
            <SecondaryButton onClick={handleReload} icon={<RefreshCcw size={15} />}>
              {t("settings.reload")}
            </SecondaryButton>
            <PrimaryButton
              onClick={handleSave}
              disabled={!isDirty}
              loading={saving}
              icon={<Save size={15} />}
            >
              {t("settings.save")}
            </PrimaryButton>
          </>
        }
      />

      {error && <ErrorNotice message={error} />}

      {/* Polling */}
      <SectionCard title={t("settings.polling.section")}>
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
      </SectionCard>

      {/* HID */}
      <SectionCard title={t("settings.hid.section")}>
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

        <SettingRow
          label={t("settings.hid.rescan_interval")}
          description={t("settings.hid.rescan_interval.desc")}
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={1}
              max={3600}
              value={draft.hid.rescan_interval_sec}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  hid: {
                    ...draft.hid,
                    rescan_interval_sec: Math.max(1, Number(e.target.value)),
                  },
                })
              }
              className="input w-28 text-right"
            />
            <span className="text-sm text-gray-500 w-8">sec</span>
          </div>
        </SettingRow>
      </SectionCard>

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
