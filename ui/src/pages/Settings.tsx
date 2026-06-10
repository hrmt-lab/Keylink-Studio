import { useEffect, useState } from "react";
import { Save, RefreshCcw } from "lucide-react";
import { reloadConfig, getLaunchAtLogin, setLaunchAtLogin } from "../api";
import { Toggle } from "../components/Toggle";
import { ErrorNotice, PageHeader, PrimaryButton, SecondaryButton, SectionCard, SettingRow } from "../components/Ui";
import { useConfigSection } from "../hooks/useConfigSection";
import { useLang } from "../i18n";
import type { AppConfig } from "../types";

interface Props {
  config: AppConfig;
  setConfig: (c: AppConfig) => void;
}

const MAX_USAGE = 0xffff;

export default function Settings({ config, setConfig }: Props) {
  const { t } = useLang();
  const { draft, setDraft, isDirty, saving, error, setError, save } = useConfigSection({
    config,
    setConfig,
    select: (c) => c,
    apply: (_c, d) => d,
  });

  const [launchAtLogin, setLaunchAtLoginState] = useState(false);
  const [launchBusy, setLaunchBusy] = useState(false);

  useEffect(() => {
    void getLaunchAtLogin()
      .then(setLaunchAtLoginState)
      .catch(() => {});
  }, []);

  const handleReload = async () => {
    try {
      setConfig(await reloadConfig());
    } catch (e) {
      setError(String(e));
    }
  };

  const updateHex = (field: "usage_page" | "usage", raw: string) => {
    const value = parseInt(raw, 16);
    if (isNaN(value) || value < 0 || value > MAX_USAGE) {
      setError(t("settings.app.hex_invalid"));
      return;
    }
    setError(null);
    setDraft({ ...draft, hid: { ...draft.hid, [field]: value } });
  };

  const toggleLaunchAtLogin = async (enabled: boolean) => {
    setLaunchBusy(true);
    setError(null);
    const previous = launchAtLogin;
    setLaunchAtLoginState(enabled);
    try {
      await setLaunchAtLogin(enabled);
    } catch (e) {
      setLaunchAtLoginState(previous);
      setError(String(e));
    } finally {
      setLaunchBusy(false);
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
              onClick={save}
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

      {/* App startup */}
      <SectionCard title={t("settings.app.section")}>
        <SettingRow
          label={t("settings.app.start_on_launch")}
          description={t("settings.app.start_on_launch.desc")}
        >
          <Toggle
            checked={draft.app.start_monitoring_on_launch}
            onChange={(v) =>
              setDraft({ ...draft, app: { ...draft.app, start_monitoring_on_launch: v } })
            }
            label={t("settings.app.start_on_launch")}
          />
        </SettingRow>
        <SettingRow
          label={t("settings.app.launch_at_login")}
          description={t("settings.app.launch_at_login.desc")}
        >
          <Toggle
            checked={launchAtLogin}
            disabled={launchBusy}
            onChange={toggleLaunchAtLogin}
            label={t("settings.app.launch_at_login")}
          />
        </SettingRow>
      </SectionCard>

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
              onChange={(e) => updateHex("usage_page", e.target.value)}
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
              onChange={(e) => updateHex("usage", e.target.value)}
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
