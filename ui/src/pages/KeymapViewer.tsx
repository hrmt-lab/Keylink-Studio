import { useCallback, useEffect, useMemo, useState, type ReactNode, type Dispatch, type SetStateAction } from "react";
import { AlertCircle, BarChart3, Keyboard, Lock, RefreshCw, XCircle } from "lucide-react";
import { getKeyStats, onKeyStatsUpdated, readStudioKeymap } from "../api";
import { KeymapCanvas } from "../components/KeymapCanvas";
import { useLang, type TranslationKey } from "../i18n";
import type {
  KeyStatsSummary,
  MonitorStatus,
  StatsPeriod,
  StudioBinding,
  StudioDeviceStatus,
  StudioKeymapSnapshot,
  StudioLayer,
} from "../types";

interface KeymapViewerProps {
  studioDevices: StudioDeviceStatus[];
  studioScanning: boolean;
  studioError: string | null;
  refreshStudioDevices: () => Promise<StudioDeviceStatus[]>;
  snapshotsByDeviceId: Record<string, StudioKeymapSnapshot>;
  setSnapshotsByDeviceId: Dispatch<SetStateAction<Record<string, StudioKeymapSnapshot>>>;
  status: MonitorStatus;
}

/** Case-insensitive serial match between a Studio device and Host Link data. */
function serialsMatch(a: string | null, b: string | null): boolean {
  if (!a || !b) return false;
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

export default function KeymapViewer({
  studioDevices,
  studioScanning,
  studioError,
  refreshStudioDevices,
  snapshotsByDeviceId,
  setSnapshotsByDeviceId,
  status,
}: KeymapViewerProps) {
  const { t } = useLang();
  const [selectedId, setSelectedId] = useState<string>("");
  const [activeLayer, setActiveLayer] = useState(0);
  const [viewMode, setViewMode] = useState<"keymap" | "heatmap">("keymap");
  const [reading, setReading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const devices = useMemo(
    () => studioDevices.filter((device) => device.rpc_status === "ok"),
    [studioDevices]
  );
  const selected = useMemo(
    () => devices.find((device) => device.id === selectedId) ?? null,
    [devices, selectedId]
  );
  const snapshot = selectedId ? snapshotsByDeviceId[selectedId] ?? null : null;
  const layer = snapshot?.layers[activeLayer] ?? null;

  // Best-effort correlation with Host Link data via the USB serial number.
  const reportedLayer = useMemo(
    () =>
      selected
        ? status.device_layers.find((entry) =>
            serialsMatch(entry.serial_number, selected.serial_number)
          ) ?? null
        : null,
    [selected, status.device_layers]
  );
  const statsUid = useMemo(
    () =>
      selected
        ? status.host_link_devices.find((device) =>
            serialsMatch(device.serial_number, selected.serial_number)
          )?.device_uid_hash ?? null
        : null,
    [selected, status.host_link_devices]
  );

  useEffect(() => {
    const ids = new Set(devices.map((device) => device.id));
    setSelectedId((current) => current && ids.has(current) ? current : devices[0]?.id ?? "");
  }, [devices]);

  useEffect(() => { setActiveLayer(0); }, [selectedId]);

  // Follow keyboard-side layer changes (LAYER_STATE uplink): switch the
  // displayed layer too, not just the live ring. Manual tab clicks still
  // work until the keyboard next changes layers.
  const reportedLayerIndex = reportedLayer?.active_layer ?? null;
  useEffect(() => {
    if (reportedLayerIndex === null || !snapshot) return;
    const index = snapshot.layers.findIndex((item) => item.index === reportedLayerIndex);
    if (index >= 0) setActiveLayer(index);
  }, [reportedLayerIndex, snapshot]);

  const readDevice = useCallback(async (device: StudioDeviceStatus) => {
    setReading(true);
    setError(null);
    try {
      const result = await readStudioKeymap(device.id);
      setSnapshotsByDeviceId((current) => ({ ...current, [device.id]: result }));
      setActiveLayer(0);
    } catch (e) {
      setError(errorLabel(String(e), t));
    } finally {
      setReading(false);
    }
  }, [setSnapshotsByDeviceId, t]);

  useEffect(() => {
    if (!selected || snapshot || studioScanning || reading || selected.keymap_viewer_status !== "available") return;
    void readDevice(selected);
  }, [readDevice, reading, selected, snapshot, studioScanning]);

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const refreshed = await refreshStudioDevices();
      const nextSelected = refreshed.find((device) => device.id === selectedId)
        ?? refreshed.find((device) => device.rpc_status === "ok")
        ?? null;
      if (nextSelected?.id && nextSelected.id !== selectedId) {
        setSelectedId(nextSelected.id);
      }
      if (nextSelected?.keymap_viewer_status === "available") {
        await readDevice(nextSelected);
      }
    } catch (e) {
      setError(String(e));
    }
  }, [readDevice, refreshStudioDevices, selectedId]);

  const busy = studioScanning || reading;
  const viewerAvailable = selected?.keymap_viewer_status === "available";
  const selectedLocked = selected?.keymap_viewer_status === "locked" || selected?.lock_state === "locked";

  return (
    <div className="p-6 w-full space-y-5">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold text-gray-800">{t("keymap.title")}</h1>
          <p className="mt-0.5 text-sm text-gray-500">{t("keymap.subtitle")}</p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={refresh}
            disabled={busy}
            className="flex items-center gap-2 rounded-lg border border-border bg-white px-4 py-2.5 text-sm font-medium text-gray-700 hover:bg-panel disabled:opacity-60 transition-colors"
          >
            <RefreshCw size={15} className={busy ? "animate-spin" : ""} />
            {t("keymap.refresh")}
          </button>
        </div>
      </div>

      {(error || studioError) && <Notice>{error ?? studioError}</Notice>}

      <div className="space-y-5">
        <section className="rounded-xl bg-white shadow-card ring-1 ring-border p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold text-gray-800">{t("keymap.devices")}</h2>
            <span className="text-xs text-gray-400">{devices.length}</span>
          </div>

          {devices.length === 0 ? (
            <div className="rounded-lg bg-background px-4 py-8 text-center text-sm text-gray-400">
              {studioScanning ? t("keymap.scanning") : t("keymap.no_devices")}
            </div>
          ) : (
            <div className="flex max-h-36 gap-2 overflow-x-auto overflow-y-auto pb-1">
              {devices.map((device) => (
                <button
                  key={device.id}
                  onClick={() => setSelectedId(device.id)}
                  className={`min-w-64 max-w-72 rounded-lg px-3 py-3 text-left ring-1 transition-colors ${
                    selectedId === device.id ? "bg-primary/5 ring-primary/30" : "bg-background ring-border hover:bg-panel"
                  }`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="truncate text-sm font-medium text-gray-800">{device.display_name}</span>
                    <StudioStatusBadge device={device} />
                  </div>
                  <div className="mt-1 truncate font-mono text-[11px] text-gray-400">{device.port_name}</div>
                  <div className="mt-1 text-[11px] text-gray-400">{t("keymap.connection_usb_serial")}</div>
                </button>
              ))}
            </div>
          )}
        </section>

        <section className="w-full overflow-hidden rounded-xl bg-white shadow-card ring-1 ring-border p-5 space-y-4">
          {!selected ? (
            <EmptyState icon={<Keyboard size={32} />} title={t("keymap.select_device")} />
          ) : selectedLocked ? (
            <EmptyState icon={<Lock size={32} />} title={t("keymap.locked_title")} body={t("keymap.locked_body")} />
          ) : !viewerAvailable ? (
            <EmptyState icon={<XCircle size={32} />} title={t("keymap.unsupported_title")} body={t("keymap.unsupported_body")} />
          ) : !snapshot ? (
            <EmptyState icon={<Keyboard size={32} />} title={reading ? t("keymap.reading") : t("keymap.ready_title")} body={reading ? undefined : t("keymap.ready_body")} />
          ) : (
            <>
              <div className="flex items-center gap-1 border-b border-border/60 pb-3">
                <ViewTab
                  active={viewMode === "keymap"}
                  onClick={() => setViewMode("keymap")}
                  icon={<Keyboard size={13} />}
                  label={t("keymap.view.keymap")}
                />
                <ViewTab
                  active={viewMode === "heatmap"}
                  onClick={() => setViewMode("heatmap")}
                  icon={<BarChart3 size={13} />}
                  label={t("keymap.view.heatmap")}
                />
              </div>
              {viewMode === "keymap" ? (
                <KeymapContent
                  snapshot={snapshot}
                  activeLayer={activeLayer}
                  setActiveLayer={setActiveLayer}
                  layer={layer}
                  reportedLayerIndex={reportedLayerIndex}
                />
              ) : (
                <HeatmapContent snapshot={snapshot} statsUid={statsUid} />
              )}
            </>
          )}
        </section>
      </div>
    </div>
  );
}

function ViewTab({ active, onClick, icon, label }: {
  active: boolean;
  onClick: () => void;
  icon: ReactNode;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-medium transition-colors ${
        active ? "bg-primary/10 text-primary" : "text-gray-500 hover:bg-background"
      }`}
    >
      {icon}
      {label}
    </button>
  );
}

function KeymapContent({ snapshot, activeLayer, setActiveLayer, layer, reportedLayerIndex }: {
  snapshot: StudioKeymapSnapshot;
  activeLayer: number;
  setActiveLayer: (value: number) => void;
  layer: StudioLayer | null;
  /** Layer the keyboard itself reports as active (LAYER_STATE uplink), or null. */
  reportedLayerIndex: number | null;
}) {
  const { t } = useLang();
  const bindingsByPosition = useMemo(() => {
    const map = new Map<number, StudioBinding>();
    if (layer) for (const binding of layer.bindings) map.set(binding.position, binding);
    return map;
  }, [layer]);

  return (
    <div className="min-w-0 space-y-4">
      <div>
        <div className="text-sm font-semibold text-gray-800">{snapshot.device_name}</div>
      </div>
      <div className="flex flex-wrap gap-2">
        {snapshot.layers.map((item, index) => {
          const live = reportedLayerIndex !== null && item.index === reportedLayerIndex;
          return (
            <button
              key={item.id}
              onClick={() => setActiveLayer(index)}
              title={live ? t("keymap.active_layer") : undefined}
              className={`relative rounded-lg px-3 py-1.5 text-sm font-medium ring-1 transition-colors ${
                activeLayer === index ? "bg-primary text-white ring-primary" : "bg-background text-gray-600 ring-border hover:bg-panel"
              } ${live ? "ring-2 ring-emerald-400" : ""}`}
            >
              {item.name}
              {live && (
                <span className="absolute -right-1 -top-1 h-2.5 w-2.5 rounded-full bg-emerald-400 ring-2 ring-white" />
              )}
            </button>
          );
        })}
      </div>
      {!layer || snapshot.selected_layout_keys.length === 0 ? (
        <EmptyState icon={<Keyboard size={32} />} title={t("keymap.empty_keymap_title")} body={t("keymap.empty_keymap_body")} />
      ) : (
        <KeymapCanvas
          keys={snapshot.selected_layout_keys}
          keyTitle={(key) => bindingsByPosition.get(key.position)?.full_label ?? "--"}
          keyContent={(key) => {
            const binding = bindingsByPosition.get(key.position);
            return (
              <>
                <div className="w-full truncate text-[11px] font-semibold leading-tight text-gray-800">
                  {binding?.primary_label ?? "--"}
                </div>
                {binding?.primary_label && (
                  <div className="absolute bottom-1 right-1 text-[9px] leading-none text-gray-500">
                    {`#${key.position}`}
                  </div>
                )}
              </>
            );
          }}
        />
      )}
    </div>
  );
}

function HeatmapContent({ snapshot, statsUid }: {
  snapshot: StudioKeymapSnapshot;
  statsUid: string | null;
}) {
  const { t } = useLang();
  const [period, setPeriod] = useState<StatsPeriod>("today");
  const [summary, setSummary] = useState<KeyStatsSummary | null>(null);
  const [statsError, setStatsError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!statsUid) return;
    try {
      setSummary(await getKeyStats(statsUid, period));
      setStatsError(null);
    } catch (e) {
      setStatsError(String(e));
    }
  }, [statsUid, period]);

  useEffect(() => {
    void load();
  }, [load]);

  // Live refresh while the keyboard keeps reporting stats.
  useEffect(() => {
    if (!statsUid) return;
    let unlisten: (() => void) | null = null;
    let disposed = false;
    void onKeyStatsUpdated((deviceKey) => {
      if (deviceKey === statsUid) void load();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [statsUid, load]);

  const counts = useMemo(() => {
    const map = new Map<number, number>();
    for (const entry of summary?.per_position ?? []) map.set(entry.position, entry.count);
    return map;
  }, [summary]);
  const maxCount = useMemo(
    () => Math.max(1, ...Array.from(counts.values())),
    [counts]
  );
  const baseLayer = snapshot.layers[0] ?? null;
  const labelByPosition = useMemo(() => {
    const map = new Map<number, string>();
    if (baseLayer) {
      for (const binding of baseLayer.bindings) {
        if (binding.primary_label) map.set(binding.position, binding.primary_label);
      }
    }
    return map;
  }, [baseLayer]);

  const topKeys = useMemo(
    () =>
      [...(summary?.per_position ?? [])]
        .sort((a, b) => b.count - a.count)
        .slice(0, 5),
    [summary]
  );

  const balance = useMemo(() => {
    const keys = snapshot.selected_layout_keys;
    if (keys.length === 0 || counts.size === 0) return null;
    const minX = Math.min(...keys.map((k) => k.x));
    const maxX = Math.max(...keys.map((k) => k.x + Math.abs(k.width)));
    const mid = (minX + maxX) / 2;
    let left = 0;
    let right = 0;
    for (const key of keys) {
      const count = counts.get(key.position) ?? 0;
      if (key.x + Math.abs(key.width) / 2 < mid) left += count;
      else right += count;
    }
    const total = left + right;
    if (total === 0) return null;
    return {
      left: Math.round((left / total) * 100),
      right: Math.round((right / total) * 100),
    };
  }, [snapshot.selected_layout_keys, counts]);

  if (!statsUid) {
    return (
      <EmptyState
        icon={<BarChart3 size={32} />}
        title={t("stats.no_link")}
        body={t("stats.no_link.hint")}
      />
    );
  }

  const periods: { value: StatsPeriod; key: TranslationKey }[] = [
    { value: "today", key: "stats.period.today" },
    { value: "last7days", key: "stats.period.last7days" },
    { value: "all", key: "stats.period.all" },
  ];

  return (
    <div className="min-w-0 space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-1">
          {periods.map((item) => (
            <button
              key={item.value}
              onClick={() => setPeriod(item.value)}
              className={`rounded-lg px-3 py-1.5 text-sm font-medium ring-1 transition-colors ${
                period === item.value
                  ? "bg-primary text-white ring-primary"
                  : "bg-background text-gray-600 ring-border hover:bg-panel"
              }`}
            >
              {t(item.key)}
            </button>
          ))}
        </div>
        <div className="flex flex-wrap items-center gap-4 text-sm text-gray-600">
          <span>
            {t("stats.total")}:{" "}
            <span className="font-semibold text-gray-800">
              {(summary?.total ?? 0).toLocaleString()}
            </span>
          </span>
          {balance && (
            <span>
              {t("stats.balance")}:{" "}
              <span className="font-semibold text-gray-800">
                {balance.left}% / {balance.right}%
              </span>
            </span>
          )}
        </div>
      </div>

      {statsError && <Notice>{statsError}</Notice>}

      {summary && summary.total === 0 && (
        <p className="text-sm text-gray-400">{t("stats.no_data")}</p>
      )}

      {snapshot.selected_layout_keys.length === 0 ? (
        <EmptyState icon={<Keyboard size={32} />} title={t("keymap.empty_keymap_title")} body={t("keymap.empty_keymap_body")} />
      ) : (
        <KeymapCanvas
          keys={snapshot.selected_layout_keys}
          keyTitle={(key) => {
            const label = labelByPosition.get(key.position) ?? `#${key.position}`;
            const count = counts.get(key.position) ?? 0;
            return `${label}: ${count.toLocaleString()}`;
          }}
          keyStyle={(key) => {
            const count = counts.get(key.position) ?? 0;
            return count > 0 ? { backgroundColor: heatColor(count / maxCount) } : undefined;
          }}
          keyContent={(key) => {
            const count = counts.get(key.position) ?? 0;
            return (
              <>
                <div className="w-full truncate text-[10px] font-medium leading-tight text-gray-700">
                  {labelByPosition.get(key.position) ?? ""}
                </div>
                <div className="w-full truncate text-[10px] font-semibold leading-tight text-gray-900">
                  {count > 0 ? count.toLocaleString() : ""}
                </div>
              </>
            );
          }}
        />
      )}

      {topKeys.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 text-xs text-gray-500">
          <span className="font-medium uppercase tracking-wide text-gray-400">
            {t("stats.top")}
          </span>
          {topKeys.map((entry) => (
            <span
              key={entry.position}
              className="inline-flex items-center gap-1 rounded-md bg-background px-2 py-0.5 ring-1 ring-border"
            >
              <span className="font-semibold text-gray-700">
                {labelByPosition.get(entry.position) ?? `#${entry.position}`}
              </span>
              <span className="font-mono text-gray-500">{entry.count.toLocaleString()}</span>
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

/** White → primary blue → orange → red, used for per-key heat coloring. */
function heatColor(ratio: number): string {
  const clamped = Math.max(0, Math.min(1, ratio));
  if (clamped < 0.5) {
    const a = clamped / 0.5;
    return `rgba(91, 112, 146, ${(0.15 + 0.45 * a).toFixed(3)})`;
  }
  if (clamped < 0.8) {
    const a = (clamped - 0.5) / 0.3;
    return `rgba(217, 119, 6, ${(0.4 + 0.35 * a).toFixed(3)})`;
  }
  const a = (clamped - 0.8) / 0.2;
  return `rgba(239, 68, 68, ${(0.55 + 0.35 * a).toFixed(3)})`;
}

function StudioStatusBadge({ device }: { device: StudioDeviceStatus }) {
  const { t } = useLang();
  const ok = device.keymap_viewer_status === "available";
  const locked = device.keymap_viewer_status === "locked";
  const className = ok
    ? "bg-emerald-100 text-emerald-700"
    : locked
      ? "bg-amber-100 text-amber-700"
      : "bg-gray-100 text-gray-500";
  return <span className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${className}`}>{t(`keymap.viewer.${device.keymap_viewer_status}` as TranslationKey)}</span>;
}

function EmptyState({ icon, title, body }: { icon: ReactNode; title: string; body?: string }) {
  return (
    <div className="flex min-h-[360px] flex-col items-center justify-center text-center">
      <div className="mb-3 text-gray-300">{icon}</div>
      <div className="text-sm font-semibold text-gray-700">{title}</div>
      {body && <div className="mt-1 max-w-md text-sm text-gray-400">{body}</div>}
    </div>
  );
}

function Notice({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-start gap-2.5 rounded-lg bg-red-50 px-4 py-3 text-sm text-red-700 ring-1 ring-red-200">
      <AlertCircle size={15} className="mt-0.5 flex-shrink-0" />
      <span>{children}</span>
    </div>
  );
}

function errorLabel(code: string, t: (key: TranslationKey, vars?: Record<string, string | number>) => string) {
  const key = `keymap.error.${code}` as TranslationKey;
  return t(key) || code;
}