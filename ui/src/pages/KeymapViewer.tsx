import { useCallback, useEffect, useMemo, useState, type ReactNode, type Dispatch, type SetStateAction } from "react";
import { AlertCircle, Keyboard, Lock, RefreshCw, XCircle } from "lucide-react";
import { readStudioKeymap } from "../api";
import { useLang, type TranslationKey } from "../i18n";
import type { StudioBinding, StudioDeviceStatus, StudioKeymapSnapshot, StudioLayer, StudioPhysicalKey } from "../types";

interface KeymapViewerProps {
  studioDevices: StudioDeviceStatus[];
  studioScanning: boolean;
  studioError: string | null;
  refreshStudioDevices: () => Promise<StudioDeviceStatus[]>;
  snapshotsByDeviceId: Record<string, StudioKeymapSnapshot>;
  setSnapshotsByDeviceId: Dispatch<SetStateAction<Record<string, StudioKeymapSnapshot>>>;
}

export default function KeymapViewer({
  studioDevices,
  studioScanning,
  studioError,
  refreshStudioDevices,
  snapshotsByDeviceId,
  setSnapshotsByDeviceId,
}: KeymapViewerProps) {
  const { t } = useLang();
  const [selectedId, setSelectedId] = useState<string>("");
  const [activeLayer, setActiveLayer] = useState(0);
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

  useEffect(() => {
    const ids = new Set(devices.map((device) => device.id));
    setSelectedId((current) => current && ids.has(current) ? current : devices[0]?.id ?? "");
  }, [devices]);

  useEffect(() => { setActiveLayer(0); }, [selectedId]);

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
            <KeymapContent snapshot={snapshot} activeLayer={activeLayer} setActiveLayer={setActiveLayer} layer={layer} />
          )}
        </section>
      </div>
    </div>
  );
}

function KeymapContent({ snapshot, activeLayer, setActiveLayer, layer }: {
  snapshot: StudioKeymapSnapshot;
  activeLayer: number;
  setActiveLayer: (value: number) => void;
  layer: StudioLayer | null;
}) {
  const { t } = useLang();
  return (
    <div className="min-w-0 space-y-4">
      <div>
        <div className="text-sm font-semibold text-gray-800">{snapshot.device_name}</div>
      </div>
      <div className="flex flex-wrap gap-2">
        {snapshot.layers.map((item, index) => (
          <button
            key={item.id}
            onClick={() => setActiveLayer(index)}
            className={`rounded-lg px-3 py-1.5 text-sm font-medium ring-1 transition-colors ${
              activeLayer === index ? "bg-primary text-white ring-primary" : "bg-background text-gray-600 ring-border hover:bg-panel"
            }`}
          >
            {item.name}
          </button>
        ))}
      </div>
      {!layer || snapshot.selected_layout_keys.length === 0 ? (
        <EmptyState icon={<Keyboard size={32} />} title={t("keymap.empty_keymap_title")} body={t("keymap.empty_keymap_body")} />
      ) : (
        <KeymapCanvas keys={snapshot.selected_layout_keys} layer={layer} />
      )}
    </div>
  );
}

function KeymapCanvas({ keys, layer }: { keys: StudioPhysicalKey[]; layer: StudioLayer }) {
  const metrics = useMemo(() => layoutMetrics(keys), [keys]);
  const bindingsByPosition = useMemo(() => {
    const map = new Map<number, StudioBinding>();
    for (const binding of layer.bindings) map.set(binding.position, binding);
    return map;
  }, [layer]);

  return (
    <div className="max-w-full overflow-x-auto overflow-y-hidden rounded-xl bg-background p-4 ring-1 ring-border">
      <div
        className="relative flex-shrink-0"
        style={{ width: metrics.width, height: metrics.height }}
      >
        {keys.map((key) => {
          const binding = bindingsByPosition.get(key.position);
          const x = (key.x - metrics.minX) * metrics.scale + metrics.padding;
          const y = (key.y - metrics.minY) * metrics.scale + metrics.padding;
          const width = Math.max(16, Math.abs(key.width) * metrics.scale);
          const height = Math.max(16, Math.abs(key.height) * metrics.scale);
          const originX = (key.rx - key.x) * metrics.scale;
          const originY = (key.ry - key.y) * metrics.scale;
          return (
            <div
              key={`${key.position}-${key.x}-${key.y}`}
              title={binding?.full_label ?? "--"}
              className="absolute flex flex-col items-center justify-center rounded-lg border border-border bg-white px-1.5 text-center shadow-sm ring-1 ring-white/70"
              style={{
                left: x,
                top: y,
                width,
                height,
                transform: key.r ? `rotate(${key.r}deg)` : undefined,
                transformOrigin: `${originX}px ${originY}px`,
              }}
            >
              <div className="w-full truncate text-[11px] font-semibold leading-tight text-gray-800">
                {binding?.primary_label ?? "--"}
              </div>
              {binding?.primary_label && (
                <div className="absolute bottom-1 right-1 text-[9px] leading-none text-gray-500">
                  {`#${key.position}`}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function layoutMetrics(keys: StudioPhysicalKey[]) {
  const padding = 24;
  const rawMinX = Math.min(...keys.map((key) => key.x));
  const rawMinY = Math.min(...keys.map((key) => key.y));
  const rawMaxX = Math.max(...keys.map((key) => key.x + Math.abs(key.width)));
  const rawMaxY = Math.max(...keys.map((key) => key.y + Math.abs(key.height)));
  const rawWidth = Math.max(1, rawMaxX - rawMinX);
  const rawHeight = Math.max(1, rawMaxY - rawMinY);
  const maxWidth = 820;
  const scale = Math.min(1.2, maxWidth / rawWidth);
  return {
    minX: rawMinX,
    minY: rawMinY,
    scale,
    padding,
    width: rawWidth * scale + padding * 2,
    height: rawHeight * scale + padding * 2,
  };
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