import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode, type Dispatch, type SetStateAction } from "react";
import { Crosshair, AlertCircle, BarChart3, Keyboard, Lock, RefreshCw, XCircle, Pencil, Save, Trash2, LogOut, Search } from "lucide-react";
import {
  getKeyStats,
  onKeyPressEvent,
  onKeyStatsUpdated,
  readStudioKeymap,
  studioBeginEdit,
  studioDiscardChanges,
  studioEndEdit,
  studioHasUnsaved,
  studioKeyCatalog,
  studioSaveChanges,
  studioSetKey,
} from "../api";
import { KeymapCanvas } from "../components/KeymapCanvas";
import { useLang, type TranslationKey } from "../i18n";
import type {
  KeyPressEvent,
  EditBehavior,
  EditState,
  KeyCatalogEntry,
  KeyStatsSummary,
  MonitorStatus,
  StatsPeriod,
  StudioBinding,
  StudioDeviceStatus,
  StudioKeymapSnapshot,
  StudioLayer,
  StudioPhysicalKey,
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

function compareDeviceName(a: string, b: string): number {
  return a.localeCompare(b, undefined, { sensitivity: "base", numeric: true });
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
  const [viewMode, setViewMode] = useState<"keymap" | "heatmap" | "tester">("keymap");
  const [reading, setReading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editState, setEditState] = useState<EditState>({
    mode: "viewing",
    dirty: false,
    operation: "idle",
    problem: null,
  });
  const [editNotice, setEditNotice] = useState<"saved" | "discarded" | null>(null);
  const [catalog, setCatalog] = useState<KeyCatalogEntry[]>([]);
  const [picker, setPicker] = useState<{
    key: StudioPhysicalKey;
    layer: StudioLayer;
    rect: { left: number; top: number; width: number; height: number };
  } | null>(null);

  const devices = useMemo(
    () => studioDevices
      .filter((device) => device.rpc_status === "ok")
      .sort((a, b) => compareDeviceName(a.display_name, b.display_name)),
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
    if (editState.mode === "editing") return;
    if (reportedLayerIndex === null || !snapshot) return;
    const index = snapshot.layers.findIndex((item) => item.index === reportedLayerIndex);
    if (index >= 0) setActiveLayer(index);
  }, [editState.mode, reportedLayerIndex, snapshot]);

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
    if (editState.mode === "editing") {
      setError(errorLabel("port_busy", t));
      return;
    }
    setError(null);
    try {
      const refreshed = await refreshStudioDevices();
      const nextAvailable = refreshed
        .filter((device) => device.rpc_status === "ok")
        .sort((a, b) => compareDeviceName(a.display_name, b.display_name))[0] ?? null;
      const nextSelected = refreshed.find((device) => device.id === selectedId)
        ?? nextAvailable
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
  }, [editState.mode, readDevice, refreshStudioDevices, selectedId, t]);

  const mapEditProblem = useCallback((code: string): EditState["problem"] => {
    if (code === "save_result_unknown") return "save_unknown";
    if (code === "save_failed" || code === "save_not_supported" || code === "save_no_space") return "save_failed";
    if (code === "locked") return "locked_again";
    if (code === "disconnected" || code === "timeout") return "disconnected";
    return null;
  }, []);

  const beginEdit = useCallback(async (forceDiscard = false) => {
    if (!selected) return;
    setError(null);
    setEditNotice(null);
    setPicker(null);
    setEditState((current) => ({ ...current, operation: "setting", problem: null }));
    try {
      const result = await studioBeginEdit(selected.id, forceDiscard);
      setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: result }));
      if (catalog.length === 0) setCatalog(await studioKeyCatalog());
      const dirty = await studioHasUnsaved(selected.id).catch(() => false);
      setEditState({ mode: "editing", dirty, operation: "idle", problem: null });
    } catch (e) {
      const code = String(e);
      if (code === "unsaved_changes_exist") {
        const discard = window.confirm(t("keymap.edit.confirm_discard_switch"));
        if (discard) {
          try {
            const result = await studioBeginEdit(selected.id, true);
            setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: result }));
            if (catalog.length === 0) setCatalog(await studioKeyCatalog());
            const dirty = await studioHasUnsaved(selected.id).catch(() => false);
            setEditState({ mode: "editing", dirty, operation: "idle", problem: null });
          } catch (retryError) {
            const retryCode = String(retryError);
            const problem = mapEditProblem(retryCode);
            setEditState((current) => ({ ...current, operation: "idle", problem }));
            setError(errorLabel(retryCode, t));
          }
          return;
        }
      } else {
        const problem = mapEditProblem(code);
        if (problem) setEditState((current) => ({ ...current, operation: "idle", problem }));
        setError(errorLabel(code, t));
      }
      setEditState((current) => ({ ...current, operation: "idle" }));
    }
  }, [catalog.length, mapEditProblem, selected, setSnapshotsByDeviceId, t]);

  const saveEdit = useCallback(async () => {
    if (!selected) return;
    setEditNotice(null);
    setEditState((current) => ({ ...current, operation: "saving", problem: null }));
    try {
      await studioSaveChanges(selected.id);
      setEditState((current) => ({ ...current, dirty: false, operation: "idle", problem: null }));
      setEditNotice("saved");
    } catch (e) {
      const code = String(e);
      const problem = mapEditProblem(code) ?? "save_failed";
      setEditState((current) => ({ ...current, operation: "idle", problem }));
      setError(errorLabel(code, t));
    }
  }, [mapEditProblem, selected, t]);

  const discardEdit = useCallback(async () => {
    if (!selected) return false;
    setEditNotice(null);
    setEditState((current) => ({ ...current, operation: "discarding", problem: null }));
    try {
      const result = await studioDiscardChanges(selected.id);
      setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: result }));
      setPicker(null);
      setEditState((current) => ({ ...current, dirty: false, operation: "idle", problem: null }));
      setEditNotice("discarded");
      return true;
    } catch (e) {
      const code = String(e);
      setEditState((current) => ({ ...current, operation: "idle" }));
      setError(errorLabel(code, t));
      return false;
    }
  }, [selected, setSnapshotsByDeviceId, t]);

  const endEdit = useCallback(async () => {
    if (!selected) return;
    let dirty = editState.dirty;
    if (dirty) dirty = await studioHasUnsaved(selected.id).catch(() => true);
    if (dirty) {
      const discard = window.confirm(t("keymap.edit.confirm_discard_end"));
      if (!discard) return;
      const discarded = await discardEdit();
      if (!discarded) return;
    }
    setEditState((current) => ({ ...current, operation: "ending" }));
    try {
      await studioEndEdit(selected.id);
      setPicker(null);
      setEditState({ mode: "viewing", dirty: false, operation: "idle", problem: null });
    } catch (e) {
      const code = String(e);
      setEditState((current) => ({ ...current, operation: "idle" }));
      setError(errorLabel(code, t));
    }
  }, [discardEdit, editState.dirty, selected, t]);

  const setKey = useCallback(async (key: StudioPhysicalKey, targetLayer: StudioLayer, behavior: EditBehavior) => {
    if (!selected || editState.operation !== "idle") return;
    setEditNotice(null);
    setEditState((current) => ({ ...current, operation: "setting", problem: null }));
    try {
      const result = await studioSetKey(selected.id, targetLayer.id, key.position, behavior);
      setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: result }));
      setPicker(null);
      setEditState((current) => ({ ...current, dirty: true, operation: "idle", problem: null }));
    } catch (e) {
      const code = String(e);
      const problem = mapEditProblem(code);
      setEditState((current) => ({ ...current, operation: "idle", problem }));
      setError(errorLabel(code, t));
    }
  }, [editState.operation, mapEditProblem, selected, setSnapshotsByDeviceId, t]);

  useEffect(() => {
    setPicker(null);
  }, [selectedId, viewMode, activeLayer]);

  useEffect(() => {
    if (!editNotice) return undefined;
    const timer = window.setTimeout(() => setEditNotice(null), 3000);
    return () => window.clearTimeout(timer);
  }, [editNotice]);

  useEffect(() => {
    return () => {
      if (selectedId) void studioEndEdit(selectedId).catch(() => undefined);
    };
  }, [selectedId]);

  const busy = studioScanning || reading;
  const editing = editState.mode === "editing";
  const viewerAvailable = selected?.keymap_viewer_status === "available";
  const selectedLocked = selected?.keymap_viewer_status === "locked" || selected?.lock_state === "locked";

  return (
    <div className="p-6 w-full space-y-5">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-medium text-ink">{t("keymap.title")}</h1>
          <p className="mt-0.5 text-sm text-muted">{t("keymap.subtitle")}</p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={refresh}
            disabled={busy || editing}
            className="btn-neu flex items-center gap-2 rounded-full px-4 py-2.5 text-sm font-medium text-ink disabled:opacity-60"
          >
            <RefreshCw size={15} className={busy ? "animate-spin" : ""} />
            {t("keymap.refresh")}
          </button>
          {viewMode === "keymap" && viewerAvailable && !selectedLocked && (
            <button
              onClick={() => editing ? void endEdit() : void beginEdit(false)}
              disabled={busy || editState.operation !== "idle"}
              className={`btn-neu flex items-center gap-2 rounded-full px-4 py-2.5 text-sm font-medium disabled:opacity-60 ${
                editing ? "text-accent-deep" : "text-ink"
              }`}
            >
              <Pencil size={15} />
              {editing ? t("keymap.edit.on") : t("keymap.edit")}
            </button>
          )}
        </div>
      </div>

      {(error || studioError) && <Notice>{error ?? studioError}</Notice>}

      <div className="space-y-5">
        <section className="rounded-card bg-surface p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-medium text-ink">{t("keymap.devices")}</h2>
            <span className="text-xs text-faint font-mono">{devices.length}</span>
          </div>

          {devices.length === 0 ? (
            <div className="rounded-lg bg-background px-4 py-8 text-center text-sm text-faint">
              {studioScanning ? t("keymap.scanning") : t("keymap.no_devices")}
            </div>
          ) : (
            <div className="flex max-h-36 gap-2 overflow-x-auto overflow-y-auto p-1">
              {devices.map((device) => (
                <button
                  key={device.id}
                  onClick={() => setSelectedId(device.id)}
                  className={`min-w-64 max-w-72 rounded-pill px-3 py-3 text-left transition-colors ${
                    selectedId === device.id
                      ? "bg-plate shadow-neu-sel-in"
                      : "bg-surface ring-1 ring-border hover:ring-disabled"
                  }`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className={`truncate text-sm font-medium ${selectedId === device.id ? "text-accent-deep" : "text-ink"}`}>{device.display_name}</span>
                    <StudioStatusBadge device={device} />
                  </div>
                  <div className="mt-1 truncate font-mono text-[11px] text-muted">{device.port_name}</div>
                  <div className="mt-1 text-[11px] text-faint">{t("keymap.connection_usb_serial")}</div>
                </button>
              ))}
            </div>
          )}
        </section>

        <section className="w-full overflow-hidden rounded-card bg-surface p-5 space-y-4">
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
              <div className="flex items-center gap-1 border-b border-background pb-3">
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
                <ViewTab
                  active={viewMode === "tester"}
                  onClick={() => setViewMode("tester")}
                  icon={<Crosshair size={13} />}
                  label={t("keymap.view.tester")}
                />
              </div>
              {viewMode === "keymap" && (
                <KeymapContent
                  snapshot={snapshot}
                  activeLayer={activeLayer}
                  setActiveLayer={setActiveLayer}
                  layer={layer}
                  reportedLayerIndex={reportedLayerIndex}
                  onKeyClick={editing ? (key, element) => {
                    if (!layer || editState.operation !== "idle") return;
                    const rect = element.getBoundingClientRect();
                    setPicker({
                      key,
                      layer,
                      rect: { left: rect.left, top: rect.top, width: rect.width, height: rect.height },
                    });
                  } : undefined}
                />
              )}
              {viewMode === "heatmap" && (
                <HeatmapContent snapshot={snapshot} statsUid={statsUid} />
              )}
              {viewMode === "tester" && (
                <TesterContent
                  snapshot={snapshot}
                  activeLayer={activeLayer}
                  setActiveLayer={setActiveLayer}
                  layer={layer}
                  reportedLayerIndex={reportedLayerIndex}
                  statsUid={statsUid}
                />
              )}
            </>
          )}
        </section>
      </div>
      {editing && selected && (
        <EditBar
          dirty={editState.dirty}
          operation={editState.operation}
          problem={editState.problem}
          notice={editNotice}
          onSave={saveEdit}
          onDiscard={discardEdit}
          onEnd={endEdit}
        />
      )}
      {picker && (
        <BindingPicker
          catalog={catalog}
          layers={snapshot?.layers ?? []}
          rect={picker.rect}
          busy={editState.operation === "setting"}
          onClose={() => setPicker(null)}
          onSelect={(behavior) => void setKey(picker.key, picker.layer, behavior)}
        />
      )}
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
      className={`flex items-center gap-1.5 rounded-pill px-3 py-1.5 text-sm font-medium transition-colors ${
        active ? "bg-plate text-accent-deep shadow-neu-sel-in" : "text-muted hover:bg-background hover:text-ink"
      }`}
    >
      {icon}
      {label}
    </button>
  );
}

function KeymapContent({ snapshot, activeLayer, setActiveLayer, layer, reportedLayerIndex, keyStyle, marquee, onKeyClick }: {
  snapshot: StudioKeymapSnapshot;
  activeLayer: number;
  setActiveLayer: (value: number) => void;
  layer: StudioLayer | null;
  /** Layer the keyboard itself reports as active (LAYER_STATE uplink), or null. */
  reportedLayerIndex: number | null;
  keyStyle?: (key: StudioPhysicalKey) => CSSProperties | undefined;
  onKeyClick?: (key: StudioPhysicalKey, element: HTMLDivElement) => void;
  /** Optional element shown to the right of the layer tabs (tester typed-char marquee). */
  marquee?: ReactNode;
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
        <div className="text-sm font-medium text-ink">{snapshot.device_name}</div>
      </div>
      <div className="flex items-start gap-2">
        <div className="flex flex-wrap gap-2">
          {snapshot.layers.map((item, index) => {
            const live = reportedLayerIndex !== null && item.index === reportedLayerIndex;
            return (
              <button
                key={item.id}
                onClick={() => setActiveLayer(index)}
                title={live ? t("keymap.active_layer") : undefined}
                className={`relative rounded-pill px-3 py-1.5 text-sm font-medium ring-1 transition-colors ${
                  activeLayer === index ? "bg-plate text-accent ring-transparent shadow-neu-sel-in" : "bg-background text-muted ring-border hover:bg-plate hover:text-ink"
                }`}
              >
                {item.name}
                {live && (
                  <span className="animate-layer-pulse absolute -right-1 -top-1 h-2.5 w-2.5 rounded-full bg-accent ring-2 ring-white" />
                )}
              </button>
            );
          })}
        </div>
        {marquee}
      </div>
      {!layer || snapshot.selected_layout_keys.length === 0 ? (
        <EmptyState icon={<Keyboard size={32} />} title={t("keymap.empty_keymap_title")} body={t("keymap.empty_keymap_body")} />
      ) : (
        <KeymapCanvas
          keys={snapshot.selected_layout_keys}
          keyTitle={(key) => bindingsByPosition.get(key.position)?.full_label ?? "--"}
          keyStyle={keyStyle}
          onKeyClick={onKeyClick}
          keyContent={(key) => {
            const binding = bindingsByPosition.get(key.position);
            return (
              <>
                <div className="w-full truncate text-[11px] font-medium leading-tight text-ink">
                  {binding?.primary_label ?? "--"}
                </div>
                {binding?.primary_label && (
                  <div className="absolute bottom-1 right-1 font-mono text-[9px] leading-none text-faint">
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

function EditBar({ dirty, operation, problem, notice, onSave, onDiscard, onEnd }: {
  dirty: boolean;
  operation: EditState["operation"];
  problem: EditState["problem"];
  notice: "saved" | "discarded" | null;
  onSave: () => void;
  onDiscard: () => void;
  onEnd: () => void;
}) {
  const { t } = useLang();
  const busy = operation !== "idle";
  const message = problem
    ? t(`keymap.edit.problem.${problem}` as TranslationKey)
    : notice
      ? t(`keymap.edit.${notice}` as TranslationKey)
    : dirty
      ? t("keymap.edit.dirty")
      : "";
  return (
    <div className="fixed bottom-4 left-1/2 z-40 flex w-[min(720px,calc(100vw-32px))] -translate-x-1/2 flex-wrap items-center justify-between gap-3 rounded-card bg-surface px-4 py-3 shadow-neu-up ring-1 ring-border">
      <div className={`min-h-5 text-sm font-medium ${problem ? "text-red-700" : "text-muted"}`}>
        {message}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <button
          onClick={onSave}
          disabled={busy || !dirty}
          className="flex items-center gap-1.5 rounded-pill bg-accent px-3 py-1.5 text-sm font-medium text-white disabled:opacity-50"
        >
          <Save size={14} />
          {operation === "saving" ? t("keymap.edit.saving") : t("keymap.edit.save")}
        </button>
        <button
          onClick={onDiscard}
          disabled={busy || !dirty}
          className="flex items-center gap-1.5 rounded-pill bg-background px-3 py-1.5 text-sm font-medium text-muted ring-1 ring-border disabled:opacity-50"
        >
          <Trash2 size={14} />
          {t("keymap.edit.discard")}
        </button>
        <button
          onClick={onEnd}
          disabled={busy}
          className="flex items-center gap-1.5 rounded-pill bg-background px-3 py-1.5 text-sm font-medium text-muted ring-1 ring-border disabled:opacity-50"
        >
          <LogOut size={14} />
          {t("keymap.edit.end")}
        </button>
      </div>
    </div>
  );
}

function BindingPicker({ catalog, layers, rect, busy, onClose, onSelect }: {
  catalog: KeyCatalogEntry[];
  layers: StudioLayer[];
  rect: { left: number; top: number; width: number; height: number };
  busy: boolean;
  onClose: () => void;
  onSelect: (behavior: EditBehavior) => void;
}) {
  const { t } = useLang();
  const [tab, setTab] = useState<"key" | "layer">("key");
  const [layerBehavior, setLayerBehavior] = useState<"momentary_layer" | "toggle_layer" | "to_layer">("momentary_layer");
  const [query, setQuery] = useState("");
  const queryLower = query.trim().toLowerCase();
  const filtered = useMemo(() => {
    if (!queryLower) return catalog;
    return catalog.filter((entry) => {
      const usage = `0x${entry.hid_usage.toString(16)}`;
      return (
        entry.display.toLowerCase().includes(queryLower) ||
        entry.canonical.toLowerCase().includes(queryLower) ||
        usage.includes(queryLower) ||
        entry.aliases.some((alias) => alias.toLowerCase().includes(queryLower))
      );
    });
  }, [catalog, queryLower]);

  const position = pickerPosition(rect);
  const grouped = useMemo(() => {
    const map = new Map<KeyCatalogEntry["category"], KeyCatalogEntry[]>();
    for (const entry of filtered) {
      const list = map.get(entry.category) ?? [];
      list.push(entry);
      map.set(entry.category, list);
    }
    return [...map.entries()];
  }, [filtered]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <>
      <button className="fixed inset-0 z-40 cursor-default bg-transparent" onClick={onClose} />
      <div
        className="fixed z-50 flex max-h-[min(560px,calc(100vh-32px))] w-[min(520px,calc(100vw-24px))] flex-col rounded-card bg-surface p-3 shadow-neu-up ring-1 ring-border"
        style={{ left: position.left, top: position.top }}
      >
        <div className="flex gap-1 rounded-pill bg-background p-1 ring-1 ring-border">
          {(["key", "layer"] as const).map((item) => (
            <button
              key={item}
              type="button"
              onClick={() => setTab(item)}
              className={`flex-1 rounded-pill px-3 py-1.5 text-sm font-medium transition-colors ${
                tab === item ? "bg-plate text-accent-deep shadow-neu-sel-in" : "text-muted hover:text-ink"
              }`}
            >
              {t(item === "key" ? "keymap.edit.tab_key" : "keymap.edit.tab_layer")}
            </button>
          ))}
        </div>
        {tab === "key" ? (
          <>
            <div className="mt-3 flex items-center gap-2 rounded-pill bg-background px-3 py-2 ring-1 ring-border">
              <Search size={15} className="text-faint" />
              <input
                autoFocus
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={t("keymap.edit.search")}
                className="min-w-0 flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-faint"
              />
            </div>
            <div className="mt-3 flex gap-2">
              <button
                disabled={busy}
                onClick={() => onSelect({ kind: "transparent" })}
                className="flex-1 rounded-lg bg-background px-3 py-2 text-left text-sm ring-1 ring-border disabled:opacity-50"
              >
                <div className="font-medium text-ink">{t("keymap.edit.transparent")}</div>
                <div className="mt-0.5 text-xs text-faint">{t("keymap.edit.transparent_desc")}</div>
              </button>
              <button
                disabled={busy}
                onClick={() => onSelect({ kind: "none" })}
                className="flex-1 rounded-lg bg-background px-3 py-2 text-left text-sm ring-1 ring-border disabled:opacity-50"
              >
                <div className="font-medium text-ink">{t("keymap.edit.none")}</div>
                <div className="mt-0.5 text-xs text-faint">{t("keymap.edit.none_desc")}</div>
              </button>
            </div>
            <div className="mt-3 min-h-0 flex-1 overflow-y-auto pr-1">
              {grouped.map(([category, entries]) => (
                <div key={category} className="mb-3">
                  <div className="mb-1.5 text-xs font-medium uppercase text-faint">
                    {t(`keymap.catalog.${category}` as TranslationKey)}
                  </div>
                  <div className="flex flex-wrap gap-1.5">
                    {entries.map((entry) => (
                      <button
                        key={`${entry.hid_usage}-${entry.canonical}`}
                        disabled={busy}
                        onClick={() => onSelect({ kind: "key_press", hid_usage: entry.hid_usage })}
                        className="rounded-md bg-background px-2.5 py-1.5 text-sm font-medium text-ink ring-1 ring-border hover:bg-plate disabled:opacity-50"
                        title={(entry.names?.length ? entry.names : [entry.canonical]).join(" / ")}
                      >
                        {entry.display}
                      </button>
                    ))}
                  </div>
                </div>
              ))}
              {grouped.length === 0 && (
                <div className="py-8 text-center text-sm text-faint">
                  {t("keymap.edit.no_results")}
                </div>
              )}
            </div>
          </>
        ) : (
          <div className="mt-3 min-h-0 flex-1 overflow-y-auto pr-1">
            <div className="mb-3">
              <div className="mb-1.5 text-xs font-medium uppercase text-faint">
                {t("keymap.edit.layer_behavior")}
              </div>
              <div className="flex gap-1.5">
                {(["momentary_layer", "toggle_layer", "to_layer"] as const).map((kind) => (
                  <button
                    key={kind}
                    type="button"
                    disabled={busy}
                    onClick={() => setLayerBehavior(kind)}
                    title={t(`keymap.edit.${kind}_tooltip` as TranslationKey)}
                    className={`rounded-md px-3 py-1.5 text-sm font-semibold ring-1 disabled:opacity-50 ${
                      layerBehavior === kind
                        ? "bg-plate text-accent-deep ring-transparent shadow-neu-sel-in"
                        : "bg-background text-ink ring-border hover:bg-plate"
                    }`}
                  >
                    {t(`keymap.edit.${kind}` as TranslationKey)}
                  </button>
                ))}
              </div>
            </div>
            <div>
              <div className="mb-1.5 text-xs font-medium uppercase text-faint">
                {t("keymap.edit.target_layer")}
              </div>
              <div className="flex flex-wrap gap-1.5">
                {layers.map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    disabled={busy}
                    onClick={() => onSelect({ kind: layerBehavior, target_layer_index: item.index })}
                    className="inline-flex max-w-full items-center gap-1.5 rounded-md bg-background px-2.5 py-1.5 text-xs font-medium text-ink ring-1 ring-border hover:bg-plate disabled:opacity-50"
                    title={`${item.name} (#${item.index})`}
                  >
                    <span className="font-mono text-[11px] text-faint">#{item.index}</span>
                    <span className="truncate">{item.name}</span>
                  </button>
                ))}
              </div>
              {layers.length === 0 && (
                <div className="py-8 text-center text-sm text-faint">
                  {t("keymap.edit.no_layers")}
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </>
  );
}

function pickerPosition(rect: { left: number; top: number; width: number; height: number }) {
  const width = Math.min(520, window.innerWidth - 24);
  const height = Math.min(560, window.innerHeight - 32);
  let left = rect.left + rect.width / 2 - width / 2;
  let top = rect.top + rect.height + 8;
  if (left + width > window.innerWidth - 12) left = window.innerWidth - width - 12;
  if (left < 12) left = 12;
  if (top + height > window.innerHeight - 12) top = Math.max(12, rect.top - height - 8);
  return { left, top };
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
              className={`rounded-pill px-3 py-1.5 text-sm font-medium ring-1 transition-colors ${
                period === item.value
                  ? "bg-plate text-accent-deep ring-transparent shadow-neu-sel-in"
                  : "bg-background text-muted ring-border hover:bg-plate hover:text-ink"
              }`}
            >
              {t(item.key)}
            </button>
          ))}
        </div>
        <div className="flex flex-wrap items-center gap-4 text-sm text-muted">
          <span>
            {t("stats.total")}:{" "}
            <span className="font-mono font-medium text-ink">
              {(summary?.total ?? 0).toLocaleString()}
            </span>
          </span>
          {balance && (
            <span>
              {t("stats.balance")}:{" "}
              <span className="font-mono font-medium text-ink">
                {balance.left}% / {balance.right}%
              </span>
            </span>
          )}
        </div>
      </div>

      {statsError && <Notice>{statsError}</Notice>}

      {summary && summary.total === 0 && (
        <p className="text-sm text-faint">{t("stats.no_data")}</p>
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
                <div className="w-full truncate text-[10px] font-medium leading-tight text-muted">
                  {labelByPosition.get(key.position) ?? ""}
                </div>
                <div className="w-full truncate font-mono text-[10px] font-medium leading-tight text-ink">
                  {count > 0 ? count.toLocaleString() : ""}
                </div>
              </>
            );
          }}
        />
      )}

      {topKeys.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted">
          <span className="font-medium uppercase tracking-wide text-faint">
            {t("stats.top")}
          </span>
          {topKeys.map((entry) => (
            <span
              key={entry.position}
              className="inline-flex items-center gap-1 rounded-md bg-plate px-2 py-0.5"
            >
              <span className="font-medium text-ink">
                {labelByPosition.get(entry.position) ?? `#${entry.position}`}
              </span>
              <span className="font-mono text-muted">{entry.count.toLocaleString()}</span>
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

function TesterContent({ snapshot, activeLayer, setActiveLayer, layer, reportedLayerIndex, statsUid }: {
  snapshot: StudioKeymapSnapshot;
  activeLayer: number;
  setActiveLayer: (v: number) => void;
  layer: StudioLayer | null;
  reportedLayerIndex: number | null;
  statsUid: string | null;
}) {
  const { t } = useLang();
  const [pressedKeys, setPressedKeys] = useState<Set<number>>(new Set());
  const [testedKeys, setTestedKeys] = useState<Set<number>>(new Set());
  const [typed, setTyped] = useState<{ id: number; label: string }[]>([]);

  // Resolve a pressed position to its keymap label on the currently displayed
  // layer. We have no real keycode from the firmware (KEY_PRESS carries only
  // position), so this shows the binding label, not the OS-level character.
  const labelByPosition = useMemo(() => {
    const map = new Map<number, string>();
    if (layer) for (const b of layer.bindings) map.set(b.position, b.primary_label);
    return map;
  }, [layer]);
  const labelByPositionRef = useRef(labelByPosition);
  labelByPositionRef.current = labelByPosition;

  useEffect(() => {
    if (!statsUid) return;
    let unlisten: (() => void) | null = null;
    let disposed = false;
    let nextId = 0;
    void onKeyPressEvent((ev: KeyPressEvent) => {
      if (ev.device_uid !== statsUid) return;
      if (ev.pressed) {
        setPressedKeys((prev) => new Set(prev).add(ev.position));
        setTestedKeys((prev) => new Set(prev).add(ev.position));
        const label = labelByPositionRef.current.get(ev.position);
        if (label && label !== "--") {
          setTyped((prev) => [...prev, { id: nextId++, label }].slice(-40));
        }
      } else {
        setPressedKeys((prev) => {
          const next = new Set(prev);
          next.delete(ev.position);
          return next;
        });
      }
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
      setPressedKeys(new Set());
    };
  }, [statsUid]);

  const keyStyle = useCallback(
    (key: StudioPhysicalKey): CSSProperties | undefined => {
      if (pressedKeys.has(key.position))
        return { backgroundColor: "rgb(var(--accent-rgb))", color: "#fff", transition: "background-color 50ms" };
      if (testedKeys.has(key.position))
        return { backgroundColor: "rgb(var(--accent-rgb) / 0.25)" };
      return undefined;
    },
    [pressedKeys, testedKeys],
  );

  if (!statsUid) {
    return (
      <EmptyState
        icon={<Crosshair size={32} />}
        title={t("stats.no_link")}
        body={t("stats.no_link.hint")}
      />
    );
  }

  return (
    <KeymapContent
      snapshot={snapshot}
      activeLayer={activeLayer}
      setActiveLayer={setActiveLayer}
      layer={layer}
      reportedLayerIndex={reportedLayerIndex}
      keyStyle={keyStyle}
      marquee={
        <>
          <TypedMarquee typed={typed} />
          <button
            onClick={() => { setTestedKeys(new Set()); setPressedKeys(new Set()); setTyped([]); }}
            className="shrink-0 rounded-pill px-3 py-1.5 text-sm text-muted hover:bg-plate hover:text-ink transition-colors"
          >
            {t("tester.reset")}
          </button>
        </>
      }
    />
  );
}

/** Right-to-left marquee of typed key labels, shown beside the tester's layer
 *  tabs. Grows/shrinks via flex-1 as the layer tabs take more/less width. */
function TypedMarquee({ typed }: { typed: { id: number; label: string }[] }) {
  return (
    <div className="flex h-8 min-w-0 flex-1 items-center overflow-hidden rounded-pill bg-background px-3 ring-1 ring-border">
      <div className="ml-auto flex items-center gap-2 whitespace-nowrap">
        {typed.map((item) => (
          <span key={item.id} className="animate-key-flow font-mono text-sm font-medium text-accent">
            {item.label}
          </span>
        ))}
      </div>
    </div>
  );
}

/** White → gauge gray → accent → red, used for per-key heat coloring. */
function heatColor(ratio: number): string {
  const clamped = Math.max(0, Math.min(1, ratio));
  if (clamped < 0.5) {
    const a = clamped / 0.5;
    return `rgba(140, 149, 163, ${(0.15 + 0.45 * a).toFixed(3)})`;
  }
  if (clamped < 0.8) {
    const a = (clamped - 0.5) / 0.3;
    return `rgb(var(--accent-rgb) / ${(0.4 + 0.35 * a).toFixed(3)})`;
  }
  const a = (clamped - 0.8) / 0.2;
  return `rgba(239, 68, 68, ${(0.55 + 0.35 * a).toFixed(3)})`;
}

function StudioStatusBadge({ device }: { device: StudioDeviceStatus }) {
  const { t } = useLang();
  const ok = device.keymap_viewer_status === "available";
  const locked = device.keymap_viewer_status === "locked";
  const className = ok
    ? "bg-accent-soft text-accent-deep"
    : locked
      ? "bg-amber-100 text-amber-700"
      : "bg-plate text-muted";
  return <span className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${className}`}>{t(`keymap.viewer.${device.keymap_viewer_status}` as TranslationKey)}</span>;
}

function EmptyState({ icon, title, body }: { icon: ReactNode; title: string; body?: string }) {
  return (
    <div className="flex min-h-[360px] flex-col items-center justify-center text-center">
      <div className="mb-3 text-disabled">{icon}</div>
      <div className="text-sm font-medium text-ink">{title}</div>
      {body && <div className="mt-1 max-w-md text-sm text-faint">{body}</div>}
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
