import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Save, RefreshCcw, Check, X, Plus, Copy, Play, Square, Terminal, FolderOpen } from "lucide-react";
import {
  getCodexIntegrationStatus,
  getLaunchAtLogin,
  launchClaudeCode,
  launchCodexCli,
  listWslDistributions,
  reloadConfig,
  setLaunchAtLogin,
  startCodexIntegration,
  stopCodexIntegration,
  stopClaudeCode,
} from "../api";
import { Toggle } from "../components/Toggle";
import { ErrorNotice, PageHeader, PrimaryButton, SecondaryButton, SectionCard, SettingRow } from "../components/Ui";
import { useConfigSection } from "../hooks/useConfigSection";
import { friendlyError } from "../lib/errors";
import { useLang } from "../i18n";
import {
  PRESET_ACCENTS,
  getAccent,
  setAccent,
  getCustomAccents,
  addCustomAccent,
  removeCustomAccent,
} from "../lib/theme";
import type { AppConfig, CodexBrokerStatus, MonitorStatus, WslDistribution } from "../types";

interface Props {
  config: AppConfig;
  setConfig: (c: AppConfig) => void;
  status: MonitorStatus;
}

const MAX_USAGE = 0xffff;

export default function Settings({ config, setConfig, status }: Props) {
  const { t } = useLang();
  const { draft, setDraft, isDirty, saving, error, setError, save, rebase } = useConfigSection({
    config,
    setConfig,
    select: (c) => c,
    apply: (_c, d) => d,
    t,
  });

  // "Launch at login" is an OS-level setting (outside AppConfig), so it has its
  // own draft. Like the rest of this page it is applied on the Save button, not
  // immediately. launchSaved is the persisted baseline; launchDraft is the edit.
  const [launchSaved, setLaunchSaved] = useState(false);
  const [launchDraft, setLaunchDraft] = useState(false);
  const [launchBusy, setLaunchBusy] = useState(false);
  const launchDirty = launchDraft !== launchSaved;

  useEffect(() => {
    void getLaunchAtLogin()
      .then((v) => {
        setLaunchSaved(v);
        setLaunchDraft(v);
      })
      .catch(() => {});
  }, []);

  const handleReload = async () => {
    try {
      setConfig(await reloadConfig());
    } catch (e) {
      setError(friendlyError(e, t));
    }
  };

  const handleSave = async () => {
    if (isDirty) await save();
    if (launchDirty) {
      setLaunchBusy(true);
      try {
        await setLaunchAtLogin(launchDraft);
        setLaunchSaved(launchDraft);
      } catch (e) {
        setLaunchDraft(launchSaved);
        setError(friendlyError(e, t));
      } finally {
        setLaunchBusy(false);
      }
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
              disabled={!isDirty && !launchDirty}
              loading={saving || launchBusy}
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
            checked={launchDraft}
            disabled={launchBusy}
            onChange={(v) => {
              setError(null);
              setLaunchDraft(v);
            }}
            label={t("settings.app.launch_at_login")}
          />
        </SettingRow>
      </SectionCard>

      <CodexIntegration
        draft={draft}
        setDraft={setDraft}
        savedConfig={config}
        rebaseConfig={rebase}
        status={status}
      />
      <ClaudeCodeIntegration draft={draft} setDraft={setDraft} />

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
        <SettingRow
          label={t("settings.polling.uplink_interval")}
          description={t("settings.polling.uplink_interval.desc")}
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={5}
              max={500}
              step={5}
              value={draft.polling.uplink_interval_ms}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  polling: {
                    ...draft.polling,
                    uplink_interval_ms: Math.max(5, Number(e.target.value)),
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
              step={50}
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
          {t("settings.note1", { file: "keylink-studio.toml" })}
        </div>
        <div>
          {t("settings.note2", { up: "0xFF60", u: "0x61" })}
        </div>
      </div>
    </div>
  );
}

function CodexIntegration({
  draft,
  setDraft,
  savedConfig,
  rebaseConfig,
  status,
}: {
  draft: AppConfig;
  setDraft: React.Dispatch<React.SetStateAction<AppConfig>>;
  savedConfig: AppConfig;
  rebaseConfig: (config: AppConfig, preserveDraft: (draft: AppConfig) => AppConfig) => void;
  status: MonitorStatus;
}) {
  const { t, lang } = useLang();
  const [broker, setBroker] = useState<CodexBrokerStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [stopConfirmOpen, setStopConfirmOpen] = useState(false);
  const [launchBusy, setLaunchBusy] = useState(false);
  const [launched, setLaunched] = useState(false);
  const [wslDistributions, setWslDistributions] = useState<WslDistribution[]>([]);
  const [wslLoadError, setWslLoadError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    const refresh = () => {
      void getCodexIntegrationStatus()
        .then((next) => active && setBroker(next))
        .catch(() => active && setBroker(null));
    };
    refresh();
    const timer = window.setInterval(refresh, 1000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, []);

  const phase = broker?.phase ?? "stopped";
  const editable = phase === "stopped" || phase === "error";
  const running = !editable;
  const launchable =
    phase === "stopped" ||
    phase === "error" ||
    phase === "waiting_for_client" ||
    phase === "connected" ||
    phase === "reconnecting";
  const capableDevices = status.host_link_devices.filter(
    (device) => (device.capabilities & (1 << 10)) !== 0,
  ).length;
  const codex = draft.ai_client.codex;
  const launcher = draft.ai_client.codex_launcher;
  const codexConfigDirty =
    JSON.stringify(codex) !== JSON.stringify(savedConfig.ai_client.codex);
  const updateCodex = (next: Partial<typeof codex>) => {
    setDraft({
      ...draft,
      ai_client: { ...draft.ai_client, codex: { ...codex, ...next } },
    });
  };
  const updateLauncher = (next: Partial<typeof launcher>) => {
    setDraft({
      ...draft,
      ai_client: {
        ...draft.ai_client,
        codex_launcher: { ...launcher, ...next },
      },
    });
  };
  useEffect(() => {
    if (launcher.environment !== "wsl") return;
    let active = true;
    void listWslDistributions()
      .then((items) => {
        if (!active) return;
        setWslDistributions(items);
        setWslLoadError(items.length === 0 ? t("settings.codex.launcher.wsl_none") : null);
      })
      .catch((error) => {
        if (active) setWslLoadError(String(error));
      });
    return () => {
      active = false;
    };
  }, [launcher.environment, lang]);
  const run = async (action: "start" | "stop") => {
    setBusy(true);
    setActionError(null);
    try {
      setBroker(action === "start" ? await startCodexIntegration() : await stopCodexIntegration());
    } catch (error) {
      setActionError(String(error));
    } finally {
      setBusy(false);
    }
  };
  const requestStop = async () => {
    setBusy(true);
    setActionError(null);
    try {
      const latest = await getCodexIntegrationStatus();
      setBroker(latest);
      if (latest.client_connected) {
        setStopConfirmOpen(true);
        return;
      }
      setBroker(await stopCodexIntegration());
    } catch (error) {
      setActionError(String(error));
    } finally {
      setBusy(false);
    }
  };
  const confirmStop = () => {
    setStopConfirmOpen(false);
    void run("stop");
  };
  const copyCommand = async () => {
    if (!broker?.cli_connection_command) return;
    try {
      await navigator.clipboard.writeText(broker.cli_connection_command);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      setActionError(t("settings.codex.copy_failed"));
    }
  };
  const browseProject = async () => {
    setActionError(null);
    if (launcher.environment === "windows") {
      const selected = await open({
        multiple: false,
        directory: true,
        defaultPath: launcher.windows_project_directory ?? undefined,
      });
      if (typeof selected === "string") {
        updateLauncher({ windows_project_directory: selected });
      }
      return;
    }
    const distro = launcher.wsl_distribution?.trim();
    if (!distro) {
      setActionError(t("settings.codex.launcher.wsl_required"));
      return;
    }
    const selected = await open({
      multiple: false,
      directory: true,
      defaultPath: wslPathToUnc(distro, launcher.wsl_project_directory ?? "/"),
    });
    if (typeof selected !== "string") return;
    const linuxPath = uncToWslPath(selected, distro);
    if (!linuxPath) {
      setActionError(t("settings.codex.launcher.wsl_path_invalid"));
      return;
    }
    updateLauncher({ wsl_project_directory: linuxPath });
  };
  const launchCli = async () => {
    setLaunchBusy(true);
    setActionError(null);
    setLaunched(false);
    try {
      const result = await launchCodexCli(launcher);
      rebaseConfig(result.config, (current) => ({
        ...current,
        ai_client: {
          ...current.ai_client,
          codex_launcher: result.config.ai_client.codex_launcher,
        },
      }));
      setBroker(await getCodexIntegrationStatus());
      setLaunched(true);
      window.setTimeout(() => setLaunched(false), 2400);
    } catch (error) {
      setActionError(String(error));
      setBroker(await getCodexIntegrationStatus().catch(() => broker));
    } finally {
      setLaunchBusy(false);
    }
  };

  return (
    <SectionCard title={t("settings.codex.section")}>
      <div className="px-5 py-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2.5">
            <span className={`h-2.5 w-2.5 rounded-full ${phase === "connected" ? "bg-accent" : phase === "error" ? "bg-red-500" : running ? "bg-amber-400" : "bg-disabled"}`} />
            <div>
              <p className="text-sm font-medium text-ink">{t(`settings.codex.phase.${phase}`)}</p>
              <p className="mt-0.5 text-xs text-faint">{broker?.codex_version ? t("settings.codex.version", { version: broker.codex_version }) : t("settings.codex.version_unknown")}</p>
              <p className="mt-0.5 text-xs text-faint">
                {t("settings.codex.connected_clients", {
                  count: broker?.connected_client_count ?? 0,
                  max: broker?.max_client_count ?? 8,
                })}
              </p>
            </div>
          </div>
          {running ? (
            <SecondaryButton onClick={() => void requestStop()} disabled={busy || launchBusy} loading={busy} icon={<Square size={14} />}>{t("settings.codex.stop")}</SecondaryButton>
          ) : (
            <PrimaryButton onClick={() => void run("start")} disabled={busy || launchBusy || codexConfigDirty} loading={busy} icon={<Play size={14} />}>{t("settings.codex.start")}</PrimaryButton>
          )}
        </div>
        {codexConfigDirty && editable && <p className="mt-3 text-xs text-amber-700">{t("settings.codex.save_first")}</p>}
        {broker?.app_server_port != null && broker?.broker_port != null && (
          <p className="mt-3 text-xs text-faint">
            {t("settings.codex.active_ports", {
              appServer: broker.app_server_port,
              broker: broker.broker_port,
            })}
          </p>
        )}
      </div>
      {(actionError || broker?.last_error) && <div className="border-t border-background px-5 py-3"><ErrorNotice message={t("settings.codex.error")} details={broker?.last_error ?? actionError} /></div>}
      <SettingRow label={t("settings.codex.executable")} description={t("settings.codex.executable.desc")} align="start">
        <input className="input w-64 max-w-full font-mono text-xs" value={codex.executable_path ?? ""} disabled={!editable} onChange={(event) => updateCodex({ executable_path: event.target.value.trim() || null })} placeholder={t("settings.codex.path_placeholder")} />
      </SettingRow>
      <SettingRow label={t("settings.codex.app_server_port")} description={t("settings.codex.app_server_port.desc")}>
        <input className="input !w-28 text-right font-mono" type="number" min={1024} max={65535} disabled={!editable} value={codex.app_server_port} onChange={(event) => updateCodex({ app_server_port: Math.max(1024, Math.min(65535, Number(event.target.value))) })} />
      </SettingRow>
      <SettingRow label={t("settings.codex.broker_port")} description={t("settings.codex.broker_port.desc")}>
        <input className="input !w-28 text-right font-mono" type="number" min={1024} max={65535} disabled={!editable} value={codex.broker_port} onChange={(event) => updateCodex({ broker_port: Math.max(1024, Math.min(65535, Number(event.target.value))) })} />
      </SettingRow>
      <div className="border-t border-background px-5 py-4">
        <p className="text-sm font-medium text-ink">{t("settings.codex.launcher.title")}</p>
        <p className="mt-1 text-xs text-faint">{t("settings.codex.launcher.desc")}</p>
      </div>
      <SettingRow label={t("settings.codex.launcher.environment")} description={t("settings.codex.launcher.environment.desc")}>
        <select
          className="input !w-40"
          value={launcher.environment}
          onChange={(event) => updateLauncher({ environment: event.target.value as "windows" | "wsl" })}
        >
          <option value="windows">Windows</option>
          <option value="wsl">WSL</option>
        </select>
      </SettingRow>
      {launcher.environment === "wsl" && (
        <>
          <SettingRow label={t("settings.codex.launcher.wsl_distribution")} description={t("settings.codex.launcher.wsl_distribution.desc")} align="start">
            <div className="w-72 max-w-full">
              <input
                className="input w-full font-mono text-xs"
                list="codex-wsl-distributions"
                value={launcher.wsl_distribution ?? ""}
                onChange={(event) => updateLauncher({ wsl_distribution: event.target.value || null })}
                placeholder="Ubuntu"
              />
              <datalist id="codex-wsl-distributions">
                {wslDistributions.map((distribution) => (
                  <option key={distribution.name} value={distribution.name}>
                    WSL {distribution.version}
                  </option>
                ))}
              </datalist>
              {wslLoadError && <p className="mt-1 text-xs text-amber-700">{wslLoadError}</p>}
            </div>
          </SettingRow>
          <SettingRow label={t("settings.codex.launcher.wsl_executable")} description={t("settings.codex.launcher.wsl_executable.desc")} align="start">
            <input
              className="input w-72 max-w-full font-mono text-xs"
              value={launcher.wsl_executable}
              onChange={(event) => updateLauncher({ wsl_executable: event.target.value })}
              placeholder="codex"
            />
          </SettingRow>
        </>
      )}
      <SettingRow label={t("settings.codex.launcher.project")} description={t("settings.codex.launcher.project.desc")} align="start">
        <div className="flex w-96 max-w-full items-center gap-2">
          <input
            className="input min-w-0 flex-1 font-mono text-xs"
            value={launcher.environment === "windows" ? launcher.windows_project_directory ?? "" : launcher.wsl_project_directory ?? ""}
            onChange={(event) =>
              updateLauncher(
                launcher.environment === "windows"
                  ? { windows_project_directory: event.target.value || null }
                  : { wsl_project_directory: event.target.value || null },
              )
            }
            placeholder={launcher.environment === "windows" ? "C:\\path\\to\\project" : "/home/user/project"}
          />
          <SecondaryButton onClick={() => void browseProject()} icon={<FolderOpen size={14} />}>
            {t("settings.codex.launcher.browse")}
          </SecondaryButton>
        </div>
      </SettingRow>
      <div className="border-t border-background px-5 py-4">
        <div className="flex flex-wrap items-center gap-3">
          <PrimaryButton
            onClick={() => void launchCli()}
            disabled={busy || launchBusy || codexConfigDirty || !launchable || (broker?.connected_client_count ?? 0) >= (broker?.max_client_count ?? 8)}
            loading={launchBusy}
            icon={<Terminal size={15} />}
          >
            {t("settings.codex.launcher.open")}
          </PrimaryButton>
          {launched && <span className="text-xs font-medium text-accent-deep">{t("settings.codex.launcher.opened")}</span>}
        </div>
        {codexConfigDirty && <p className="mt-2 text-xs text-amber-700">{t("settings.codex.save_first")}</p>}
      </div>
      <SettingRow label={t("settings.codex.devices")} description={t("settings.codex.devices.desc")}>
        <span className={`text-sm font-medium ${capableDevices > 0 ? "text-ink" : "text-amber-700"}`}>{t("settings.codex.device_count", { count: capableDevices })}</span>
      </SettingRow>
      {capableDevices === 0 && <p className="border-t border-background px-5 py-3 text-xs text-amber-700">{t("settings.codex.no_devices")}</p>}
      {broker?.cli_connection_command && <div className="border-t border-background px-5 py-4">
        <p className="text-sm font-medium text-ink">{t("settings.codex.command")}</p>
        <p className="mt-1 text-xs text-faint">{t("settings.codex.command.desc")}</p>
        <div className="mt-3 flex items-start gap-2 rounded-xl bg-plate p-3 ring-1 ring-border">
          <code className="min-w-0 flex-1 break-all font-mono text-xs leading-5 text-muted">{broker.cli_connection_command}</code>
          <button onClick={() => void copyCommand()} className="rounded-lg p-2 text-muted hover:bg-surface hover:text-ink" title={t("settings.codex.copy")}>{copied ? <Check size={15} className="text-accent-deep" /> : <Copy size={15} />}</button>
        </div>
      </div>}
      {stopConfirmOpen && (
        <CodexStopConfirmDialog
          onCancel={() => setStopConfirmOpen(false)}
          onConfirm={confirmStop}
        />
      )}
    </SectionCard>
  );
}

function ClaudeCodeIntegration({
  draft,
  setDraft,
}: {
  draft: AppConfig;
  setDraft: React.Dispatch<React.SetStateAction<AppConfig>>;
}) {
  const launcher = draft.ai_client.claude_launcher;
  const [busy, setBusy] = useState(false);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const update = (next: Partial<typeof launcher>) => {
    setDraft({
      ...draft,
      ai_client: {
        ...draft.ai_client,
        claude_launcher: { ...launcher, ...next },
      },
    });
  };
  const browse = async () => {
    const selected = await open({
      multiple: false,
      directory: true,
      defaultPath: launcher.project_directory ?? undefined,
    });
    if (typeof selected === "string") update({ project_directory: selected });
  };
  const launch = async () => {
    setBusy(true);
    setError(null);
    try {
      await launchClaudeCode(launcher);
      setRunning(true);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };
  const stop = async () => {
    setBusy(true);
    setError(null);
    try {
      await stopClaudeCode();
      setRunning(false);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };
  return (
    <SectionCard title="Claude Code連携（ScreenKey）">
      <div className="px-5 py-4">
        <p className="text-xs text-faint">
          Claude CodeをWindows Terminalで起動し、状態をScreenKeyへ送信します。複数起動した場合は、HOST_ACTIONを割り当てたキーでCodexを含む表示セッションを切り替えられます。
        </p>
      </div>
      {error && <div className="border-t border-background px-5 py-3"><ErrorNotice message="Claude Code連携を開始できません" details={error} /></div>}
      <SettingRow label="Claude Code実行ファイル" description="空欄ならPATH上の claude を使用します。" align="start">
        <input className="input w-72 max-w-full font-mono text-xs" value={launcher.executable_path ?? ""} disabled={busy} onChange={(event) => update({ executable_path: event.target.value.trim() || null })} placeholder="claude" />
      </SettingRow>
      <SettingRow label="プロジェクト" description="Claude Codeを起動するWindowsのプロジェクトフォルダです。" align="start">
        <div className="flex w-96 max-w-full items-center gap-2">
          <input className="input min-w-0 flex-1 font-mono text-xs" value={launcher.project_directory ?? ""} disabled={busy} onChange={(event) => update({ project_directory: event.target.value || null })} placeholder="C:\\path\\to\\project" />
          <SecondaryButton onClick={() => void browse()} disabled={busy} icon={<FolderOpen size={14} />}>参照</SecondaryButton>
        </div>
      </SettingRow>
      <div className="border-t border-background px-5 py-4">
        <div className="flex flex-wrap gap-2">
          <PrimaryButton onClick={() => void launch()} disabled={busy} loading={busy} icon={<Terminal size={15} />}>
            {running ? "Claude Codeを追加起動" : "Claude Codeを起動"}
          </PrimaryButton>
          {running && (
            <SecondaryButton onClick={() => void stop()} disabled={busy} loading={busy} icon={<Square size={14} />}>すべてのClaude Code連携を停止</SecondaryButton>
          )}
        </div>
      </div>
    </SectionCard>
  );
}

function wslPathToUnc(distro: string, path: string): string {
  const suffix = path.replace(/^\/+/, "").replace(/\//g, "\\");
  return `\\\\wsl.localhost\\${distro}${suffix ? `\\${suffix}` : "\\"}`;
}

function uncToWslPath(path: string, distro: string): string | null {
  const normalized = path.replace(/\//g, "\\");
  const lower = normalized.toLowerCase();
  const prefixes = [
    `\\\\wsl.localhost\\${distro.toLowerCase()}`,
    `\\\\wsl$\\${distro.toLowerCase()}`,
  ];
  const prefix = prefixes.find((candidate) => lower === candidate || lower.startsWith(`${candidate}\\`));
  if (!prefix) return null;
  const suffix = normalized.slice(prefix.length).replace(/^\\+/, "");
  return suffix ? `/${suffix.replace(/\\/g, "/")}` : "/";
}

function CodexStopConfirmDialog({
  onCancel,
  onConfirm,
}: {
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useLang();
  const cancelForKeyboard = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Enter" || event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onCancel();
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-ink/25 px-5"
      role="dialog"
      aria-modal="true"
      aria-labelledby="codex-stop-confirm-title"
      aria-describedby="codex-stop-confirm-description"
      onClick={(event) => {
        if (event.target === event.currentTarget) onCancel();
      }}
      onKeyDown={cancelForKeyboard}
    >
      <div className="w-full max-w-lg rounded-2xl bg-background p-6 shadow-2xl ring-1 ring-ink/10">
        <h2 id="codex-stop-confirm-title" className="text-base font-medium text-ink">
          {t("settings.codex.stop_confirm.title")}
        </h2>
        <p id="codex-stop-confirm-description" className="mt-3 whitespace-pre-line text-sm leading-6 text-muted">
          {t("settings.codex.stop_confirm.description")}
        </p>
        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            autoFocus
            onClick={onCancel}
            className="btn-neu rounded-full px-4 py-2 text-sm font-medium text-ink"
          >
            {t("settings.codex.stop_confirm.cancel")}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            className="rounded-full bg-accent px-4 py-2 text-sm font-medium text-white shadow-sm"
          >
            {t("settings.codex.stop_confirm.confirm")}
          </button>
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
