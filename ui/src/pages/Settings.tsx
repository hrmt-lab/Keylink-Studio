import { useEffect, useState } from "react";
import { Save, RefreshCcw, Check, X, Plus } from "lucide-react";
import { reloadConfig, getLaunchAtLogin, setLaunchAtLogin } from "../api";
import { Toggle } from "../components/Toggle";
import { ErrorNotice, PageHeader, PrimaryButton, SecondaryButton, SectionCard, SettingRow } from "../components/Ui";
import { useConfigSection } from "../hooks/useConfigSection";
import { useLang } from "../i18n";
import {
  PRESET_ACCENTS,
  getAccent,
  setAccent,
  getCustomAccents,
  addCustomAccent,
  removeCustomAccent,
} from "../lib/theme";
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

      {/* Appearance */}
      <SectionCard title={t("settings.appearance.section")}>
        <AccentPicker />
      </SectionCard>

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
              className="input !w-28 text-right font-mono"
            />
            <span className="text-sm text-muted w-8">ms</span>
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
            <span className="text-sm text-faint font-mono">0x</span>
            <input
              className="input !w-24 font-mono"
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
            <span className="text-sm text-faint font-mono">0x</span>
            <input
              className="input !w-24 font-mono"
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
              className="input !w-28 text-right font-mono"
            />
            <span className="text-sm text-muted w-8">ms</span>
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
              className="input !w-28 text-right font-mono"
            />
            <span className="text-sm text-muted w-8">sec</span>
          </div>
        </SettingRow>
      </SectionCard>

      <div className="rounded-card bg-plate px-4 py-3 text-xs text-muted space-y-1">
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

/** Accent-color picker: preset swatches + user-added custom colors. */
function AccentPicker() {
  const { t } = useLang();
  const [accent, setAccentState] = useState(getAccent());
  const [custom, setCustom] = useState<string[]>(getCustomAccents());

  const choose = (color: string) => {
    setAccent(color);
    setAccentState(getAccent());
  };

  const addAndChoose = (color: string) => {
    setCustom(addCustomAccent(color));
    choose(color);
  };

  const remove = (color: string) => {
    setCustom(removeCustomAccent(color));
  };

  return (
    <div className="space-y-3 px-5 py-4">
      <div>
        <div className="text-sm font-medium text-ink">{t("settings.appearance.accent")}</div>
        <div className="mt-0.5 text-xs text-muted">{t("settings.appearance.accent.desc")}</div>
      </div>
      <div className="flex flex-wrap items-center gap-3">
        {PRESET_ACCENTS.map((color) => (
          <AccentSwatch
            key={color}
            color={color}
            selected={accent === color}
            onSelect={() => choose(color)}
          />
        ))}
        {custom.length > 0 && <span className="h-6 w-px bg-border" aria-hidden="true" />}
        {custom.map((color) => (
          <AccentSwatch
            key={color}
            color={color}
            selected={accent === color}
            onSelect={() => choose(color)}
            onRemove={accent === color ? undefined : () => remove(color)}
            removeLabel={t("settings.appearance.accent.remove")}
          />
        ))}
        <label
          className="relative flex h-8 w-8 cursor-pointer items-center justify-center rounded-full border border-dashed border-disabled text-muted transition-colors hover:border-ink hover:text-ink"
          title={t("settings.appearance.accent.pick")}
        >
          <Plus size={14} />
          <input
            type="color"
            value={accent}
            onChange={(e) => addAndChoose(e.target.value)}
            className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
            aria-label={t("settings.appearance.accent.pick")}
          />
        </label>
      </div>
    </div>
  );
}

function AccentSwatch({ color, selected, onSelect, onRemove, removeLabel }: {
  color: string;
  selected: boolean;
  onSelect: () => void;
  onRemove?: () => void;
  removeLabel?: string;
}) {
  return (
    <span className="group relative inline-flex">
      <button
        onClick={onSelect}
        title={color}
        aria-label={color}
        aria-pressed={selected}
        className="flex h-8 w-8 items-center justify-center rounded-full transition-transform hover:scale-110"
        style={{
          backgroundColor: color,
          boxShadow: selected ? `0 0 0 2px #FFFFFF, 0 0 0 4px ${color}` : "inset 0 0 0 1px rgba(0,0,0,0.08)",
        }}
      >
        {selected && <Check size={14} className="text-white" />}
      </button>
      {onRemove && (
        <button
          onClick={onRemove}
          title={removeLabel}
          aria-label={removeLabel}
          className="absolute -right-1 -top-1 hidden h-4 w-4 items-center justify-center rounded-full bg-ink text-white group-hover:flex"
        >
          <X size={9} />
        </button>
      )}
    </span>
  );
}
