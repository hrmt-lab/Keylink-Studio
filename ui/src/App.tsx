import { useEffect, useRef, useState, useCallback } from "react";
import { Sidebar } from "./components/Sidebar";
import Rules from "./pages/Rules";
import Actions from "./pages/Actions";
import TimeSync from "./pages/TimeSync";
import AiUsage from "./pages/AiUsage";
import KeymapViewer from "./pages/KeymapViewer";
import Devices from "./pages/Devices";
import Settings from "./pages/Settings";
import {
  getConfig,
  getStatus,
  getLogEntries,
  onStatusUpdate,
  onLogAdded,
  probeStudioDevices,
} from "./api";
import type {
  AppConfig,
  MonitorStatus,
  LogEntry,
  Page,
  StudioDeviceStatus,
  StudioKeymapSnapshot,
} from "./types";
import { LangProvider, useLang } from "./i18n";

const MAX_LOGS = 200;
const STUDIO_STARTUP_RETRY_DELAYS_MS = [750, 1250] as const;

function isTransientStudioResult(device: StudioDeviceStatus): boolean {
  return device.error_code === "open_failed" ||
    device.error_code === "rpc_failed" ||
    device.error_code === "rpc_timeout";
}

function mergeStartupStudioResults(
  current: StudioDeviceStatus[],
  incoming: StudioDeviceStatus[],
): StudioDeviceStatus[] {
  const merged = new Map(current.map((device) => [device.id, device]));
  for (const device of incoming) {
    const previous = merged.get(device.id);
    if (previous?.rpc_status === "ok" && isTransientStudioResult(device)) continue;
    merged.set(device.id, device);
  }
  return [...merged.values()];
}

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

type PendingNavigationAction = "save" | "discard";

interface KeymapNavigationGuard {
  hasUnsaved: () => Promise<boolean>;
  canLeave: () => boolean;
  saveAndLeave: () => Promise<boolean>;
  discardAndLeave: () => Promise<boolean>;
}

function sortLogsNewestFirst(entries: LogEntry[]) {
  return [...entries].sort((a, b) => {
    if (a.timestamp_ms !== b.timestamp_ms) return b.timestamp_ms - a.timestamp_ms;
    return b.id - a.id;
  });
}

export default function App() {
  return (
    <LangProvider>
      <AppInner />
    </LangProvider>
  );
}

function AppInner() {
  const [page, setPage] = useState<Page>("devices");
  const keymapNavigationGuardRef = useRef<KeymapNavigationGuard | null>(null);
  const [pendingNavigation, setPendingNavigation] = useState<Page | null>(null);
  const [pendingNavigationCanLeave, setPendingNavigationCanLeave] = useState(false);
  const [pendingNavigationAction, setPendingNavigationAction] =
    useState<PendingNavigationAction | null>(null);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<MonitorStatus>({
    running: false,
    connected_devices: 0,
    connected_device_names: [],
    host_link_devices: [],
    current_layer: null,
    current_rule: null,
    last_error: null,
    ai_usage: [],
    device_battery: [],
    device_layers: [],
  });
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [studioDevices, setStudioDevices] = useState<StudioDeviceStatus[]>([]);
  const [studioScanning, setStudioScanning] = useState(false);
  const [studioError, setStudioError] = useState<string | null>(null);
  const [keymapSnapshotsByDeviceId, setKeymapSnapshotsByDeviceId] = useState<Record<string, StudioKeymapSnapshot>>({});
  const [loading, setLoading] = useState(true);
  const studioScanPromiseRef = useRef<Promise<StudioDeviceStatus[]> | null>(null);
  const startupStudioScanStartedRef = useRef(false);

  const addLog = useCallback((entry: LogEntry) => {
    setLogs((prev) => {
      if (prev.some((existing) => existing.id === entry.id)) return prev;
      const next = sortLogsNewestFirst([...prev, entry]);
      return next.length > MAX_LOGS ? next.slice(0, MAX_LOGS) : next;
    });
  }, []);

  const scanStudioDevices = useCallback((startup: boolean) => {
    if (studioScanPromiseRef.current) return studioScanPromiseRef.current;

    setStudioScanning(true);
    setStudioError(null);
    const scan = (async () => {
      let devices: StudioDeviceStatus[] = [];
      const attempts = startup ? STUDIO_STARTUP_RETRY_DELAYS_MS.length + 1 : 1;
      for (let attempt = 0; attempt < attempts; attempt += 1) {
        if (attempt > 0) await wait(STUDIO_STARTUP_RETRY_DELAYS_MS[attempt - 1]);
        let probed: StudioDeviceStatus[];
        try {
          probed = await probeStudioDevices();
        } catch (error) {
          if (!startup || attempt === attempts - 1) throw error;
          continue;
        }
        devices = startup ? mergeStartupStudioResults(devices, probed) : probed;
        const hasTransientResult = devices.length === 0 || devices.some(isTransientStudioResult);
        if (!hasTransientResult) break;
      }

      const ids = new Set(devices.map((device) => device.id));
      setStudioDevices(devices);
      setKeymapSnapshotsByDeviceId((current) => {
        const next: Record<string, StudioKeymapSnapshot> = {};
        for (const [id, snapshot] of Object.entries(current)) {
          if (ids.has(id)) next[id] = snapshot;
        }
        return next;
      });
      return devices;
    })();
    studioScanPromiseRef.current = scan;
    void scan.catch((e) => {
      setStudioError(String(e));
    }).finally(() => {
      studioScanPromiseRef.current = null;
      setStudioScanning(false);
    });
    return scan;
  }, []);

  const refreshStudioDevices = useCallback(
    () => scanStudioDevices(false),
    [scanStudioDevices],
  );

  useEffect(() => {
    let cancelled = false;
    let unlisten1: (() => void) | null = null;
    let unlisten2: (() => void) | null = null;

    (async () => {
      try {
        const [cfg, st, logEntries] = await Promise.all([
          getConfig(),
          getStatus(),
          getLogEntries(),
        ]);
        if (cancelled) return;
        setConfig(cfg);
        setStatus(st);
        setLogs(sortLogsNewestFirst(logEntries).slice(0, MAX_LOGS));
      } finally {
        if (!cancelled) setLoading(false);
      }

      const statusUnlisten = await onStatusUpdate(setStatus);
      if (cancelled) {
        statusUnlisten();
        return;
      }
      unlisten1 = statusUnlisten;

      const logUnlisten = await onLogAdded(addLog);
      if (cancelled) {
        logUnlisten();
        return;
      }
      unlisten2 = logUnlisten;
    })();

    return () => {
      cancelled = true;
      unlisten1?.();
      unlisten2?.();
    };
  }, [addLog]);

  useEffect(() => {
    if (loading || startupStudioScanStartedRef.current) return;
    startupStudioScanStartedRef.current = true;
    void scanStudioDevices(true).catch(() => {});
  }, [loading, scanStudioDevices]);

  useEffect(() => {
    if (!pendingNavigation) return undefined;
    const update = () => setPendingNavigationCanLeave(keymapNavigationGuardRef.current?.canLeave() ?? false);
    update();
    const timer = window.setInterval(update, 250);
    return () => window.clearInterval(timer);
  }, [pendingNavigation]);

  const { t } = useLang();
  const registerKeymapNavigationGuard = useCallback((guard: KeymapNavigationGuard | null) => {
    keymapNavigationGuardRef.current = guard;
  }, []);

  const requestNavigate = useCallback(async (nextPage: Page) => {
    if (nextPage === page) return;
    const guard = page === "keymap_viewer" ? keymapNavigationGuardRef.current : null;
    if (guard && await guard.hasUnsaved()) {
      setPendingNavigationCanLeave(guard.canLeave());
      setPendingNavigation(nextPage);
      return;
    }
    setPage(nextPage);
  }, [page]);

  const closePendingNavigation = useCallback(() => {
    if (pendingNavigationAction) return;
    setPendingNavigation(null);
    setPendingNavigationCanLeave(false);
  }, [pendingNavigationAction]);

  const completePendingNavigation = useCallback(async (action: PendingNavigationAction) => {
    if (!pendingNavigation || pendingNavigationAction) return;
    const guard = keymapNavigationGuardRef.current;
    if (!guard || !guard.canLeave()) return;

    setPendingNavigationAction(action);
    try {
      const ok = action === "save"
        ? await guard.saveAndLeave()
        : await guard.discardAndLeave();
      if (!ok) return;
      setPage(pendingNavigation);
      setPendingNavigation(null);
      setPendingNavigationCanLeave(false);
    } finally {
      setPendingNavigationAction(null);
    }
  }, [pendingNavigation, pendingNavigationAction]);

  if (loading || !config) {
    return (
      <div className="flex h-full items-center justify-center bg-background">
        <div className="flex flex-col items-center gap-3">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-border border-t-accent" />
          <span className="text-sm text-muted">{t("app.loading")}</span>
        </div>
      </div>
    );
  }

  const updateConfig = (c: AppConfig) => setConfig(c);

  return (
    <div className="flex h-full overflow-hidden bg-background">
      <Sidebar
        currentPage={page}
        onNavigate={(nextPage) => void requestNavigate(nextPage)}
        status={status}
        studioDevices={studioDevices}
      />
      <main className="app-main-scroll flex-1 overflow-y-auto">
        {page === "rules" && (
          <Rules config={config} setConfig={updateConfig} status={status} />
        )}
        {page === "actions" && (
          <Actions config={config} setConfig={updateConfig} status={status} />
        )}
        {page === "timesync" && (
          <TimeSync config={config} setConfig={updateConfig} />
        )}
        {page === "ai_usage" && (
          <AiUsage config={config} setConfig={updateConfig} status={status} />
        )}
        {page === "keymap_viewer" && (
          <KeymapViewer
            studioDevices={studioDevices}
            studioScanning={studioScanning}
            studioError={studioError}
            refreshStudioDevices={refreshStudioDevices}
            snapshotsByDeviceId={keymapSnapshotsByDeviceId}
            setSnapshotsByDeviceId={setKeymapSnapshotsByDeviceId}
            status={status}
            onRegisterNavigationGuard={registerKeymapNavigationGuard}
          />
        )}
        {page === "devices" && (
          <Devices
            studioDevices={studioDevices}
            studioScanning={studioScanning}
            studioError={studioError}
            refreshStudioDevices={refreshStudioDevices}
            status={status}
            logs={logs}
          />
        )}
        {page === "settings" && (
          <Settings config={config} setConfig={updateConfig} />
        )}
      </main>
      {pendingNavigation && (
        <UnsavedNavigationDialog
          busyAction={pendingNavigationAction}
          canLeave={pendingNavigationCanLeave}
          onSave={() => void completePendingNavigation("save")}
          onDiscard={() => void completePendingNavigation("discard")}
          onCancel={closePendingNavigation}
        />
      )}
    </div>
  );
}

function UnsavedNavigationDialog({
  busyAction,
  canLeave,
  onSave,
  onDiscard,
  onCancel,
}: {
  busyAction: PendingNavigationAction | null;
  canLeave: boolean;
  onSave: () => void;
  onDiscard: () => void;
  onCancel: () => void;
}) {
  const { t } = useLang();
  const busy = busyAction !== null;
  const actionDisabled = busy || !canLeave;
  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/20 px-4">
      <div className="w-[min(440px,100%)] rounded-card bg-surface p-5 shadow-neu-up ring-1 ring-border">
        <div>
          <h2 className="text-base font-medium text-ink">
            {t("keymap.edit.leave_unsaved_title")}
          </h2>
          <p className="mt-2 text-sm leading-6 text-muted">
            {canLeave
              ? t("keymap.edit.leave_unsaved_body")
              : t("keymap.edit.leave_unsaved_busy")}
          </p>
        </div>
        <div className="mt-5 flex flex-wrap justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            className="rounded-pill bg-background px-3 py-2 text-sm font-medium text-muted ring-1 ring-border disabled:opacity-50"
          >
            {t("keymap.edit.leave_cancel")}
          </button>
          <button
            type="button"
            onClick={onDiscard}
            disabled={actionDisabled}
            className="rounded-pill bg-background px-3 py-2 text-sm font-medium text-red-700 ring-1 ring-red-100 disabled:opacity-50"
          >
            {busyAction === "discard"
              ? t("keymap.edit.discarding")
              : t("keymap.edit.leave_discard")}
          </button>
          <button
            type="button"
            onClick={onSave}
            disabled={actionDisabled}
            className="rounded-pill bg-accent px-3 py-2 text-sm font-medium text-white disabled:opacity-50"
          >
            {busyAction === "save"
              ? t("keymap.edit.saving")
              : t("keymap.edit.leave_save")}
          </button>
        </div>
      </div>
    </div>
  );
}
