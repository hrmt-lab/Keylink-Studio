import { useEffect, useState, useCallback } from "react";
import { Sidebar } from "./components/Sidebar";
import Dashboard from "./pages/Dashboard";
import Rules from "./pages/Rules";
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

export default function App() {
  return (
    <LangProvider>
      <AppInner />
    </LangProvider>
  );
}

function AppInner() {
  const [page, setPage] = useState<Page>("dashboard");
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
  });
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [studioDevices, setStudioDevices] = useState<StudioDeviceStatus[]>([]);
  const [studioScanning, setStudioScanning] = useState(false);
  const [studioError, setStudioError] = useState<string | null>(null);
  const [keymapSnapshotsByDeviceId, setKeymapSnapshotsByDeviceId] = useState<Record<string, StudioKeymapSnapshot>>({});
  const [loading, setLoading] = useState(true);

  const addLog = useCallback((entry: LogEntry) => {
    setLogs((prev) => {
      const next = [...prev, entry];
      return next.length > MAX_LOGS ? next.slice(next.length - MAX_LOGS) : next;
    });
  }, []);

  const refreshStudioDevices = useCallback(async () => {
    setStudioScanning(true);
    setStudioError(null);
    try {
      const devices = await probeStudioDevices();
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
    } catch (e) {
      setStudioError(String(e));
      throw e;
    } finally {
      setStudioScanning(false);
    }
  }, []);

  useEffect(() => {
    let unlisten1: (() => void) | null = null;
    let unlisten2: (() => void) | null = null;

    (async () => {
      try {
        const [cfg, st, logEntries] = await Promise.all([
          getConfig(),
          getStatus(),
          getLogEntries(),
        ]);
        setConfig(cfg);
        setStatus(st);
        setLogs(logEntries);
      } finally {
        setLoading(false);
      }

      unlisten1 = await onStatusUpdate(setStatus);
      unlisten2 = await onLogAdded(addLog);
    })();

    return () => {
      unlisten1?.();
      unlisten2?.();
    };
  }, [addLog]);

  useEffect(() => {
    void refreshStudioDevices().catch(() => {});
  }, [refreshStudioDevices]);

  const { t } = useLang();

  if (loading || !config) {
    return (
      <div className="flex h-full items-center justify-center bg-background">
        <div className="flex flex-col items-center gap-3">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-border border-t-primary" />
          <span className="text-sm text-gray-500">{t("app.loading")}</span>
        </div>
      </div>
    );
  }

  const updateConfig = (c: AppConfig) => setConfig(c);

  return (
    <div className="flex h-full overflow-hidden bg-background">
      <Sidebar currentPage={page} onNavigate={setPage} status={status} />
      <main className="flex-1 overflow-y-auto">
        {page === "dashboard" && (
          <Dashboard
            config={config}
            setConfig={updateConfig}
            status={status}
            logs={logs}
          />
        )}
        {page === "rules" && (
          <Rules config={config} setConfig={updateConfig} status={status} />
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
          />
        )}
        {page === "devices" && (
          <Devices
            studioDevices={studioDevices}
            studioScanning={studioScanning}
            studioError={studioError}
            refreshStudioDevices={refreshStudioDevices}
          />
        )}
        {page === "settings" && (
          <Settings config={config} setConfig={updateConfig} />
        )}
      </main>
    </div>
  );
}
