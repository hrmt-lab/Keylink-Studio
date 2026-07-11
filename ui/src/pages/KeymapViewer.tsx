import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode, type Dispatch, type SetStateAction } from "react";
import { open, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Crosshair, AlertCircle, BarChart3, Keyboard, Lock, RefreshCw, XCircle, Pencil, Save, Trash2, LogOut, Search, Plus, Usb, Bluetooth, Download, Upload } from "lucide-react";
import {
  getKeyStats,
  onKeyPressEvent,
  onKeyStatsUpdated,
  readEncoderLayerBindings,
  readEncoderInfo,
  readStudioKeymap,
  resolveStudioBehaviorLabels,
  studioAddLayer,
  studioApplyKeymapRestore,
  studioBeginEdit,
  studioDiscardChanges,
  studioEncoderHasUnsaved,
  studioEndEdit,
  studioExportKeymap,
  studioHasUnsaved,
  studioKeyCatalog,
  studioPreviewKeymapRestore,
  studioRemoveLayer,
  studioRenameLayer,
  studioResyncEditState,
  studioSaveChanges,
  studioSetEncoderBindings,
  studioSetKey,
} from "../api";
import { KeymapCanvas } from "../components/KeymapCanvas";
import { friendlyError } from "../lib/errors";
import { useLang, type TranslationKey } from "../i18n";
import type {
  DeviceInfo,
  DeviceLayerState,
  DiscardChangesDto,
  KeyPressEvent,
  EditBehavior,
  EditState,
  EncoderBindingsDto,
  KeyCatalogEntry,
  KeyStatsSummary,
  MonitorStatus,
  RestoreReport,
  SaveOrDiscardResultDto,
  StatsPeriod,
  StudioBinding,
  StudioBindingLabelPatch,
  StudioDeviceStatus,
  StudioKeymapSnapshot,
  StudioLayer,
  StudioPhysicalKey,
  StudioRawBinding,
} from "../types";

// Config RPC (Host Link DEVICE_HELLO capability bit); see packet::CAPABILITY_CONFIG_RPC.
const CONFIG_RPC_CAPABILITY = 1 << 9;

interface KeymapViewerProps {
  studioDevices: StudioDeviceStatus[];
  studioScanning: boolean;
  studioError: string | null;
  refreshStudioDevices: () => Promise<StudioDeviceStatus[]>;
  snapshotsByDeviceId: Record<string, StudioKeymapSnapshot>;
  setSnapshotsByDeviceId: Dispatch<SetStateAction<Record<string, StudioKeymapSnapshot>>>;
  status: MonitorStatus;
  onRegisterNavigationGuard?: (guard: KeymapNavigationGuard | null) => void;
}

interface PendingKeyWrite {
  deviceId: string;
  layerIndex: number;
  layerId: number;
  position: number;
  behavior: EditBehavior;
  previousSnapshot: StudioKeymapSnapshot | null;
}

type PickerRect = { left: number; top: number; width: number; height: number };
type EncoderLoadState = "loading" | "available" | "unsupported" | "error";
type EncoderDirection = "cw" | "ccw";

interface KeymapNavigationGuard {
  hasUnsaved: () => Promise<boolean>;
  canLeave: () => boolean;
  saveAndLeave: () => Promise<boolean>;
  discardAndLeave: () => Promise<boolean>;
}

function changedKeyId(layerIndex: number, position: number): string {
  return `${layerIndex}:${position}`;
}

function encoderTileId(layerId: number, encoderId: number): string {
  return `${layerId}:${encoderId}`;
}

function encoderSideEquals(a: EncoderBindingsDto[EncoderDirection], b: EncoderBindingsDto[EncoderDirection]): boolean {
  return a.behavior_id === b.behavior_id && a.param1 === b.param1 && a.param2 === b.param2;
}

function encoderBindingDiffers(current: EncoderBindingsDto, baseline: EncoderBindingsDto): boolean {
  if (baseline.source === "keymap") {
    return current.source !== "keymap" || current.runtime_dirty;
  }
  return !encoderSideEquals(current.cw, baseline.cw) || !encoderSideEquals(current.ccw, baseline.ccw);
}

/** Case-insensitive serial match between a Studio device and Host Link data. */
function serialsMatch(a: string | null, b: string | null): boolean {
  if (!a || !b) return false;
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

function uidHex(value: string | null | undefined): string | null {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) return null;
  const hex = normalized.startsWith("uid:") ? normalized.slice(4) : normalized;
  return /^[0-9a-f]{16}$/.test(hex) ? hex : null;
}

function uidStringsMatch(a: string | null | undefined, b: string | null | undefined): boolean {
  const uidA = uidHex(a);
  const uidB = uidHex(b);
  return uidA !== null && uidA === uidB;
}

function normalizedName(value: string | null | undefined): string | null {
  const normalized = value?.trim().toLowerCase();
  return normalized ? normalized : null;
}

function studioDeviceNames(device: StudioDeviceStatus): Set<string> {
  return new Set(
    [device.product, device.display_name]
      .map(normalizedName)
      .filter((value): value is string => value !== null)
  );
}

function findDeviceLayerForStudio(
  entries: DeviceLayerState[],
  studioDevice: StudioDeviceStatus
): DeviceLayerState | null {
  const uidMatch = entries.find((entry) =>
    uidStringsMatch(entry.device_key, studioDevice.serial_number)
  );
  if (uidMatch) return uidMatch;

  const serialMatch = entries.find((entry) =>
    serialsMatch(entry.serial_number, studioDevice.serial_number)
  );
  if (serialMatch) return serialMatch;

  const names = studioDeviceNames(studioDevice);
  if (names.size === 0) return null;
  const productMatches = entries.filter((entry) => {
    const product = normalizedName(entry.product);
    return product !== null && names.has(product);
  });
  return productMatches.length === 1 ? productMatches[0] : null;
}

/**
 * Strict UID-only match between a Studio device and a Host Link device. Unlike
 * `findHostLinkDeviceForStudio` (used for read-only stats, with serial/name
 * fallbacks), write-capable features (encoder Config RPC) must never bind to
 * the wrong physical device, so only an exact device_uid_hash match qualifies.
 */
function findHostLinkDeviceForStudioStrict(
  devices: DeviceInfo[],
  studioDevice: StudioDeviceStatus
): DeviceInfo | null {
  return (
    devices.find((device) =>
      uidStringsMatch(device.device_uid_hash, studioDevice.serial_number)
    ) ?? null
  );
}

function findHostLinkDeviceForStudio(
  devices: DeviceInfo[],
  studioDevice: StudioDeviceStatus
): DeviceInfo | null {
  const uidMatch = devices.find((device) =>
    uidStringsMatch(device.device_uid_hash, studioDevice.serial_number)
  );
  if (uidMatch) return uidMatch;

  const serialMatch = devices.find((device) =>
    serialsMatch(device.serial_number, studioDevice.serial_number)
  );
  if (serialMatch) return serialMatch;

  const names = studioDeviceNames(studioDevice);
  if (names.size === 0) return null;
  const productMatches = devices.filter((device) => {
    const product = normalizedName(device.product);
    return (
      device.connection_type === "bluetooth" &&
      product !== null &&
      names.has(product)
    );
  });
  return productMatches.length === 1 ? productMatches[0] : null;
}

function compareDeviceName(a: string, b: string): number {
  return a.localeCompare(b, undefined, { sensitivity: "base", numeric: true });
}

function rawBindingKey(binding: StudioRawBinding): string {
  return `${binding.behavior_id}:${binding.param1}:${binding.param2}`;
}

function unresolvedRawBindings(snapshot: StudioKeymapSnapshot): StudioRawBinding[] {
  const seen = new Set<string>();
  const bindings: StudioRawBinding[] = [];
  for (const layer of snapshot.layers) {
    for (const binding of layer.bindings) {
      if (!binding.behavior.startsWith("behavior ")) continue;
      const key = rawBindingKey(binding.raw);
      if (seen.has(key)) continue;
      seen.add(key);
      bindings.push(binding.raw);
    }
  }
  return bindings;
}

function resolvedLabelPatchesFromSnapshot(snapshot: StudioKeymapSnapshot): StudioBindingLabelPatch[] {
  const seen = new Set<string>();
  const patches: StudioBindingLabelPatch[] = [];
  for (const layer of snapshot.layers) {
    for (const binding of layer.bindings) {
      if (binding.behavior.startsWith("behavior ")) continue;
      const key = rawBindingKey(binding.raw);
      if (seen.has(key)) continue;
      seen.add(key);
      patches.push({
        behavior_id: binding.raw.behavior_id,
        param1: binding.raw.param1,
        param2: binding.raw.param2,
        behavior: binding.behavior,
        binding_label: binding.binding_label,
        primary_label: binding.primary_label,
        secondary_label: binding.secondary_label,
        full_label: binding.full_label,
      });
    }
  }
  return patches;
}

function applyBehaviorLabelPatches(
  snapshot: StudioKeymapSnapshot,
  patches: StudioBindingLabelPatch[],
): StudioKeymapSnapshot {
  if (patches.length === 0) return snapshot;
  const patchByRaw = new Map(patches.map((patch) => [
    rawBindingKey(patch),
    patch,
  ]));
  return {
    ...snapshot,
    layers: snapshot.layers.map((layer) => ({
      ...layer,
      bindings: layer.bindings.map((binding) => {
        const patch = patchByRaw.get(rawBindingKey(binding.raw));
        if (!patch) return binding;
        return {
          ...binding,
          behavior: patch.behavior,
          binding_label: patch.binding_label,
          primary_label: patch.primary_label,
          secondary_label: patch.secondary_label,
          full_label: patch.full_label,
        };
      }),
    })),
  };
}

function optimisticSnapshotForSetKey(
  snapshot: StudioKeymapSnapshot,
  layerId: number,
  position: number,
  behavior: EditBehavior,
  catalog: KeyCatalogEntry[],
): StudioKeymapSnapshot {
  const labels = optimisticLabelsForBehavior(behavior, catalog);
  return {
    ...snapshot,
    layers: snapshot.layers.map((layer) => {
      if (layer.id !== layerId) return layer;
      return {
        ...layer,
        bindings: layer.bindings.map((binding) => {
          if (binding.position !== position) return binding;
          return {
            ...binding,
            behavior: labels.behavior,
            binding_label: labels.full_label,
            primary_label: labels.primary_label,
            secondary_label: labels.secondary_label,
            full_label: labels.full_label,
            params: labels.params,
          };
        }),
      };
    }),
    updated_ms: Date.now(),
  };
}

function optimisticLabelsForBehavior(
  behavior: EditBehavior,
  catalog: KeyCatalogEntry[],
): {
  behavior: string;
  primary_label: string;
  secondary_label: string;
  full_label: string;
  params: number[];
} {
  switch (behavior.kind) {
    case "key_press": {
      const label = keyUsageDisplayLabel(behavior.hid_usage, catalog);
      return {
        behavior: "key press",
        primary_label: label,
        secondary_label: "",
        full_label: `&kp ${label}`,
        params: [behavior.hid_usage, 0],
      };
    }
    case "transparent":
      return {
        behavior: "transparent",
        primary_label: "&trans",
        secondary_label: "",
        full_label: "&trans",
        params: [0, 0],
      };
    case "none":
      return {
        behavior: "none",
        primary_label: "",
        secondary_label: "",
        full_label: "&none",
        params: [0, 0],
      };
    case "momentary_layer":
      return layerLabel("momentary layer", "mo", behavior.target_layer_index);
    case "toggle_layer":
      return layerLabel("toggle layer", "tog", behavior.target_layer_index);
    case "to_layer":
      return layerLabel("to layer", "to", behavior.target_layer_index);
    case "mod_tap": {
      const hold = modifierUsageDisplayLabel(behavior.hold_hid_usage);
      const tap = keyUsageDisplayLabel(behavior.tap_hid_usage, catalog);
      return {
        behavior: "mod-tap",
        primary_label: `mt ${hold} ${tap}`,
        secondary_label: "",
        full_label: `&mt ${hold} ${tap}`,
        params: [behavior.hold_hid_usage, behavior.tap_hid_usage],
      };
    }
    case "layer_tap": {
      const tap = keyUsageDisplayLabel(behavior.tap_hid_usage, catalog);
      return {
        behavior: "layer-tap",
        primary_label: `lt ${behavior.target_layer_index} ${tap}`,
        secondary_label: "",
        full_label: `&lt ${behavior.target_layer_index} ${tap}`,
        params: [behavior.target_layer_index, behavior.tap_hid_usage],
      };
    }
    case "sticky_key": {
      const label = keyUsageDisplayLabel(behavior.hid_usage, catalog);
      return {
        behavior: "sticky key",
        primary_label: `sk ${label}`,
        secondary_label: "",
        full_label: `&sk ${label}`,
        params: [behavior.hid_usage, 0],
      };
    }
    case "sticky_layer":
      return layerLabel("sticky layer", "sl", behavior.target_layer_index);
    case "bluetooth":
      return commandLabel("bluetooth", `&bt ${behavior.command} ${behavior.value}`, [behavior.command, behavior.value]);
    case "output_selection":
      return commandLabel("output selection", outputCommandLabel(behavior.value), [behavior.value, 0]);
    case "mouse_key_press":
      return commandLabel("mouse key press", mouseCommandLabel(MOUSE_BUTTON_COMMANDS, behavior.value, "&mkp"), [behavior.value, 0]);
    case "mouse_move":
      return commandLabel("mouse move", mouseCommandLabel(MOUSE_MOVE_COMMANDS, behavior.value, "&mmv"), [behavior.value, 0]);
    case "mouse_scroll":
      return commandLabel("mouse scroll", mouseCommandLabel(MOUSE_SCROLL_COMMANDS, behavior.value, "&msc"), [behavior.value, 0]);
    case "caps_word":
      return commandLabel("caps word", "&caps_word", [0, 0]);
    case "key_repeat":
      return commandLabel("key repeat", "&key_repeat", [0, 0]);
    case "reset":
      return commandLabel("reset", "&reset", [0, 0]);
    case "bootloader":
      return commandLabel("bootloader", "&bootloader", [0, 0]);
    case "studio_unlock":
      return commandLabel("studio unlock", "&studio_unlock", [0, 0]);
    case "grave_escape":
      return commandLabel("grave/escape", "&gresc", [0, 0]);
  }
}

function layerLabel(behavior: string, prefix: string, layerIndex: number) {
  return {
    behavior,
    primary_label: `${prefix} ${layerIndex}`,
    secondary_label: "",
    full_label: `&${prefix} ${layerIndex}`,
    params: [layerIndex, 0],
  };
}

function commandLabel(behavior: string, fullLabel: string, params: number[]) {
  return {
    behavior,
    primary_label: fullLabel,
    secondary_label: "",
    full_label: fullLabel,
    params,
  };
}

function keyUsageDisplayLabel(usage: number, catalog: KeyCatalogEntry[]): string {
  const modifierBits = (usage >>> 24) & 0xff;
  const baseUsage = usage & 0x00ff_ffff;
  const baseLabel = catalog.find((entry) => entry.hid_usage === baseUsage)?.display
    ?? `0x${baseUsage.toString(16)}`;
  if (modifierBits === 0) return baseLabel;
  const mods = MODIFIER_OPTIONS
    .filter((option) => (modifierBits & option.modifierBit) !== 0)
    .map((option) => modifierShortLabel(option.zmkName));
  return mods.reduceRight((label, mod) => `${mod}(${label})`, baseLabel);
}

function modifierUsageDisplayLabel(usage: number): string {
  const modifierBits = (usage >>> 24) & 0xff;
  const baseUsage = usage & 0x00ff_ffff;
  const base = MODIFIER_OPTIONS.find((option) => option.baseUsage === baseUsage);
  const baseLabel = base ? modifierShortLabel(base.zmkName) : `0x${baseUsage.toString(16)}`;
  if (modifierBits === 0) return baseLabel;
  return MODIFIER_OPTIONS
    .filter((option) => (modifierBits & option.modifierBit) !== 0)
    .map((option) => modifierShortLabel(option.zmkName))
    .reduce((label, mod) => `${label}(${mod})`, baseLabel);
}

function modifierShortLabel(label: string): string {
  switch (label) {
    case "LCTRL":
      return "LC";
    case "LSHIFT":
      return "LS";
    case "LALT":
      return "LA";
    case "LGUI":
      return "LG";
    case "RCTRL":
      return "RC";
    case "RSHIFT":
      return "RS";
    case "RALT":
      return "RA";
    case "RGUI":
      return "RG";
    default:
      return label;
  }
}

function outputCommandLabel(value: number): string {
  return OUTPUT_COMMANDS.find((command) => command.value === value)?.title ?? `&out ${value}`;
}

function mouseCommandLabel(
  commands: Array<{ title: string; value: number }>,
  value: number,
  fallbackPrefix: string,
): string {
  return commands.find((command) => command.value === value)?.title ?? `${fallbackPrefix} ${value}`;
}

export default function KeymapViewer({
  studioDevices,
  studioScanning,
  studioError,
  refreshStudioDevices,
  snapshotsByDeviceId,
  setSnapshotsByDeviceId,
  status,
  onRegisterNavigationGuard,
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
  const [keymapFileBusy, setKeymapFileBusy] = useState<"export" | "restore" | null>(null);
  const [restoreReport, setRestoreReport] = useState<RestoreReport | null>(null);
  const [restoreNotice, setRestoreNotice] = useState<"no_targets" | null>(null);
  const [changedKeys, setChangedKeys] = useState<Set<string>>(() => new Set());
  const [flashKeys, setFlashKeys] = useState<Map<string, number>>(() => new Map());
  const [flashEncoderTiles, setFlashEncoderTiles] = useState<Map<string, number>>(() => new Map());
  const [catalog, setCatalog] = useState<KeyCatalogEntry[]>([]);
  const [pendingKeyWrites, setPendingKeyWrites] = useState(0);
  const [keyWriteErrorCode, setKeyWriteErrorCode] = useState<string | null>(null);
  const [picker, setPicker] = useState<{
    key: StudioPhysicalKey;
    layer: StudioLayer;
    rect: PickerRect;
  } | null>(null);
  const [encoderPanel, setEncoderPanel] = useState<{ encoderId: number; rect: PickerRect } | null>(null);
  const [pendingEncoderWrites, setPendingEncoderWrites] = useState(0);
  const [encoderDirtyByUid, setEncoderDirtyByUid] = useState<Record<string, boolean>>({});
  const [editHostLinkUid, setEditHostLinkUid] = useState<string | null>(null);
  const [encoderWriteErrorCode, setEncoderWriteErrorCode] = useState<string | null>(null);
  const [encoderRefreshNonce, setEncoderRefreshNonce] = useState(0);
  const [encoderLoadState, setEncoderLoadState] = useState<EncoderLoadState>("unsupported");
  const [changedEncoderTiles, setChangedEncoderTiles] = useState<Set<string>>(() => new Set());
  const behaviorResolveKeysRef = useRef<Set<string>>(new Set());
  const latestKeyWriteSnapshotRef = useRef<StudioKeymapSnapshot | null>(null);
  const keyWriteQueueRef = useRef<PendingKeyWrite[]>([]);
  const keyWriteActiveRef = useRef(false);
  // Preserve UI request order. The Rust Host Link worker also serializes all
  // HID traffic and owns the persistent Config RPC sequence.
  const configRpcChainRef = useRef<Promise<void>>(Promise.resolve());
  const encoderInfoGenerationRef = useRef(0);
  const encoderRequestGenerationRef = useRef(0);
  const encoderBindingsCacheRef = useRef(new Map<string, EncoderBindingsDto[]>());
  const encoderCacheDeviceRef = useRef<string | null>(null);
  const encoderCacheUidRef = useRef<string | null>(null);
  const encoderBaselineRef = useRef(new Map<string, EncoderBindingsDto>());
  const encoderFlashVersionRef = useRef(0);

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
  const changedKeyStyle = useCallback((key: StudioPhysicalKey): CSSProperties | undefined => {
    if (!changedKeys.has(changedKeyId(activeLayer, key.position))) return undefined;
    return {
      backgroundColor: "rgb(var(--accent-rgb) / 0.18)",
      boxShadow: "inset 0 0 0 2px rgb(var(--accent-rgb) / 0.72)",
    };
  }, [activeLayer, changedKeys]);

  // Prefer the firmware UID contract: Studio serial_number is the 16-char UID
  // hex, while Host Link exposes the same value as uid:<hex>. Keep older
  // serial/name fallbacks for firmware that has not adopted that contract yet.
  const reportedLayer = useMemo(
    () =>
      selected
        ? findDeviceLayerForStudio(status.device_layers, selected)
        : null,
    [selected, status.device_layers]
  );
  const statsUid = useMemo(
    () =>
      selected
        ? findHostLinkDeviceForStudio(status.host_link_devices, selected)?.device_uid_hash ?? null
        : null,
    [selected, status.host_link_devices]
  );

  // Config RPC (encoder editing) requires the stricter UID-only match: unlike
  // stats display, sending SET_BINDINGS to the wrong physical device is not
  // an acceptable failure mode.
  const hostLinkDevice = useMemo(
    () =>
      selected
        ? findHostLinkDeviceForStudioStrict(status.host_link_devices, selected)
        : null,
    [selected, status.host_link_devices]
  );
  const hostLinkUid = hostLinkDevice?.device_uid_hash ?? null;
  const configRpcCapable =
    hostLinkDevice !== null &&
    (hostLinkDevice.capabilities & CONFIG_RPC_CAPABILITY) !== 0;
  const editing = editState.mode === "editing";
  const encoderDirtyUid = editHostLinkUid ?? hostLinkUid;
  const encoderDirty = encoderDirtyUid ? encoderDirtyByUid[encoderDirtyUid] ?? false : false;
  const setEncoderDirty = useCallback((value: boolean | ((current: boolean) => boolean)) => {
    const uid = editHostLinkUid ?? hostLinkUid;
    if (!uid) return;
    setEncoderDirtyByUid((current) => {
      const previous = current[uid] ?? false;
      const next = typeof value === "function" ? value(previous) : value;
      return { ...current, [uid]: next };
    });
  }, [editHostLinkUid, hostLinkUid]);

  const [encoderCount, setEncoderCount] = useState<number | null>(null);
  const [encoderBindings, setEncoderBindings] = useState<EncoderBindingsDto[]>([]);
  const [encoderError, setEncoderError] = useState<string | null>(null);

  useEffect(() => {
    const generation = ++encoderInfoGenerationRef.current;
    setEncoderCount(null);
    setEncoderError(null);
    const selectedDeviceChanged = encoderCacheDeviceRef.current !== selectedId;
    const physicalDeviceChanged =
      hostLinkUid !== null &&
      encoderCacheUidRef.current !== null &&
      encoderCacheUidRef.current !== hostLinkUid;
    if (selectedDeviceChanged || physicalDeviceChanged) {
      encoderBindingsCacheRef.current.clear();
      setEncoderBindings([]);
    }
    encoderCacheDeviceRef.current = selectedId;
    if (hostLinkUid !== null) encoderCacheUidRef.current = hostLinkUid;
    if (!selectedId) {
      setEncoderLoadState("unsupported");
      return;
    }
    if (!hostLinkUid) {
      setEncoderLoadState("error");
      setEncoderError(t("keymap.error.encoder_read_failed"));
      return;
    }
    if (!configRpcCapable) {
      setEncoderLoadState("unsupported");
      return;
    }
    setEncoderLoadState("loading");
    let cancelled = false;
    const uid = hostLinkUid;
    configRpcChainRef.current = configRpcChainRef.current.then(async () => {
      if (cancelled || generation !== encoderInfoGenerationRef.current) return;
      try {
        const info = await readEncoderInfo(uid);
        if (!cancelled && generation === encoderInfoGenerationRef.current) {
          setEncoderCount(info.encoder_count);
          setEncoderLoadState("available");
        }
      } catch (err) {
        if (!cancelled && generation === encoderInfoGenerationRef.current) {
          setEncoderError(friendlyError(err, t, "keymap.error.encoder_read_failed"));
          setEncoderLoadState("error");
        }
      }
    });
    return () => {
      cancelled = true;
    };
  }, [selectedId, hostLinkUid, configRpcCapable, encoderRefreshNonce, t]);

  useEffect(() => {
    const generation = ++encoderRequestGenerationRef.current;
    if (!selectedId || layer === null) {
      setEncoderBindings([]);
      return;
    }
    const cacheUid = hostLinkUid ?? encoderCacheUidRef.current;
    const cacheKey = cacheUid ? `${cacheUid}:${layer.id}` : null;
    const cached = cacheKey ? encoderBindingsCacheRef.current.get(cacheKey) : undefined;
    setEncoderBindings(cached ?? []);
    if (encoderCount === 0) return;
    // Keep the last confirmed tiles visible (disabled by encoderLoadState) while
    // Host Link is temporarily unavailable or GET_INFO is still pending.
    if (!hostLinkUid || encoderCount === null) return;
    const activeCacheKey = `${hostLinkUid}:${layer.id}`;
    setEncoderLoadState("loading");
    setEncoderError(null);
    let cancelled = false;
    const deviceId = selectedId;
    const uid = hostLinkUid;
    const layerId = layer.id;
    const count = encoderCount;
    configRpcChainRef.current = configRpcChainRef.current.then(async () => {
      try {
        const results = await readEncoderLayerBindings(deviceId, uid, layerId, count);
        if (!cancelled && generation === encoderRequestGenerationRef.current) {
          encoderBindingsCacheRef.current.set(activeCacheKey, results);
          setEncoderBindings(results);
          setEncoderLoadState("available");
          if (editing) {
            for (const result of results) {
              const id = encoderTileId(result.layer_id, result.encoder_id);
              const baseline = encoderBaselineRef.current.get(id);
              if (!baseline) {
                encoderBaselineRef.current.set(id, result);
                if (result.runtime_dirty) {
                  setChangedEncoderTiles((current) => new Set(current).add(id));
                }
                continue;
              }

              setChangedEncoderTiles((current) => {
                const next = new Set(current);
                if (encoderBindingDiffers(result, baseline)) next.add(id);
                else next.delete(id);
                return next;
              });
            }
          }
        }
      } catch (err) {
        if (!cancelled && generation === encoderRequestGenerationRef.current) {
          setEncoderError(friendlyError(err, t, "keymap.error.encoder_read_failed"));
          setEncoderLoadState("error");
        }
      }
    });
    return () => {
      cancelled = true;
    };
  }, [selectedId, hostLinkUid, encoderCount, layer, encoderRefreshNonce, editing]);

  // Encoder edit availability (keymap-encoder-editing-plan.md): Studio editing
  // active, strict-UID Host Link device with CONFIG_RPC, GET_INFO succeeded
  // with encoder_count > 0, and a target layer resolved.
  const encoderEditAvailable =
    editing &&
    hostLinkUid !== null &&
    configRpcCapable &&
    (encoderCount ?? 0) > 0 &&
    layer !== null;

  // Pick up pre-existing runtime override dirt (e.g. a previous session's
  // unsaved SET_BINDINGS) when entering edit mode.
  useEffect(() => {
    if (!editing || !editHostLinkUid) return;
    let cancelled = false;
    const uid = editHostLinkUid;
    studioEncoderHasUnsaved(uid)
      .then((dirty) => {
        if (!cancelled) {
          setEncoderDirtyByUid((current) => ({ ...current, [uid]: dirty }));
          if (!dirty) setEncoderRefreshNonce((nonce) => nonce + 1);
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [editing, editHostLinkUid]);

  useEffect(() => {
    const ids = new Set(devices.map((device) => device.id));
    setSelectedId((current) => current && ids.has(current) ? current : devices[0]?.id ?? "");
  }, [devices]);

  useEffect(() => {
    setActiveLayer(0);
    setKeyWriteErrorCode(null);
    keyWriteQueueRef.current = [];
    latestKeyWriteSnapshotRef.current = null;
    setPendingKeyWrites(0);
    setEncoderWriteErrorCode(null);
    setFlashEncoderTiles(new Map());
  }, [selectedId]);

  useEffect(() => {
    setFlashEncoderTiles(new Map());
  }, [activeLayer]);

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

  const readDevice = useCallback(async (device: StudioDeviceStatus, clearBeforeRead = false) => {
    setReading(true);
    setError(null);
    setRestoreReport(null);
    setRestoreNotice(null);
    setChangedKeys(new Set());
    if (clearBeforeRead) {
      setSnapshotsByDeviceId((current) => {
        if (!(device.id in current)) return current;
        const next = { ...current };
        delete next[device.id];
        return next;
      });
    }
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
    if (!selected || !snapshot || selected.connection_type !== "ble_studio") return;
    const unresolved = unresolvedRawBindings(snapshot);
    if (unresolved.length === 0) return;
    const resolveKey = `${selected.id}:${snapshot.updated_ms}:${unresolved
      .map((binding) => `${binding.behavior_id}/${binding.param1}/${binding.param2}`)
      .join("|")}`;
    if (behaviorResolveKeysRef.current.has(resolveKey)) return;
    behaviorResolveKeysRef.current.add(resolveKey);

    let disposed = false;
    void resolveStudioBehaviorLabels(selected.id, unresolved)
      .then((patches) => {
        if (disposed || patches.length === 0) return;
        setSnapshotsByDeviceId((current) => {
          const currentSnapshot = current[selected.id];
          if (!currentSnapshot || currentSnapshot.updated_ms !== snapshot.updated_ms) return current;
          return {
            ...current,
            [selected.id]: applyBehaviorLabelPatches(currentSnapshot, patches),
          };
        });
      })
      .catch((error) => {
        console.debug("failed to resolve Studio behavior labels", error);
      });

    return () => {
      disposed = true;
    };
  }, [selected, setSnapshotsByDeviceId, snapshot]);

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
    setRestoreReport(null);
    setRestoreNotice(null);
    setChangedKeys(new Set());
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
      setError(friendlyError(e, t));
    }
  }, [editState.mode, readDevice, refreshStudioDevices, selectedId, t]);

  const resyncKeyState = useCallback(async () => {
    if (!selected || pendingKeyWrites > 0) return;
    setKeyWriteErrorCode(null);
    try {
      const result = await studioResyncEditState(selected.id);
      setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: result.snapshot }));
      setEditState((current) => ({ ...current, dirty: result.has_unsaved, problem: null }));
    } catch (e) {
      setKeyWriteErrorCode(String(e));
    }
  }, [pendingKeyWrites, selected, setSnapshotsByDeviceId]);

  const resyncEncoderState = useCallback(async () => {
    const uid = editHostLinkUid ?? hostLinkUid;
    if (!uid || pendingEncoderWrites > 0) return;
    setEncoderWriteErrorCode(null);
    try {
      const dirty = await studioEncoderHasUnsaved(uid);
      setEncoderDirtyByUid((current) => ({ ...current, [uid]: dirty }));
      setEncoderRefreshNonce((nonce) => nonce + 1);
    } catch (e) {
      setEncoderWriteErrorCode(String(e));
    }
  }, [editHostLinkUid, hostLinkUid, pendingEncoderWrites]);

  const mapEditProblem = useCallback((code: string): EditState["problem"] => {
    if (code === "save_result_unknown") return "save_unknown";
    if (code === "save_failed" || code === "save_not_supported" || code === "save_no_space") return "save_failed";
    if (code === "locked") return "locked_again";
    if (code === "disconnected" || code === "timeout") return "disconnected";
    return null;
  }, []);

  const processKeyWriteQueue = useCallback(async () => {
    if (keyWriteActiveRef.current) return;
    keyWriteActiveRef.current = true;

    try {
      while (keyWriteQueueRef.current.length > 0) {
        const job = keyWriteQueueRef.current.shift();
        if (!job) break;

        try {
          const result = await studioSetKey(job.deviceId, job.layerId, job.position, job.behavior);
          latestKeyWriteSnapshotRef.current = result;
          setChangedKeys((current) => {
            const next = new Set(current);
            next.add(changedKeyId(job.layerIndex, job.position));
            return next;
          });
          const flashId = changedKeyId(job.layerIndex, job.position);
          const flashVersion = Date.now();
          setFlashKeys((current) => { const next = new Map(current); next.set(flashId, flashVersion); return next; });
          window.setTimeout(() => {
            setFlashKeys((current) => {
              if (current.get(flashId) !== flashVersion) return current;
              const next = new Map(current); next.delete(flashId); return next;
            });
          }, 1200);
          setPendingKeyWrites((current) => {
            const next = Math.max(0, current - 1);
            if (next === 0 && latestKeyWriteSnapshotRef.current) {
              const latest = latestKeyWriteSnapshotRef.current;
              latestKeyWriteSnapshotRef.current = null;
              setSnapshotsByDeviceId((snapshots) => ({ ...snapshots, [job.deviceId]: latest }));
            }
            return next;
          });
          setEditState((current) => ({ ...current, dirty: true, problem: null }));
        } catch (e) {
          const code = String(e);
          const problem = mapEditProblem(code);
          keyWriteQueueRef.current = [];
          latestKeyWriteSnapshotRef.current = null;
          setPendingKeyWrites(0);
          if (job.previousSnapshot) {
            setSnapshotsByDeviceId((current) => ({ ...current, [job.deviceId]: job.previousSnapshot! }));
          }
          setEditState((current) => ({ ...current, problem }));
          setKeyWriteErrorCode(code);
          break;
        }
      }
    } finally {
      keyWriteActiveRef.current = false;
    }
  }, [mapEditProblem, setSnapshotsByDeviceId]);

  const beginEdit = useCallback(async (forceDiscard = false) => {
    if (!selected) return;
    const labelPatches = snapshot ? resolvedLabelPatchesFromSnapshot(snapshot) : [];
    setError(null);
    setEditNotice(null);
    setKeyWriteErrorCode(null);
    setRestoreReport(null);
    setRestoreNotice(null);
    if (forceDiscard) setChangedKeys(new Set());
    setPicker(null);
    encoderBaselineRef.current.clear();
    setChangedEncoderTiles(new Set());
    setFlashEncoderTiles(new Map());
    setEditState((current) => ({ ...current, operation: "setting", problem: null }));
    try {
      const result = applyBehaviorLabelPatches(
        await studioBeginEdit(selected.id, forceDiscard, labelPatches),
        labelPatches,
      );
      setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: result }));
      if (catalog.length === 0) setCatalog(await studioKeyCatalog());
      const dirty = await studioHasUnsaved(selected.id).catch(() => false);
      setEditHostLinkUid(configRpcCapable ? hostLinkUid : null);
      setEditState({ mode: "editing", dirty, operation: "idle", problem: null });
    } catch (e) {
      const code = String(e);
      if (code === "unsaved_changes_exist") {
        const discard = window.confirm(t("keymap.edit.confirm_discard_switch"));
        if (discard) {
          try {
            const result = applyBehaviorLabelPatches(
              await studioBeginEdit(selected.id, true, labelPatches),
              labelPatches,
            );
            setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: result }));
            if (catalog.length === 0) setCatalog(await studioKeyCatalog());
            const dirty = await studioHasUnsaved(selected.id).catch(() => false);
            setEditHostLinkUid(configRpcCapable ? hostLinkUid : null);
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
  }, [catalog.length, configRpcCapable, hostLinkUid, mapEditProblem, selected, setSnapshotsByDeviceId, snapshot, t]);

  // Saves the normal-key (Studio RPC) and encoder (Config RPC) sides in a
  // single Rust-side command that composes both outcomes into a structured
  // result: a failure on one side must not strand the other, and only the
  // failed side keeps its dirty state for the next retry
  // (keymap-encoder-editing-plan.md).
  const saveEdit = useCallback(async () => {
    if (!selected) return false;
    setEditNotice(null);
    setKeyWriteErrorCode(null);
    setEncoderWriteErrorCode(null);
    setEditState((current) => ({ ...current, operation: "saving", problem: null }));
    let result: SaveOrDiscardResultDto;
    try {
      result = await studioSaveChanges(
        selected.id,
        editHostLinkUid ?? (configRpcCapable ? hostLinkUid : null),
      );
    } catch (e) {
      const code = String(e);
      setEditState((current) => ({ ...current, operation: "idle", problem: mapEditProblem(code) ?? "save_failed" }));
      setError(errorLabel(code, t));
      return false;
    }
    // Success includes "skipped" (nothing to save).
    if (result.studio.success) {
      setEditState((current) => ({ ...current, dirty: false }));
    }
    const encoderFeature = result.config.results.find((r) => r.feature === "ENCODER");
    if (encoderFeature ? encoderFeature.success : result.config.skipped) {
      setEncoderDirty(false);
    }
    if (encoderFeature?.attempted && encoderFeature.success) {
      setEncoderRefreshNonce((nonce) => nonce + 1);
    }
    if (result.overall_success) {
      setEditState((current) => ({ ...current, operation: "idle", problem: null }));
      setRestoreReport(null);
      setRestoreNotice(null);
      setChangedKeys(new Set());
      encoderBaselineRef.current.clear();
      setChangedEncoderTiles(new Set());
      setEditNotice("saved");
      return true;
    }
    const problem = !result.studio.success
      ? mapEditProblem(result.studio.error ?? "save_failed") ?? "save_failed"
      : "save_failed";
    setEditState((current) => ({ ...current, operation: "idle", problem }));
    const encoderError = encoderFeature && !encoderFeature.success ? encoderFeature.error : null;
    setError(
      [
        !result.studio.success ? errorLabel(result.studio.error ?? "save_failed", t) : null,
        encoderError
          ? `${t("keymap.error.encoder_save_failed")} (${errorLabel(encoderError, t)})`
          : null,
      ]
        .filter((message): message is string => message !== null)
        .join(" / ")
    );
    return false;
  }, [configRpcCapable, editHostLinkUid, encoderDirty, hostLinkUid, mapEditProblem, selected, t]);

  // Same composition as saveEdit: discard normal keys and encoder overrides
  // together via a single command, then re-sync the encoder snapshot so the
  // UI matches the device state after DISCARD.
  const discardEdit = useCallback(async () => {
    if (!selected) return false;
    setEditNotice(null);
    setKeyWriteErrorCode(null);
    setEncoderWriteErrorCode(null);
    setEditState((current) => ({ ...current, operation: "discarding", problem: null }));
    let result: DiscardChangesDto;
    try {
      result = await studioDiscardChanges(
        selected.id,
        editHostLinkUid ?? (configRpcCapable ? hostLinkUid : null),
      );
    } catch (e) {
      setEditState((current) => ({ ...current, operation: "idle" }));
      setError(errorLabel(String(e), t));
      return false;
    }
    if (result.result.studio.success && result.snapshot) {
      const snapshot = result.snapshot;
      setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: snapshot }));
      setPicker(null);
      setEditState((current) => ({ ...current, dirty: false }));
    }
    const encoderFeature = result.result.config.results.find((r) => r.feature === "ENCODER");
    if (encoderFeature?.success) {
      setEncoderDirty(false);
    }
    if (encoderFeature?.attempted) {
      // Re-sync after DISCARD regardless of outcome, same as the panel close.
      setEncoderPanel(null);
      setEncoderRefreshNonce((nonce) => nonce + 1);
    }
    if (result.result.overall_success) {
      setEditState((current) => ({ ...current, operation: "idle", problem: null }));
      setRestoreReport(null);
      setRestoreNotice(null);
      setChangedKeys(new Set());
      encoderBaselineRef.current.clear();
      setChangedEncoderTiles(new Set());
      setFlashEncoderTiles(new Map());
      setEditNotice("discarded");
      return true;
    }
    setEditState((current) => ({ ...current, operation: "idle" }));
    const encoderError = encoderFeature && !encoderFeature.success ? encoderFeature.error : null;
    setError(
      [
        !result.result.studio.success
          ? errorLabel(result.result.studio.error ?? "discard_failed", t)
          : null,
        encoderError
          ? `${t("keymap.error.encoder_discard_failed")} (${errorLabel(encoderError, t)})`
          : null,
      ]
        .filter((message): message is string => message !== null)
        .join(" / ")
    );
    return false;
  }, [configRpcCapable, editHostLinkUid, encoderDirty, hostLinkUid, selected, setSnapshotsByDeviceId, t]);

  const endEdit = useCallback(async () => {
    if (!selected) return;
    let studioDirty = editState.dirty;
    if (studioDirty) studioDirty = await studioHasUnsaved(selected.id).catch(() => true);
    if (studioDirty || encoderDirty) {
      const discard = window.confirm(t("keymap.edit.confirm_discard_end"));
      if (!discard) return;
      const discarded = await discardEdit();
      if (!discarded) return;
    }
    setEditState((current) => ({ ...current, operation: "ending" }));
    try {
      await studioEndEdit(selected.id);
      setPicker(null);
      setEncoderPanel(null);
      setRestoreReport(null);
      setRestoreNotice(null);
      setChangedKeys(new Set());
      encoderBaselineRef.current.clear();
      setChangedEncoderTiles(new Set());
      setFlashEncoderTiles(new Map());
      setEditHostLinkUid(null);
      setEditState({ mode: "viewing", dirty: false, operation: "idle", problem: null });
    } catch (e) {
      const code = String(e);
      setEditState((current) => ({ ...current, operation: "idle" }));
      setError(errorLabel(code, t));
    }
  }, [discardEdit, editState.dirty, encoderDirty, selected, t]);

  const selectStudioDevice = useCallback(async (nextId: string) => {
    if (nextId === selectedId) return;
    if (editState.mode === "editing" && selected) {
      if (editState.dirty || encoderDirty) {
        if (!window.confirm(t("keymap.edit.confirm_discard_switch"))) return;
        if (!(await discardEdit())) return;
      }
      try {
        await studioEndEdit(selected.id);
      } catch (error) {
        setError(errorLabel(String(error), t));
        return;
      }
      setEditHostLinkUid(null);
      setEditState({ mode: "viewing", dirty: false, operation: "idle", problem: null });
    }
    setSelectedId(nextId);
  }, [discardEdit, editState.dirty, editState.mode, encoderDirty, selected, selectedId, t]);

  const canLeaveForNavigation = useCallback(
    () => pendingKeyWrites === 0 && pendingEncoderWrites === 0 && editState.operation === "idle",
    [editState.operation, pendingEncoderWrites, pendingKeyWrites],
  );

  const hasUnsavedForNavigation = useCallback(async () => {
    if (!selected || editState.mode !== "editing") return false;
    if (pendingKeyWrites > 0 || pendingEncoderWrites > 0) return true;
    if (encoderDirty) return true;
    if (editState.dirty) {
      return await studioHasUnsaved(selected.id).catch(() => true);
    }
    return await studioHasUnsaved(selected.id).catch(() => false);
  }, [editState.dirty, editState.mode, encoderDirty, pendingEncoderWrites, pendingKeyWrites, selected]);

  const finishEditForNavigation = useCallback(async () => {
    if (!selected) return true;
    setEditState((current) => ({ ...current, operation: "ending" }));
    try {
      await studioEndEdit(selected.id);
      setPicker(null);
      setEncoderPanel(null);
      setRestoreReport(null);
      setRestoreNotice(null);
      setChangedKeys(new Set());
      encoderBaselineRef.current.clear();
      setChangedEncoderTiles(new Set());
      setFlashEncoderTiles(new Map());
      setEditHostLinkUid(null);
      setEditState({ mode: "viewing", dirty: false, operation: "idle", problem: null });
      return true;
    } catch (e) {
      const code = String(e);
      setEditState((current) => ({ ...current, operation: "idle" }));
      setError(errorLabel(code, t));
      return false;
    }
  }, [selected, t]);

  const saveAndLeaveForNavigation = useCallback(async () => {
    if (!canLeaveForNavigation()) return false;
    const saved = await saveEdit();
    if (!saved) return false;
    return await finishEditForNavigation();
  }, [canLeaveForNavigation, finishEditForNavigation, saveEdit]);

  const discardAndLeaveForNavigation = useCallback(async () => {
    if (!canLeaveForNavigation()) return false;
    const discarded = await discardEdit();
    if (!discarded) return false;
    return await finishEditForNavigation();
  }, [canLeaveForNavigation, discardEdit, finishEditForNavigation]);

  useEffect(() => {
    if (!onRegisterNavigationGuard) return undefined;
    onRegisterNavigationGuard({
      hasUnsaved: hasUnsavedForNavigation,
      canLeave: canLeaveForNavigation,
      saveAndLeave: saveAndLeaveForNavigation,
      discardAndLeave: discardAndLeaveForNavigation,
    });
    return () => onRegisterNavigationGuard(null);
  }, [
    canLeaveForNavigation,
    discardAndLeaveForNavigation,
    hasUnsavedForNavigation,
    onRegisterNavigationGuard,
    saveAndLeaveForNavigation,
  ]);

  const exportKeymap = useCallback(async () => {
    if (!selected || keymapFileBusy || pendingKeyWrites > 0 || editState.operation !== "idle") return;
    setError(null);
    setRestoreReport(null);
    setRestoreNotice(null);
    setChangedKeys(new Set());
    const encoderExportUnavailable =
      encoderLoadState === "loading" ||
      encoderLoadState === "error" ||
      (encoderLoadState === "available" && (encoderCount ?? 0) > 0 && hostLinkUid === null);
    if (encoderExportUnavailable) {
      setError(t("keymap.error.encoder_export_unavailable"));
      return;
    }
    const defaultName = `${selected.display_name.replace(/[\\/:*?"<>|]+/g, "-")}-keymap.json`;
    const path = await saveDialog({
      defaultPath: defaultName,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path) return;
    setKeymapFileBusy("export");
    try {
      const encoderExportUid =
        encoderLoadState === "available" && (encoderCount ?? 0) > 0 ? hostLinkUid : null;
      await studioExportKeymap(selected.id, path, encoderExportUid);
      setEditNotice(null);
      window.alert(t("keymap.export.done"));
    } catch (e) {
      setError(errorLabel(String(e), t));
    } finally {
      setKeymapFileBusy(null);
    }
  }, [configRpcCapable, editState.operation, encoderCount, encoderLoadState, hostLinkUid, keymapFileBusy, pendingKeyWrites, selected, t]);

  const ensureRestoreEditSession = useCallback(async (): Promise<boolean> => {
    if (!selected) return false;
    const labelPatches = snapshot ? resolvedLabelPatchesFromSnapshot(snapshot) : [];
    const hasDirty =
      editState.mode === "editing" &&
      (editState.dirty || (await studioHasUnsaved(selected.id).catch(() => editState.dirty)));
    if (hasDirty) {
      const discard = window.confirm(t("keymap.restore.discard_confirm"));
      if (!discard) return false;
    }
    setEditState((current) => ({ ...current, operation: "setting", problem: null }));
    try {
      const result = applyBehaviorLabelPatches(
        await studioBeginEdit(selected.id, hasDirty, labelPatches),
        labelPatches,
      );
      setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: result }));
      if (hasDirty) setChangedKeys(new Set());
      if (catalog.length === 0) setCatalog(await studioKeyCatalog());
      setEditHostLinkUid(configRpcCapable ? hostLinkUid : null);
      setEditState({ mode: "editing", dirty: false, operation: "idle", problem: null });
      return true;
    } catch (e) {
      const code = String(e);
      setEditState((current) => ({ ...current, operation: "idle", problem: mapEditProblem(code) }));
      setError(errorLabel(code, t));
      return false;
    }
  }, [catalog.length, configRpcCapable, editState.dirty, editState.mode, hostLinkUid, mapEditProblem, selected, setSnapshotsByDeviceId, snapshot, t]);

  const restoreKeymap = useCallback(async () => {
    if (!selected || keymapFileBusy || pendingKeyWrites > 0 || editState.operation !== "idle") return;
    setError(null);
    setRestoreReport(null);
    setRestoreNotice(null);
    setChangedKeys(new Set());
    const selectedPath = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!selectedPath || Array.isArray(selectedPath)) return;
    setKeymapFileBusy("restore");
    try {
      const ready = await ensureRestoreEditSession();
      if (!ready) return;
      const encoderRestoreUid = configRpcCapable && (encoderCount ?? 0) > 0 ? hostLinkUid : null;
      const preview = await studioPreviewKeymapRestore(selected.id, selectedPath, encoderRestoreUid);
      if (
        preview.can_apply &&
        preview.will_write === 0 &&
        preview.blocked === 0 &&
        preview.encoder_will_write === 0 &&
        preview.encoder_blocked === 0
      ) {
        setRestoreReport(preview);
        setRestoreNotice("no_targets");
        setEditState({ mode: "editing", dirty: false, operation: "idle", problem: null });
        return;
      }
      const confirmText = restoreConfirmText(preview, t);
      if (!preview.can_apply || !window.confirm(confirmText)) {
        setRestoreReport(preview);
        return;
      }
      const [nextSnapshot, report] = await studioApplyKeymapRestore(selected.id, selectedPath, encoderRestoreUid);
      setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: nextSnapshot }));
      setRestoreReport(report);
      setChangedKeys(new Set(report.applied_keys.map((key) => changedKeyId(key.layer_index, key.position))));
      setChangedEncoderTiles(new Set(report.applied_encoders.map((item) => {
        const restoredLayer = nextSnapshot.layers[item.layer_index];
        return encoderTileId(restoredLayer?.id ?? item.layer_index, item.encoder_id);
      })));
      setEditState({ mode: "editing", dirty: report.applied_keys.length > 0, operation: "idle", problem: null });
      if (
        report.applied_encoders.length > 0 ||
        report.errors.some((issue) => issue.code === "encoder_apply_failed")
      ) {
        setEncoderDirty(true);
        setEncoderRefreshNonce((nonce) => nonce + 1);
      }
      if (report.apply_status === "partial") {
        setError(t("keymap.restore.apply_partial"));
        return;
      }
      window.alert(t("keymap.restore.done"));
    } catch (e) {
      const code = String(e);
      setError(errorLabel(code, t));
    } finally {
      setKeymapFileBusy(null);
      setEditState((current) => ({ ...current, operation: "idle" }));
    }
  }, [configRpcCapable, editState.operation, encoderCount, ensureRestoreEditSession, hostLinkUid, keymapFileBusy, pendingKeyWrites, selected, setSnapshotsByDeviceId, t]);

  const setKey = useCallback((key: StudioPhysicalKey, targetLayer: StudioLayer, behavior: EditBehavior) => {
    if (!selected || editState.operation === "saving" || editState.operation === "discarding" || editState.operation === "ending") return;
    const previousSnapshot = snapshot;
    setEditNotice(null);
    setKeyWriteErrorCode(null);
    setRestoreReport(null);
    setRestoreNotice(null);
    setPicker(null);
    setPendingKeyWrites((current) => current + 1);
    setEditState((current) => ({ ...current, problem: null }));
    if (previousSnapshot) {
      const optimistic = optimisticSnapshotForSetKey(
        previousSnapshot,
        targetLayer.id,
        key.position,
        behavior,
        catalog,
      );
      setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: optimistic }));
    }
    keyWriteQueueRef.current.push({
      deviceId: selected.id,
      layerIndex: snapshot?.layers.findIndex((item) => item.id === targetLayer.id) ?? targetLayer.index,
      layerId: targetLayer.id,
      position: key.position,
      behavior,
      previousSnapshot,
    });
    void processKeyWriteQueue();
  }, [catalog, editState.operation, processKeyWriteQueue, selected, setSnapshotsByDeviceId, snapshot]);

  // Sends CW/CCW encoder bindings over Host Link Config RPC. A null side keeps
  // the current runtime override value (backend fills it in); the initial edit
  // of a `source=keymap` encoder must pass both sides (EncoderPanel enforces
  // this and never auto-completes the unset side).
  const writeEncoderBinding = useCallback((encoderId: number, cw: EditBehavior | null, ccw: EditBehavior | null) => {
    if (!selected || !hostLinkUid || !layer) return;
    if (editState.operation === "saving" || editState.operation === "discarding" || editState.operation === "ending") return;
    const deviceId = selected.id;
    const uid = hostLinkUid;
    const layerId = layer.id;
    setEncoderWriteErrorCode(null);
    setPendingEncoderWrites((current) => current + 1);
    configRpcChainRef.current = configRpcChainRef.current.then(async () => {
      try {
        const result = await studioSetEncoderBindings(deviceId, uid, layerId, encoderId, cw, ccw);
        setEncoderBindings((current) =>
          current.map((item) =>
            item.encoder_id === result.encoder_id && item.layer_id === result.layer_id ? result : item
          )
        );
        const tileId = encoderTileId(result.layer_id, result.encoder_id);
        const baseline = encoderBaselineRef.current.get(tileId);
        setChangedEncoderTiles((current) => {
          const next = new Set(current);
          if (!baseline || encoderBindingDiffers(result, baseline)) next.add(tileId);
          else next.delete(tileId);
          return next;
        });
        const flashVersion = ++encoderFlashVersionRef.current;
        setFlashEncoderTiles((current) => {
          const next = new Map(current);
          next.set(tileId, flashVersion);
          return next;
        });
        window.setTimeout(() => {
          setFlashEncoderTiles((current) => {
            if (current.get(tileId) !== flashVersion) return current;
            const next = new Map(current);
            next.delete(tileId);
            return next;
          });
        }, 1200);
        setEncoderDirty(true);
      } catch (e) {
        setEncoderWriteErrorCode(String(e));
      } finally {
        setPendingEncoderWrites((current) => Math.max(0, current - 1));
      }
    });
  }, [editState.operation, hostLinkUid, layer, selected]);

  const addLayer = useCallback(async (name: string) => {
    if (!selected || editState.operation !== "idle") return null;
    const previousLayerIds = new Set(snapshot?.layers.map((item) => item.id) ?? []);
    setEditNotice(null);
    setKeyWriteErrorCode(null);
    setEditState((current) => ({ ...current, operation: "setting", problem: null }));
    try {
      const result = await studioAddLayer(selected.id, name);
      setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: result }));
      const addedIndex = result.layers.findIndex((item) => !previousLayerIds.has(item.id));
      setActiveLayer(addedIndex >= 0 ? addedIndex : Math.max(0, result.layers.length - 1));
      setRestoreReport(null);
      setRestoreNotice(null);
      setPicker(null);
      setEncoderRefreshNonce((nonce) => nonce + 1);
      setEditState((current) => ({ ...current, dirty: true, operation: "idle", problem: null }));
      return result;
    } catch (e) {
      const code = String(e);
      const problem = mapEditProblem(code);
      setEditState((current) => ({ ...current, operation: "idle", problem }));
      setError(errorLabel(code, t));
      return null;
    }
  }, [editState.operation, mapEditProblem, selected, setSnapshotsByDeviceId, snapshot, t]);

  const renameLayer = useCallback(async (targetLayer: StudioLayer, name: string) => {
    if (!selected || editState.operation !== "idle") return null;
    setEditNotice(null);
    setKeyWriteErrorCode(null);
    setEditState((current) => ({ ...current, operation: "setting", problem: null }));
    try {
      const result = await studioRenameLayer(selected.id, targetLayer.id, name);
      setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: result }));
      const nextIndex = result.layers.findIndex((item) => item.id === targetLayer.id);
      if (nextIndex >= 0) setActiveLayer(nextIndex);
      setRestoreReport(null);
      setRestoreNotice(null);
      setPicker(null);
      setEditState((current) => ({ ...current, dirty: true, operation: "idle", problem: null }));
      return result;
    } catch (e) {
      const code = String(e);
      const problem = mapEditProblem(code);
      setEditState((current) => ({ ...current, operation: "idle", problem }));
      setError(errorLabel(code, t));
      return null;
    }
  }, [editState.operation, mapEditProblem, selected, setSnapshotsByDeviceId, t]);

  const removeLayer = useCallback(async (targetLayer: StudioLayer) => {
    if (!selected || editState.operation !== "idle") return null;
    setEditNotice(null);
    setKeyWriteErrorCode(null);
    setEditState((current) => ({ ...current, operation: "setting", problem: null }));
    try {
      const result = await studioRemoveLayer(selected.id, targetLayer.index);
      setSnapshotsByDeviceId((current) => ({ ...current, [selected.id]: result }));
      setActiveLayer((current) => Math.min(current, Math.max(0, result.layers.length - 1)));
      setRestoreReport(null);
      setRestoreNotice(null);
      setPicker(null);
      setEncoderRefreshNonce((nonce) => nonce + 1);
      setEditState((current) => ({ ...current, dirty: true, operation: "idle", problem: null }));
      return result;
    } catch (e) {
      const code = String(e);
      const problem = mapEditProblem(code);
      setEditState((current) => ({ ...current, operation: "idle", problem }));
      setError(errorLabel(code, t));
      return null;
    }
  }, [editState.operation, mapEditProblem, selected, setSnapshotsByDeviceId, t]);

  useEffect(() => {
    setPicker(null);
    setEncoderPanel(null);
  }, [selectedId, viewMode, activeLayer]);

  useEffect(() => {
    setRestoreReport(null);
    setRestoreNotice(null);
    setChangedKeys(new Set());
  }, [selectedId]);

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
  const keyWritesPending = pendingKeyWrites > 0 || pendingEncoderWrites > 0;
  const structuralEditBusy = busy || keyWritesPending || editState.operation !== "idle";
  const viewerAvailable = selected?.keymap_viewer_status === "available";
  const selectedLocked = selected?.keymap_viewer_status === "locked" || selected?.lock_state === "locked";
  const fileOperationBusy = keymapFileBusy !== null || keyWritesPending || editState.operation !== "idle";
  const encoderExportReady =
    encoderLoadState === "unsupported" ||
    (encoderLoadState === "available" && ((encoderCount ?? 0) === 0 || hostLinkUid !== null));
  const canExport = !!selected && !!snapshot && !busy && !fileOperationBusy && encoderExportReady;
  const canRestore = !!selected && viewerAvailable && !selectedLocked && !busy && !fileOperationBusy;

  return (
    <div className="p-6 w-full space-y-5">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-medium text-ink">{t("keymap.title")}</h1>
          <p className="mt-0.5 text-sm text-muted">{t("keymap.subtitle")}</p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => void exportKeymap()}
            disabled={!canExport}
            title={t("keymap.export")}
            className="btn-neu flex items-center gap-2 rounded-full px-4 py-2.5 text-sm font-medium text-ink disabled:opacity-60"
          >
            <Download size={15} />
            {keymapFileBusy === "export" ? t("keymap.exporting") : t("keymap.export")}
          </button>
          <button
            onClick={() => void restoreKeymap()}
            disabled={!canRestore}
            title={t("keymap.restore")}
            className="btn-neu flex items-center gap-2 rounded-full px-4 py-2.5 text-sm font-medium text-ink disabled:opacity-60"
          >
            <Upload size={15} />
            {keymapFileBusy === "restore" ? t("keymap.restoring") : t("keymap.restore")}
          </button>
          <button
            onClick={refresh}
            disabled={busy || editing}
            className="btn-neu flex items-center gap-2 rounded-full px-4 py-2.5 text-sm font-medium text-ink disabled:opacity-60"
          >
            <RefreshCw size={15} className={busy ? "animate-spin" : ""} />
            {t("keymap.refresh")}
          </button>
        </div>
      </div>

      {(error || studioError) && <Notice>{error ?? friendlyError(studioError, t)}</Notice>}
      {restoreNotice === "no_targets" && (
        <InfoNotice>{t("keymap.restore.no_targets")}</InfoNotice>
      )}
      {restoreReport && restoreReportNotice(restoreReport, t) && (
        <WarnNotice>{restoreReportNotice(restoreReport, t)}</WarnNotice>
      )}

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
                  onClick={() => void selectStudioDevice(device.id)}
                  className={`min-h-[4.75rem] min-w-64 max-w-72 rounded-pill px-3 py-3 text-left transition-colors ${
                    selectedId === device.id
                      ? "bg-plate shadow-neu-sel-in"
                      : "bg-surface ring-1 ring-border hover:ring-disabled"
                  } relative`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className={`flex min-w-0 items-center gap-1.5 text-sm font-medium ${selectedId === device.id ? "text-accent-deep" : "text-ink"}`}>
                      <span className="truncate">{device.display_name}</span>
                    </span>
                    <span className="flex shrink-0 items-center gap-1.5">
                      <StudioConnectionBadge device={device} />
                      <StudioStatusBadge device={device} />
                    </span>
                  </div>
                  {device.connection_type === "ble_studio" ? (
                    <div className="mt-1 h-4" aria-hidden="true" />
                  ) : (
                    <div className="mt-1 truncate font-mono text-[11px] text-muted">{device.port_name}</div>
                  )}
                </button>
              ))}
            </div>
          )}
        </section>

        <section
          className={`w-full overflow-hidden rounded-card bg-surface p-5 space-y-4 ${editing ? "pb-24" : ""}`}
        >
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
                  keyStyle={changedKeyStyle}
                  flashKeys={flashKeys}
                  editing={editing}
                  hasChangedKeys={changedKeys.size > 0 || changedEncoderTiles.size > 0}
                  editBusy={structuralEditBusy}
                  editAvailable={viewerAvailable && !selectedLocked}
                  onToggleEdit={() => editing ? void endEdit() : void beginEdit(false)}
                  onAddLayer={addLayer}
                  onRenameLayer={renameLayer}
                  onRemoveLayer={removeLayer}
                  onKeyClick={editing ? (key, element) => {
                    if (!layer || editState.operation === "saving" || editState.operation === "discarding" || editState.operation === "ending") return;
                    const rect = element.getBoundingClientRect();
                    setPicker({
                      key,
                      layer,
                      rect: { left: rect.left, top: rect.top, width: rect.width, height: rect.height },
                    });
                  } : undefined}
                  encoderBindings={encoderBindings}
                  encoderLoadState={encoderLoadState}
                  encoderError={encoderError}
                  changedEncoderTiles={changedEncoderTiles}
                  flashEncoderTiles={flashEncoderTiles}
                  onRetryEncoders={() => setEncoderRefreshNonce((nonce) => nonce + 1)}
                  onEncoderClick={encoderEditAvailable ? (binding, element) => {
                    if (editState.operation === "saving" || editState.operation === "discarding" || editState.operation === "ending") return;
                    const rect = element.getBoundingClientRect();
                    setEncoderPanel({
                      encoderId: binding.encoder_id,
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
          dirty={editState.dirty || encoderDirty}
          operation={editState.operation}
          pendingCount={pendingKeyWrites + pendingEncoderWrites}
          problem={editState.problem}
          keyWriteError={keyWriteErrorCode ? errorLabel(keyWriteErrorCode, t) : null}
          encoderWriteError={encoderWriteErrorCode ? errorLabel(encoderWriteErrorCode, t) : null}
          notice={editNotice}
          onResyncKey={resyncKeyState}
          onResyncEncoder={resyncEncoderState}
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
          busy={editState.operation === "saving" || editState.operation === "discarding" || editState.operation === "ending"}
          onClose={() => setPicker(null)}
          onSelect={(behavior) => void setKey(picker.key, picker.layer, behavior)}
        />
      )}
      {encoderPanel && (() => {
        const panelBinding = encoderBindings.find((item) => item.encoder_id === encoderPanel.encoderId);
        if (!panelBinding) return null;
        return (
          <EncoderPanel
            binding={panelBinding}
            rect={encoderPanel.rect}
            catalog={catalog}
            layers={snapshot?.layers ?? []}
            busy={
              pendingEncoderWrites > 0 ||
              editState.operation === "saving" ||
              editState.operation === "discarding" ||
              editState.operation === "ending"
            }
            onClose={() => setEncoderPanel(null)}
            onWrite={(cw, ccw) => writeEncoderBinding(panelBinding.encoder_id, cw, ccw)}
          />
        );
      })()}
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

function KeymapContent({
  snapshot,
  activeLayer,
  setActiveLayer,
  layer,
  reportedLayerIndex,
  keyStyle,
  flashKeys,
  marquee,
  onKeyClick,
  editing = false,
  hasChangedKeys = false,
  editBusy = false,
  editAvailable = false,
  onToggleEdit,
  onAddLayer,
  onRenameLayer,
  onRemoveLayer,
  encoderBindings = null,
  encoderLoadState = "unsupported",
  encoderError = null,
  changedEncoderTiles = new Set(),
  flashEncoderTiles = new Map(),
  onRetryEncoders,
  onEncoderClick,
}: {
  snapshot: StudioKeymapSnapshot;
  activeLayer: number;
  setActiveLayer: (value: number) => void;
  layer: StudioLayer | null;
  /** Layer the keyboard itself reports as active (LAYER_STATE uplink), or null. */
  reportedLayerIndex: number | null;
  keyStyle?: (key: StudioPhysicalKey) => CSSProperties | undefined;
  flashKeys?: Map<string, number>;
  onKeyClick?: (key: StudioPhysicalKey, element: HTMLDivElement) => void;
  /** Optional element shown to the right of the layer tabs (tester typed-char marquee). */
  marquee?: ReactNode;
  editing?: boolean;
  hasChangedKeys?: boolean;
  editBusy?: boolean;
  editAvailable?: boolean;
  onToggleEdit?: () => void;
  onAddLayer?: (name: string) => Promise<StudioKeymapSnapshot | null>;
  onRenameLayer?: (layer: StudioLayer, name: string) => Promise<StudioKeymapSnapshot | null>;
  onRemoveLayer?: (layer: StudioLayer) => Promise<StudioKeymapSnapshot | null>;
  /** null when the connected device does not advertise CONFIG_RPC (encoders never shown). */
  encoderBindings?: EncoderBindingsDto[] | null;
  encoderLoadState?: EncoderLoadState;
  encoderError?: string | null;
  changedEncoderTiles?: Set<string>;
  flashEncoderTiles?: Map<string, number>;
  onRetryEncoders?: () => void;
  /** Set while encoder editing is available; makes encoder tiles clickable. */
  onEncoderClick?: (binding: EncoderBindingsDto, element: HTMLDivElement) => void;
}) {
  const { t } = useLang();
  const layerTabRefs = useRef(new Map<number, HTMLDivElement>());
  const editInputRef = useRef<HTMLInputElement | null>(null);
  const [editingLayerId, setEditingLayerId] = useState<number | null>(null);
  const [draftLayerName, setDraftLayerName] = useState("");
  const bindingsByPosition = useMemo(() => {
    const map = new Map<number, StudioBinding>();
    if (layer) for (const binding of layer.bindings) map.set(binding.position, binding);
    return map;
  }, [layer]);

  useEffect(() => {
    const active = snapshot.layers[activeLayer];
    if (!active) return;
    layerTabRefs.current.get(active.id)?.scrollIntoView({
      block: "nearest",
      inline: "nearest",
    });
  }, [activeLayer, snapshot.layers]);

  useEffect(() => {
    if (!editingLayerId) return;
    window.setTimeout(() => {
      editInputRef.current?.focus();
      editInputRef.current?.select();
    }, 0);
  }, [editingLayerId]);

  const beginLayerRename = (target: StudioLayer) => {
    setActiveLayer(snapshot.layers.findIndex((item) => item.id === target.id));
    setEditingLayerId(target.id);
    setDraftLayerName(target.name);
  };

  const commitLayerRename = async (target: StudioLayer) => {
    const name = draftLayerName.trim();
    if (!name || name === target.name) {
      setEditingLayerId(null);
      setDraftLayerName("");
      return;
    }
    const result = await onRenameLayer?.(target, name);
    if (result) {
      setEditingLayerId(null);
      setDraftLayerName("");
    }
  };

  const addLayer = async () => {
    if (!onAddLayer) return;
    const previousIds = new Set(snapshot.layers.map((item) => item.id));
    const nextNumber = Math.max(-1, ...snapshot.layers.map((item) => item.index)) + 1;
    const result = await onAddLayer(`Layer ${nextNumber}`);
    const added = result?.layers.find((item) => !previousIds.has(item.id));
    if (added) {
      setEditingLayerId(added.id);
      setDraftLayerName(added.name);
    }
  };

  const removeLayer = async (target: StudioLayer) => {
    if (!onRemoveLayer || snapshot.layers.length <= 1) return;
    if (!window.confirm(t("keymap.edit.confirm_delete_layer"))) return;
    const result = await onRemoveLayer(target);
    if (result) {
      setEditingLayerId(null);
      setDraftLayerName("");
    }
  };

  return (
    <div className="min-w-0 space-y-4">
      <div className="flex min-w-0 items-start gap-2">
        {/* pt-2/pr-2: room for the live-layer dot (-top-1 -right-1) and its
            pulse ring so overflow-x-auto doesn't clip them into a half-circle. */}
        <div className="min-w-0 flex-1 overflow-x-auto pt-2 pr-2 pb-1">
          <div className="flex w-max gap-2">
          {snapshot.layers.map((item, index) => {
            const live = reportedLayerIndex !== null && item.index === reportedLayerIndex;
            const active = activeLayer === index;
            const renaming = editingLayerId === item.id;
            return (
              <div
                key={item.id}
                ref={(node) => {
                  if (node) layerTabRefs.current.set(item.id, node);
                  else layerTabRefs.current.delete(item.id);
                }}
                className={`relative inline-flex items-center rounded-pill text-sm font-medium ring-1 transition-colors ${
                  active ? "bg-plate text-accent ring-transparent shadow-neu-sel-in" : "bg-background text-muted ring-border hover:bg-plate hover:text-ink"
                }`}
              >
                {renaming ? (
                  <input
                    ref={editInputRef}
                    value={draftLayerName}
                    disabled={editBusy}
                    onChange={(event) => setDraftLayerName(event.target.value)}
                    onBlur={() => void commitLayerRename(item)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") void commitLayerRename(item);
                      if (event.key === "Escape") {
                        setEditingLayerId(null);
                        setDraftLayerName("");
                      }
                    }}
                    className="w-28 rounded-pill bg-background px-3 py-1.5 text-sm font-medium text-ink outline-none ring-1 ring-border"
                  />
                ) : (
                  <button
                    type="button"
                    onClick={() => setActiveLayer(index)}
                    title={live ? t("keymap.active_layer") : undefined}
                    className="px-3 py-1.5"
                  >
                    {item.name}
                  </button>
                )}
                {editing && active && !renaming && (
                  <div className="flex items-center pr-1">
                    <button
                      type="button"
                      disabled={editBusy}
                      onClick={() => beginLayerRename(item)}
                      title={t("keymap.edit.rename_layer")}
                      className="rounded-full p-1 text-faint hover:bg-background hover:text-ink disabled:opacity-50"
                    >
                      <Pencil size={12} />
                    </button>
                    <button
                      type="button"
                      disabled={editBusy || snapshot.layers.length <= 1}
                      onClick={() => void removeLayer(item)}
                      title={t("keymap.edit.delete_layer")}
                      className="rounded-full p-1 text-faint hover:bg-background hover:text-red-700 disabled:opacity-50"
                    >
                      <Trash2 size={12} />
                    </button>
                  </div>
                )}
                {live && (
                  <span className="animate-layer-pulse absolute -right-1 -top-1 h-2.5 w-2.5 rounded-full bg-accent ring-2 ring-white" />
                )}
              </div>
            );
          })}
          </div>
        </div>
        {editing && hasChangedKeys && (
          <div className="flex shrink-0 items-center gap-1.5 pt-2 text-xs font-medium text-muted">
            <span className="h-3 w-3 rounded-sm bg-accent-soft ring-1 ring-accent/70" aria-hidden="true" />
            <span>{t("keymap.changed_key_legend")}</span>
          </div>
        )}
        {editAvailable && onToggleEdit && (
          /* pt-2 matches the layer-tab scroller above so this control row stays
             vertically aligned with the tabs (parent is items-start). */
          <div className="flex shrink-0 items-center gap-1 pt-2">
            {editing && (
              <button
                type="button"
                disabled={editBusy}
                onClick={() => void addLayer()}
                title={t("keymap.edit.add_layer")}
                className="btn-neu flex items-center gap-1.5 rounded-full px-3 py-1.5 text-sm font-medium text-ink disabled:opacity-60"
              >
                <Plus size={14} />
                {t("keymap.edit.add_layer_short")}
              </button>
            )}
            <button
              type="button"
              onClick={onToggleEdit}
              disabled={editBusy}
              className={`btn-neu flex items-center gap-1.5 rounded-full px-3 py-1.5 text-sm font-medium disabled:opacity-60 ${
                editing ? "text-accent-deep" : "text-ink"
              }`}
            >
              <Pencil size={14} />
              {editing ? t("keymap.edit.on") : t("keymap.edit")}
            </button>
          </div>
        )}
        </div>
        {marquee}
      {!layer || snapshot.selected_layout_keys.length === 0 ? (
        <EmptyState icon={<Keyboard size={32} />} title={t("keymap.empty_keymap_title")} body={t("keymap.empty_keymap_body")} />
      ) : (
        <KeymapCanvas
          keys={snapshot.selected_layout_keys}
          keyTitle={(key) => bindingsByPosition.get(key.position)?.full_label ?? "--"}
          keyStyle={keyStyle}
          onKeyClick={onKeyClick}
          footer={
            <>
              {encoderLoadState === "error" && (
                <div className="mt-3 flex items-center gap-2 text-xs text-danger">
                  <span>{encoderError ?? t("keymap.error.encoder_read_failed")}</span>
                  <button
                    type="button"
                    onClick={onRetryEncoders}
                    className="flex items-center gap-1 rounded-pill bg-red-50 px-2 py-1 font-medium text-red-700 ring-1 ring-red-100"
                  >
                    <RefreshCw size={12} />
                    {t("keymap.encoder.retry")}
                  </button>
                </div>
              )}
              {encoderLoadState === "loading" && (!encoderBindings || encoderBindings.length === 0) && (
                <div className="mt-3 flex items-center gap-1.5 text-xs text-faint">
                  <RefreshCw size={12} className="animate-spin" />
                  {t("keymap.encoder.loading")}
                </div>
              )}
              {encoderBindings && encoderBindings.length > 0 && (
                // The plan places encoders bottom-left of the keymap plate, in
                // encoder id order (no physical-position metadata in MVP).
                <div className="mt-3 flex flex-wrap justify-start gap-3">
                  {encoderBindings.map((binding) => (
                    (() => {
                      const tileId = encoderTileId(binding.layer_id, binding.encoder_id);
                      const tileChanged = changedEncoderTiles.has(tileId);
                      const flashVersion = flashEncoderTiles.get(tileId);
                      const disabled = encoderLoadState !== "available";
                      return (
                    <div
                      key={binding.encoder_id}
                      title={t("keymap.encoder.title", { n: binding.encoder_id + 1 })}
                      onClick={(event) => !disabled && onEncoderClick?.(binding, event.currentTarget)}
                      className={`relative flex h-16 w-16 shrink-0 flex-col items-center justify-center rounded-full bg-surface px-1.5 text-center ${disabled ? "opacity-50" : onEncoderClick ? "cursor-pointer hover:ring-2 hover:ring-accent" : ""}`}
                      style={tileChanged ? {
                        backgroundColor: "rgb(var(--accent-rgb) / 0.18)",
                        boxShadow: "inset 0 0 0 2px rgb(var(--accent-rgb) / 0.72)",
                      } : undefined}
                    >
                      {(binding.stale_saved_exists || binding.invalid_saved_exists) && (
                        <span
                          className="absolute -right-0.5 -top-0.5 h-2.5 w-2.5 rounded-full bg-amber-400 ring-2 ring-white"
                          title={t(binding.stale_saved_exists ? "keymap.encoder.stale_saved" : "keymap.encoder.invalid_saved")}
                        />
                      )}
                      {binding.source === "keymap" ? (
                        <div className="text-[9px] leading-tight text-faint">
                          {t("keymap.encoder.using_keymap")}
                        </div>
                      ) : (
                        <>
                          <div
                            className="w-full truncate text-[10px] font-medium leading-tight text-ink"
                            title={binding.cw.label?.full_label ?? undefined}
                          >
                            {binding.cw.label?.full_label ?? "--"}
                          </div>
                          <div
                            className="w-full truncate text-[10px] font-medium leading-tight text-ink"
                            title={binding.ccw.label?.full_label ?? undefined}
                          >
                            {binding.ccw.label?.full_label ?? "--"}
                          </div>
                        </>
                      )}
                      <div className="mt-0.5 font-mono text-[9px] leading-none text-faint">
                        {t("keymap.encoder.title", { n: binding.encoder_id + 1 })}
                      </div>
                      {flashVersion !== undefined && (
                        <div key={flashVersion} className="pointer-events-none absolute inset-0 flex items-center justify-center">
                          <div
                            className="animate-key-check-flash flex h-6 w-6 items-center justify-center rounded-full"
                            style={{ backgroundColor: "rgb(var(--accent-rgb))" }}
                          >
                            <span className="text-base font-bold leading-none text-white">✓</span>
                          </div>
                        </div>
                      )}
                    </div>
                      );
                    })()
                  ))}
                </div>
              )}
            </>
          }
          keyContent={(key) => {
            const binding = bindingsByPosition.get(key.position);
            const flashVersion = flashKeys?.get(`${activeLayer}:${key.position}`);
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
                {flashVersion !== undefined && (
                  <div key={flashVersion} className="pointer-events-none absolute inset-0 flex items-center justify-center">
                    <div
                      className="animate-key-check-flash flex h-6 w-6 items-center justify-center rounded-full"
                      style={{ backgroundColor: "rgb(var(--accent-rgb))" }}
                    >
                      <span className="text-base font-bold leading-none text-white">✓</span>
                    </div>
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

function EditBar({ dirty, operation, pendingCount, problem, keyWriteError, encoderWriteError, notice, onResyncKey, onResyncEncoder, onSave, onDiscard, onEnd }: {
  dirty: boolean;
  operation: EditState["operation"];
  pendingCount: number;
  problem: EditState["problem"];
  keyWriteError: string | null;
  encoderWriteError: string | null;
  notice: "saved" | "discarded" | null;
  onResyncKey: () => void;
  onResyncEncoder: () => void;
  onSave: () => void;
  onDiscard: () => void;
  onEnd: () => void;
}) {
  const { t } = useLang();
  const pending = pendingCount > 0;
  const busy = operation !== "idle" || pending;
  const message = keyWriteError
    ? t("keymap.edit.key_write_failed")
    : encoderWriteError
    ? t("keymap.edit.encoder_write_failed")
    : problem
    ? t(`keymap.edit.problem.${problem}` as TranslationKey)
    : notice
      ? t(`keymap.edit.${notice}` as TranslationKey)
    : dirty
      ? t("keymap.edit.dirty")
      : "";
  return (
    <div className="fixed bottom-4 left-1/2 z-40 flex w-[min(720px,calc(100vw-32px))] -translate-x-1/2 flex-wrap items-center justify-between gap-3 rounded-card bg-surface px-4 py-3 shadow-neu-up ring-1 ring-border">
      <div className={`flex min-h-5 min-w-0 flex-1 flex-wrap items-center gap-2 text-sm font-medium ${problem || keyWriteError || encoderWriteError ? "text-red-700" : "text-muted"}`}>
        {message && <span>{message}</span>}
        {!keyWriteError && encoderWriteError && (
          <span className="text-xs font-normal text-red-600">{encoderWriteError}</span>
        )}
        {keyWriteError && (
          <>
            <span className="text-xs font-normal text-red-600">{keyWriteError}</span>
            <button
              onClick={onResyncKey}
              disabled={pending}
              className="flex items-center gap-1 rounded-pill bg-red-50 px-2 py-1 text-xs font-medium text-red-700 ring-1 ring-red-100 disabled:opacity-50"
            >
              <RefreshCw size={12} />
              {t("keymap.edit.recheck")}
            </button>
          </>
        )}
        {!keyWriteError && encoderWriteError && (
          <button
            onClick={onResyncEncoder}
            disabled={pending}
            className="flex items-center gap-1 rounded-pill bg-red-50 px-2 py-1 text-xs font-medium text-red-700 ring-1 ring-red-100 disabled:opacity-50"
          >
            <RefreshCw size={12} />
            {t("keymap.edit.recheck")}
          </button>
        )}
        {pending && (
          <span className="rounded-pill bg-accent-soft px-2 py-0.5 text-xs font-medium text-accent-deep">
            {t("keymap.edit.pending_writes", { count: pendingCount })}
          </span>
        )}
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

type PickerTab = "key" | "layer" | "tap_hold" | "bt_out" | "advanced";
type LayerBehaviorKind = "momentary_layer" | "toggle_layer" | "to_layer";
type TapHoldBehaviorKind = "mod_tap" | "layer_tap";

interface BehaviorChoice<T extends string> {
  kind: T;
  labelKey: TranslationKey;
  tooltipKey: TranslationKey;
}

interface ModifierOption {
  id: string;
  label: string;
  zmkName: string;
  baseUsage: number;
  modifierBit: number;
}

const MODIFIER_OPTIONS: ModifierOption[] = [
  { id: "lctrl", label: "LCtrl", zmkName: "LCTRL", baseUsage: 0x0007_00e0, modifierBit: 0x01 },
  { id: "lshift", label: "LShift", zmkName: "LSHIFT", baseUsage: 0x0007_00e1, modifierBit: 0x02 },
  { id: "lalt", label: "LAlt", zmkName: "LALT", baseUsage: 0x0007_00e2, modifierBit: 0x04 },
  { id: "lgui", label: "LGUI", zmkName: "LGUI", baseUsage: 0x0007_00e3, modifierBit: 0x08 },
  { id: "rctrl", label: "RCtrl", zmkName: "RCTRL", baseUsage: 0x0007_00e4, modifierBit: 0x10 },
  { id: "rshift", label: "RShift", zmkName: "RSHIFT", baseUsage: 0x0007_00e5, modifierBit: 0x20 },
  { id: "ralt", label: "RAlt", zmkName: "RALT", baseUsage: 0x0007_00e6, modifierBit: 0x40 },
  { id: "rgui", label: "RGUI", zmkName: "RGUI", baseUsage: 0x0007_00e7, modifierBit: 0x80 },
];

const LAYER_BEHAVIOR_CHOICES: BehaviorChoice<LayerBehaviorKind>[] = [
  { kind: "momentary_layer", labelKey: "keymap.edit.momentary_layer", tooltipKey: "keymap.edit.momentary_layer_tooltip" },
  { kind: "toggle_layer", labelKey: "keymap.edit.toggle_layer", tooltipKey: "keymap.edit.toggle_layer_tooltip" },
  { kind: "to_layer", labelKey: "keymap.edit.to_layer", tooltipKey: "keymap.edit.to_layer_tooltip" },
];

const TAP_HOLD_BEHAVIOR_CHOICES: BehaviorChoice<TapHoldBehaviorKind>[] = [
  { kind: "mod_tap", labelKey: "keymap.edit.mod_tap", tooltipKey: "keymap.edit.mod_tap_tooltip" },
  { kind: "layer_tap", labelKey: "keymap.edit.layer_tap", tooltipKey: "keymap.edit.layer_tap_tooltip" },
];

const BLUETOOTH_COMMANDS: Array<{
  label: string;
  title: string;
  command: number;
  value: number | null;
}> = [
  { label: "Clear", title: "&bt BT_CLR", command: 0, value: 0 },
  { label: "Next", title: "&bt BT_NXT", command: 1, value: 0 },
  { label: "Previous", title: "&bt BT_PRV", command: 2, value: 0 },
  { label: "Clear All", title: "&bt BT_CLR_ALL", command: 4, value: 0 },
  ...[0, 1, 2, 3, 4].map((profile) => ({ label: `Select ${profile}`, title: `&bt BT_SEL ${profile}`, command: 3, value: profile })),
  ...[0, 1, 2, 3, 4].map((profile) => ({ label: `Disconnect ${profile}`, title: `&bt BT_DISC ${profile}`, command: 5, value: profile })),
];

const OUTPUT_COMMANDS = [
  { label: "Toggle", title: "&out OUT_TOG", value: 0 },
  { label: "USB", title: "&out OUT_USB", value: 1 },
  { label: "BLE", title: "&out OUT_BLE", value: 2 },
  { label: "None", title: "&out OUT_NONE", value: 3 },
];

const MOUSE_BUTTON_COMMANDS = [
  { label: "Left Click", title: "&mkp LCLK", value: 0x01 },
  { label: "Right Click", title: "&mkp RCLK", value: 0x02 },
  { label: "Middle Click", title: "&mkp MCLK", value: 0x04 },
  { label: "Button 4", title: "&mkp MB4", value: 0x08 },
  { label: "Button 5", title: "&mkp MB5", value: 0x10 },
];

const MOUSE_MOVE_COMMANDS = [
  { label: "Move Up", title: "&mmv MOVE_UP", value: 0x0000_fda8 },
  { label: "Move Down", title: "&mmv MOVE_DOWN", value: 0x0000_0258 },
  { label: "Move Left", title: "&mmv MOVE_LEFT", value: 0xfda8_0000 },
  { label: "Move Right", title: "&mmv MOVE_RIGHT", value: 0x0258_0000 },
];

const MOUSE_SCROLL_COMMANDS = [
  { label: "Scroll Up", title: "&msc SCRL_UP", value: 0x0000_000a },
  { label: "Scroll Down", title: "&msc SCRL_DOWN", value: 0x0000_fff6 },
  { label: "Scroll Left", title: "&msc SCRL_LEFT", value: 0xfff6_0000 },
  { label: "Scroll Right", title: "&msc SCRL_RIGHT", value: 0x000a_0000 },
];

const UTILITY_COMMANDS: Array<{ label: string; title: string; behavior: EditBehavior }> = [
  { label: "Caps Word", title: "&caps_word", behavior: { kind: "caps_word" } },
  { label: "Key Repeat", title: "&key_repeat", behavior: { kind: "key_repeat" } },
  { label: "Grave Escape", title: "&gresc", behavior: { kind: "grave_escape" } },
];

const SYSTEM_COMMANDS: Array<{ label: string; title: string; behavior: EditBehavior }> = [
  { label: "Reset", title: "&reset", behavior: { kind: "reset" } },
  { label: "Bootloader", title: "&bootloader", behavior: { kind: "bootloader" } },
  { label: "Studio Unlock", title: "&studio_unlock", behavior: { kind: "studio_unlock" } },
];

function holdUsageFromModifiers(selectedIds: string[]): number | null {
  const selected = MODIFIER_OPTIONS.filter((option) => selectedIds.includes(option.id));
  if (selected.length === 0) return null;
  const [base, ...additional] = selected;
  const modifierBits = additional.reduce((bits, option) => bits | option.modifierBit, 0);
  return base.baseUsage | (modifierBits << 24);
}

/** Apply the selected modifiers as implicit modifier bits on a base key usage
 *  (e.g. A + LShift -> LS(A)). Returns the base usage unchanged when no
 *  modifier is selected, preserving plain &kp behavior. */
function applyModifiers(baseUsage: number, selectedIds: string[]): number {
  const bits = MODIFIER_OPTIONS
    .filter((option) => selectedIds.includes(option.id))
    .reduce((acc, option) => acc | option.modifierBit, 0);
  return bits === 0 ? baseUsage : (baseUsage | (bits << 24)) >>> 0;
}

/** Toggle a modifier id within a string[] state setter. */
function toggleIn(id: string, setter: Dispatch<SetStateAction<string[]>>) {
  setter((current) =>
    current.includes(id) ? current.filter((item) => item !== id) : [...current, id]
  );
}

/** A row of modifier toggle buttons (LCtrl..RGUI), shared by the key /
 *  tap-key / sticky-key catalogs and the Mod-Tap hold-modifier picker. */
function ModifierToggleRow({ label, selectedIds, onToggle, busy }: {
  label: string;
  selectedIds: string[];
  onToggle: (id: string) => void;
  busy: boolean;
}) {
  return (
    <div>
      <div className="mb-1.5 text-xs font-medium uppercase text-faint">{label}</div>
      <div className="flex flex-wrap gap-1.5">
        {MODIFIER_OPTIONS.map((option) => {
          const active = selectedIds.includes(option.id);
          return (
            <button
              key={option.id}
              type="button"
              disabled={busy}
              onClick={() => onToggle(option.id)}
              title={option.zmkName}
              className={`rounded-md px-2.5 py-1.5 text-sm font-medium ring-1 disabled:opacity-50 ${
                active
                  ? "bg-plate text-accent-deep ring-transparent shadow-neu-sel-in"
                  : "bg-background text-ink ring-border hover:bg-plate"
              }`}
            >
              {option.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function BindingPicker({ catalog, layers, rect, busy, onClose, onSelect, variant = "key" }: {
  catalog: KeyCatalogEntry[];
  layers: StudioLayer[];
  rect: PickerRect;
  busy: boolean;
  onClose: () => void;
  onSelect: (behavior: EditBehavior) => void;
  /** "encoder" hides everything the MVP encoder override does not allow
   *  (layer/tap-hold tabs, &trans, modifier-only keys, non-select &bt commands,
   *  utility/system/sticky behaviors); see keymap-encoder-editing-plan.md. */
  variant?: "key" | "encoder";
}) {
  const { t } = useLang();
  const isEncoder = variant === "encoder";
  const [tab, setTab] = useState<PickerTab>("key");
  const [layerBehavior, setLayerBehavior] = useState<LayerBehaviorKind | null>(null);
  const [tapHoldBehavior, setTapHoldBehavior] = useState<TapHoldBehaviorKind | null>(null);
  const [selectedModifierIds, setSelectedModifierIds] = useState<string[]>([]);
  const [selectedKeyModifierIds, setSelectedKeyModifierIds] = useState<string[]>([]);
  const [selectedTapKeyModifierIds, setSelectedTapKeyModifierIds] = useState<string[]>([]);
  const [selectedStickyKeyModifierIds, setSelectedStickyKeyModifierIds] = useState<string[]>([]);
  const [selectedTapLayerIndex, setSelectedTapLayerIndex] = useState<number | null>(null);
  const [selectedStickyLayerIndex, setSelectedStickyLayerIndex] = useState<number | null>(null);
  const [query, setQuery] = useState("");
  const queryLower = query.trim().toLowerCase();
  // Modifier-only keycodes (LCtrl..RGUI) are meaningless as a per-detent tap;
  // the Rust resolver rejects them for encoders, so don't offer them either.
  const pickerCatalog = useMemo(
    () =>
      isEncoder
        ? catalog.filter(
            (entry) => !MODIFIER_OPTIONS.some((option) => option.baseUsage === entry.hid_usage)
          )
        : catalog,
    [catalog, isEncoder]
  );
  const filtered = useMemo(() => {
    if (!queryLower) return pickerCatalog;
    return pickerCatalog.filter((entry) => {
      const usage = `0x${entry.hid_usage.toString(16)}`;
      return (
        entry.display.toLowerCase().includes(queryLower) ||
        entry.canonical.toLowerCase().includes(queryLower) ||
        usage.includes(queryLower) ||
        entry.aliases.some((alias) => alias.toLowerCase().includes(queryLower))
      );
    });
  }, [pickerCatalog, queryLower]);

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
  const selectedHoldUsage = useMemo(
    () => holdUsageFromModifiers(selectedModifierIds),
    [selectedModifierIds]
  );
  // Tap-key controls (modifier toggles + catalog) stay visible at all times but
  // are disabled until a behavior and its hold side are chosen — mirroring how
  // the catalog itself behaves.
  const tapEntriesDisabled =
    tapHoldBehavior === null ||
    (tapHoldBehavior === "mod_tap" ? selectedHoldUsage === null : selectedTapLayerIndex === null);

  useEffect(() => {
    if (selectedTapLayerIndex !== null && !layers.some((item) => item.index === selectedTapLayerIndex)) {
      setSelectedTapLayerIndex(null);
    }
    if (selectedStickyLayerIndex !== null && !layers.some((item) => item.index === selectedStickyLayerIndex)) {
      setSelectedStickyLayerIndex(null);
    }
  }, [layers, selectedStickyLayerIndex, selectedTapLayerIndex]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const renderCatalogButtons = (
    onEntrySelect: (entry: KeyCatalogEntry) => void,
    entriesDisabled = false,
  ) => (
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
                disabled={busy || entriesDisabled}
                onClick={() => onEntrySelect(entry)}
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
  );

  const renderCommandButtons = (
    commands: Array<{ label: string; title: string; behavior: EditBehavior }>,
    confirmBeforeSelect = false,
  ) => (
    <div className="flex flex-wrap gap-1.5">
      {commands.map((command) => (
        <button
          key={command.title}
          type="button"
          disabled={busy}
          onClick={() => {
            if (confirmBeforeSelect && !window.confirm(t("keymap.edit.confirm_system_behavior"))) {
              return;
            }
            onSelect(command.behavior);
          }}
          title={command.title}
          className="rounded-md bg-background px-2.5 py-1.5 text-sm font-medium text-ink ring-1 ring-border hover:bg-plate disabled:opacity-50"
        >
          {command.label}
        </button>
      ))}
    </div>
  );

  return (
    <>
      <button className="fixed inset-0 z-40 cursor-default bg-transparent" onClick={onClose} />
      <div
        className="fixed z-50 flex max-h-[min(560px,calc(100vh-32px))] w-[min(520px,calc(100vw-24px))] flex-col rounded-card bg-surface p-3 shadow-neu-up ring-1 ring-border"
        style={{ left: position.left, top: position.top }}
      >
        <div className="flex gap-1 rounded-pill bg-background p-1 ring-1 ring-border">
          {(isEncoder
            ? (["key", "bt_out", "advanced"] as const)
            : (["key", "layer", "tap_hold", "bt_out", "advanced"] as const)
          ).map((item) => (
            <button
              key={item}
              type="button"
              onClick={() => setTab(item)}
              className={`flex-1 rounded-pill px-3 py-1.5 text-sm font-medium transition-colors ${
                tab === item ? "bg-plate text-accent-deep shadow-neu-sel-in" : "text-muted hover:text-ink"
              }`}
            >
              {t(
                item === "key"
                  ? "keymap.edit.tab_key"
                  : item === "layer"
                    ? "keymap.edit.tab_layer"
                    : item === "tap_hold"
                      ? "keymap.edit.tab_tap_hold"
                      : item === "bt_out"
                        ? "keymap.edit.tab_bt_out"
                        : "keymap.edit.tab_advanced"
              )}
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
              {!isEncoder && (
                <button
                  disabled={busy}
                  onClick={() => onSelect({ kind: "transparent" })}
                  title="&trans"
                  className="flex-1 rounded-lg bg-background px-3 py-2 text-left text-sm ring-1 ring-border disabled:opacity-50"
                >
                  <div className="font-medium text-ink">{t("keymap.edit.transparent")}</div>
                  <div className="mt-0.5 text-xs text-faint">{t("keymap.edit.transparent_desc")}</div>
                </button>
              )}
              <button
                disabled={busy}
                onClick={() => onSelect({ kind: "none" })}
                title="&none"
                className="flex-1 rounded-lg bg-background px-3 py-2 text-left text-sm ring-1 ring-border disabled:opacity-50"
              >
                <div className="font-medium text-ink">{t("keymap.edit.none")}</div>
                <div className="mt-0.5 text-xs text-faint">{t("keymap.edit.none_desc")}</div>
              </button>
            </div>
            <div className="mt-3">
              <ModifierToggleRow
                label={t("keymap.edit.key_modifiers")}
                selectedIds={selectedKeyModifierIds}
                onToggle={(id) => toggleIn(id, setSelectedKeyModifierIds)}
                busy={busy}
              />
            </div>
            {renderCatalogButtons((entry) =>
              onSelect({ kind: "key_press", hid_usage: applyModifiers(entry.hid_usage, selectedKeyModifierIds) })
            )}
          </>
        ) : tab === "layer" ? (
          <div className="mt-3 min-h-0 flex-1 overflow-y-auto pr-1">
            <div className="mb-3">
              <div className="mb-1.5 text-xs font-medium uppercase text-faint">
                {t("keymap.edit.layer_behavior")}
              </div>
              <div className="flex gap-1.5">
                {LAYER_BEHAVIOR_CHOICES.map(({ kind, labelKey, tooltipKey }) => (
                  <button
                    key={kind}
                    type="button"
                    disabled={busy}
                    onClick={() => setLayerBehavior(kind)}
                    title={t(tooltipKey)}
                    className={`rounded-md px-3 py-1.5 text-sm font-medium ring-1 disabled:opacity-50 ${
                      layerBehavior === kind
                        ? "bg-plate text-accent-deep ring-transparent shadow-neu-sel-in"
                        : "bg-background text-ink ring-border hover:bg-plate"
                    }`}
                  >
                    {t(labelKey)}
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
                    disabled={busy || layerBehavior === null}
                    onClick={() => {
                      if (layerBehavior) onSelect({ kind: layerBehavior, target_layer_index: item.index });
                    }}
                    className="inline-flex max-w-full items-center gap-1.5 rounded-md bg-background px-2.5 py-1.5 text-sm font-medium text-ink ring-1 ring-border hover:bg-plate disabled:opacity-50"
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
              {layers.length > 0 && layerBehavior === null && (
                <div className="mt-3 text-xs text-faint">
                  {t("keymap.edit.select_behavior_first")}
                </div>
              )}
            </div>
          </div>
        ) : tab === "tap_hold" ? (
          <div className="mt-3 min-h-0 flex flex-1 flex-col overflow-hidden">
            <div className="mb-3">
              <div className="mb-1.5 text-xs font-medium uppercase text-faint">
                {t("keymap.edit.tap_hold_behavior")}
              </div>
              <div className="flex gap-1.5">
                {TAP_HOLD_BEHAVIOR_CHOICES.map(({ kind, labelKey, tooltipKey }) => (
                  <button
                    key={kind}
                    type="button"
                    disabled={busy}
                    onClick={() => setTapHoldBehavior(kind)}
                    title={t(tooltipKey)}
                    className={`rounded-md px-3 py-1.5 text-sm font-medium ring-1 disabled:opacity-50 ${
                      tapHoldBehavior === kind
                        ? "bg-plate text-accent-deep ring-transparent shadow-neu-sel-in"
                        : "bg-background text-ink ring-border hover:bg-plate"
                    }`}
                  >
                    {t(labelKey)}
                  </button>
                ))}
              </div>
            </div>
            {tapHoldBehavior === null ? (
              <div className="mb-3 text-xs text-faint">
                {t("keymap.edit.select_behavior_first")}
              </div>
            ) : tapHoldBehavior === "mod_tap" ? (
              <div className="mb-3">
                <ModifierToggleRow
                  label={t("keymap.edit.hold_modifier")}
                  selectedIds={selectedModifierIds}
                  onToggle={(id) => toggleIn(id, setSelectedModifierIds)}
                  busy={busy}
                />
              </div>
            ) : (
              <div className="mb-3">
                <div className="mb-1.5 text-xs font-medium uppercase text-faint">
                  {t("keymap.edit.hold_layer")}
                </div>
                <div className="flex flex-wrap gap-1.5">
                  {layers.map((item) => {
                    const active = selectedTapLayerIndex === item.index;
                    return (
                      <button
                        key={item.id}
                        type="button"
                        disabled={busy}
                        onClick={() => setSelectedTapLayerIndex(item.index)}
                        className={`inline-flex max-w-full items-center gap-1.5 rounded-md px-2.5 py-1.5 text-sm font-medium ring-1 disabled:opacity-50 ${
                          active
                            ? "bg-plate text-accent-deep ring-transparent shadow-neu-sel-in"
                            : "bg-background text-ink ring-border hover:bg-plate"
                        }`}
                        title={`${item.name} (#${item.index})`}
                      >
                        <span className="font-mono text-[11px] text-faint">#{item.index}</span>
                        <span className="truncate">{item.name}</span>
                      </button>
                    );
                  })}
                </div>
                {layers.length === 0 && (
                  <div className="py-4 text-center text-sm text-faint">
                    {t("keymap.edit.no_layers")}
                  </div>
                )}
              </div>
            )}
            <div className="mb-1.5 text-xs font-medium uppercase text-faint">
              {t("keymap.edit.tap_key")}
            </div>
            <div className="mb-3">
              <ModifierToggleRow
                label={t("keymap.edit.key_modifiers")}
                selectedIds={selectedTapKeyModifierIds}
                onToggle={(id) => toggleIn(id, setSelectedTapKeyModifierIds)}
                busy={busy || tapEntriesDisabled}
              />
            </div>
            {renderCatalogButtons((entry) => {
              const tapUsage = applyModifiers(entry.hid_usage, selectedTapKeyModifierIds);
              if (tapHoldBehavior === "mod_tap" && selectedHoldUsage !== null) {
                onSelect({
                  kind: "mod_tap",
                  hold_hid_usage: selectedHoldUsage,
                  tap_hid_usage: tapUsage,
                });
              } else if (tapHoldBehavior === "layer_tap" && selectedTapLayerIndex !== null) {
                onSelect({
                  kind: "layer_tap",
                  target_layer_index: selectedTapLayerIndex,
                  tap_hid_usage: tapUsage,
                });
              }
            }, tapEntriesDisabled)}
          </div>
        ) : tab === "bt_out" ? (
          <div className="mt-3 min-h-0 flex flex-1 flex-col overflow-y-auto pr-1">
            <div className="mb-3">
              <div className="mb-1.5 text-xs font-medium uppercase text-faint">
                {t("keymap.edit.bluetooth_command")}
              </div>
              <div className="flex flex-wrap gap-1.5">
                {/* MVP encoder override only allows the select-profile command. */}
                {(isEncoder ? BLUETOOTH_COMMANDS.filter((command) => command.command === 3) : BLUETOOTH_COMMANDS).map((command) => (
                  <button
                    key={command.title}
                    type="button"
                    disabled={busy || command.value === null}
                    onClick={() => {
                      if (command.value !== null) {
                        onSelect({ kind: "bluetooth", command: command.command, value: command.value });
                      }
                    }}
                    title={command.title}
                    className="rounded-md bg-background px-2.5 py-1.5 text-sm font-medium text-ink ring-1 ring-border hover:bg-plate disabled:opacity-50"
                  >
                    {command.label}
                  </button>
                ))}
              </div>
            </div>
            <div>
              <div className="mb-1.5 text-xs font-medium uppercase text-faint">
                {t("keymap.edit.output_command")}
              </div>
              <div className="flex flex-wrap gap-1.5">
                {OUTPUT_COMMANDS.map((command) => (
                  <button
                    key={command.value}
                    type="button"
                    disabled={busy}
                    onClick={() => onSelect({ kind: "output_selection", value: command.value })}
                    title={command.title}
                    className="rounded-md bg-background px-3 py-1.5 text-sm font-medium text-ink ring-1 ring-border hover:bg-plate disabled:opacity-50"
                  >
                    {command.label}
                  </button>
                ))}
              </div>
            </div>
          </div>
        ) : (
          <div className="mt-3 min-h-0 flex-1 space-y-4 overflow-y-auto pr-1">
            <div>
              <div className="mb-1.5 text-xs font-medium uppercase text-faint">
                {t("keymap.edit.mouse")}
              </div>
              {renderCommandButtons([
                ...MOUSE_BUTTON_COMMANDS.map((command) => ({
                  ...command,
                  behavior: { kind: "mouse_key_press", value: command.value } as EditBehavior,
                })),
                ...MOUSE_MOVE_COMMANDS.map((command) => ({
                  ...command,
                  behavior: { kind: "mouse_move", value: command.value } as EditBehavior,
                })),
                ...MOUSE_SCROLL_COMMANDS.map((command) => ({
                  ...command,
                  behavior: { kind: "mouse_scroll", value: command.value } as EditBehavior,
                })),
              ])}
            </div>
            {!isEncoder && (<>
            <div>
              <div className="mb-1.5 text-xs font-medium uppercase text-faint">
                {t("keymap.edit.utility")}
              </div>
              {renderCommandButtons(UTILITY_COMMANDS)}
            </div>
            <div>
              <div className="mb-1.5 text-xs font-medium uppercase text-faint">
                {t("keymap.edit.system")}
              </div>
              {renderCommandButtons(SYSTEM_COMMANDS, true)}
            </div>
            <div>
              <div className="mb-1.5 text-xs font-medium uppercase text-faint">
                {t("keymap.edit.sticky_layer")}
              </div>
              <div className="flex flex-wrap gap-1.5">
                {layers.map((item) => {
                  const active = selectedStickyLayerIndex === item.index;
                  return (
                    <button
                      key={item.id}
                      type="button"
                      disabled={busy}
                      onClick={() => {
                        setSelectedStickyLayerIndex(item.index);
                        onSelect({ kind: "sticky_layer", target_layer_index: item.index });
                      }}
                      className={`inline-flex max-w-full items-center gap-1.5 rounded-md px-2.5 py-1.5 text-sm font-medium ring-1 disabled:opacity-50 ${
                        active
                          ? "bg-plate text-accent-deep ring-transparent shadow-neu-sel-in"
                          : "bg-background text-ink ring-border hover:bg-plate"
                      }`}
                      title={`${item.name} (#${item.index})`}
                    >
                      <span className="font-mono text-[11px] text-faint">#{item.index}</span>
                      <span className="truncate">{item.name}</span>
                    </button>
                  );
                })}
              </div>
              {layers.length === 0 && (
                <div className="py-4 text-center text-sm text-faint">
                  {t("keymap.edit.no_layers")}
                </div>
              )}
            </div>
            <div>
              <div className="mb-1.5 text-xs font-medium uppercase text-faint">
                {t("keymap.edit.sticky_key")}
              </div>
              <div className="mb-3">
                <ModifierToggleRow
                  label={t("keymap.edit.key_modifiers")}
                  selectedIds={selectedStickyKeyModifierIds}
                  onToggle={(id) => toggleIn(id, setSelectedStickyKeyModifierIds)}
                  busy={busy}
                />
              </div>
              {renderCatalogButtons((entry) =>
                onSelect({ kind: "sticky_key", hid_usage: applyModifiers(entry.hid_usage, selectedStickyKeyModifierIds) })
              )}
            </div>
            </>)}
          </div>
        )}
      </div>
    </>
  );
}

/** Popover for editing one encoder's CW / CCW bindings.
 *
 *  `source=override`: picking a side writes it immediately; the backend keeps
 *  the other side's current runtime value.
 *  `source=keymap` (initial edit): selections are held as a local draft and
 *  nothing is sent until both CW and CCW are explicitly chosen. Closing the
 *  panel cancels without sending, leaving the encoder on its `.keymap`
 *  bindings (keymap-encoder-editing-plan.md).
 */
function EncoderPanel({ binding, rect, catalog, layers, busy, onClose, onWrite }: {
  binding: EncoderBindingsDto;
  rect: PickerRect;
  catalog: KeyCatalogEntry[];
  layers: StudioLayer[];
  busy: boolean;
  onClose: () => void;
  onWrite: (cw: EditBehavior | null, ccw: EditBehavior | null) => void;
}) {
  const { t } = useLang();
  const [pickDirection, setPickDirection] = useState<"cw" | "ccw" | null>(null);
  const [draft, setDraft] = useState<{
    cw: { behavior: EditBehavior; label: string } | null;
    ccw: { behavior: EditBehavior; label: string } | null;
  }>({ cw: null, ccw: null });
  const initial = binding.source === "keymap";

  useEffect(() => {
    // Once the initial write is confirmed the device override becomes the
    // source of truth; drop the local draft.
    if (!initial) setDraft({ cw: null, ccw: null });
  }, [initial]);

  useEffect(() => {
    // While the BindingPicker is open its own Escape handler wins.
    if (pickDirection !== null) return undefined;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, pickDirection]);

  const rowLabel = (direction: "cw" | "ccw"): { text: string; set: boolean } => {
    const draftSide = draft[direction];
    if (draftSide) return { text: draftSide.label, set: true };
    if (initial) return { text: t("keymap.encoder.unset"), set: false };
    const side = direction === "cw" ? binding.cw : binding.ccw;
    return { text: side.label?.full_label ?? "--", set: true };
  };

  const handleSelect = (behavior: EditBehavior) => {
    const direction = pickDirection;
    if (!direction) return;
    setPickDirection(null);
    if (!initial) {
      onWrite(direction === "cw" ? behavior : null, direction === "ccw" ? behavior : null);
      return;
    }
    const label = optimisticLabelsForBehavior(behavior, catalog).full_label;
    const next = { ...draft, [direction]: { behavior, label } };
    setDraft(next);
    // Initial override: only send once both directions are explicitly chosen;
    // the host must never fill in a side the user did not pick.
    if (next.cw && next.ccw) onWrite(next.cw.behavior, next.ccw.behavior);
  };

  const position = popoverPosition(rect, Math.min(320, window.innerWidth - 24), 240);

  return (
    <>
      <button className="fixed inset-0 z-40 cursor-default bg-transparent" onClick={onClose} />
      <div
        className="fixed z-50 w-[min(320px,calc(100vw-24px))] rounded-card bg-surface p-3 shadow-neu-up ring-1 ring-border"
        style={{ left: position.left, top: position.top }}
      >
        <div className="text-xs font-medium uppercase text-faint">
          {t("keymap.encoder.title", { n: binding.encoder_id + 1 })}
        </div>
        {initial && (
          <div className="mt-2 text-xs text-faint">{t("keymap.encoder.initial_notice")}</div>
        )}
        {(binding.stale_saved_exists || binding.invalid_saved_exists) && (
          <div className="mt-2 flex items-start gap-1.5 text-xs text-amber-800">
            <AlertCircle size={12} className="mt-0.5 flex-shrink-0" />
            <span>{t(binding.stale_saved_exists ? "keymap.encoder.stale_saved" : "keymap.encoder.invalid_saved")}</span>
          </div>
        )}
        <div className="mt-3 space-y-2">
          {(["cw", "ccw"] as const).map((direction) => {
            const label = rowLabel(direction);
            return (
              <button
                key={direction}
                type="button"
                disabled={busy}
                onClick={() => setPickDirection(direction)}
                title={label.text}
                className="flex w-full items-center gap-2 rounded-lg bg-background px-3 py-2 text-left text-sm ring-1 ring-border hover:bg-plate disabled:opacity-50"
              >
                <span className="w-9 shrink-0 font-mono text-[11px] text-faint">
                  {t(direction === "cw" ? "keymap.encoder.cw" : "keymap.encoder.ccw")}
                </span>
                <span className={`min-w-0 flex-1 truncate font-medium ${label.set ? "text-ink" : "text-faint"}`}>
                  {label.text}
                </span>
              </button>
            );
          })}
        </div>
      </div>
      {pickDirection && (
        <BindingPicker
          variant="encoder"
          catalog={catalog}
          layers={layers}
          rect={rect}
          busy={busy}
          onClose={() => setPickDirection(null)}
          onSelect={handleSelect}
        />
      )}
    </>
  );
}

function popoverPosition(rect: PickerRect, width: number, height: number) {
  let left = rect.left + rect.width / 2 - width / 2;
  let top = rect.top + rect.height + 8;
  if (left + width > window.innerWidth - 12) left = window.innerWidth - width - 12;
  if (left < 12) left = 12;
  if (top + height > window.innerHeight - 12) top = Math.max(12, rect.top - height - 8);
  return { left, top };
}

function pickerPosition(rect: PickerRect) {
  return popoverPosition(
    rect,
    Math.min(520, window.innerWidth - 24),
    Math.min(560, window.innerHeight - 32)
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
  const pressedKeysRef = useRef<Set<number>>(new Set());

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
        if (pressedKeysRef.current.has(ev.position)) return;
        pressedKeysRef.current.add(ev.position);
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
        pressedKeysRef.current.delete(ev.position);
      }
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
      pressedKeysRef.current.clear();
      setPressedKeys(new Set());
      setTestedKeys(new Set());
      setTyped([]);
    };
  }, [statsUid]);

  const resetTester = useCallback(() => {
    pressedKeysRef.current.clear();
    setTestedKeys(new Set());
    setPressedKeys(new Set());
    setTyped([]);
  }, []);

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
        <div className="flex min-w-0 items-center gap-2">
          <TypedMarquee typed={typed} />
          <button
            onClick={resetTester}
            className="shrink-0 rounded-pill px-3 py-1.5 text-sm text-muted hover:bg-plate hover:text-ink transition-colors"
          >
            {t("tester.reset")}
          </button>
        </div>
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
  const className = studioStatusBadgeClass(device);
  return <span className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${className}`}>{t(`keymap.viewer.${device.keymap_viewer_status}` as TranslationKey)}</span>;
}

function studioStatusBadgeClass(device: StudioDeviceStatus): string {
  const ok = device.keymap_viewer_status === "available";
  const locked = device.keymap_viewer_status === "locked";
  return ok
    ? "bg-accent-soft text-accent-deep"
    : locked
      ? "bg-amber-100 text-amber-700"
      : "bg-plate text-muted";
}

function StudioConnectionBadge({ device }: { device: StudioDeviceStatus }) {
  const title = device.connection_type === "ble_studio" ? "BLE" : "USB";
  const Icon = device.connection_type === "ble_studio" ? Bluetooth : Usb;
  return (
    <span className={`inline-flex items-center rounded-full px-2 py-0.5 ${studioStatusBadgeClass(device)}`} title={title}>
      <Icon size={12} className="shrink-0" aria-label={title} />
    </span>
  );
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

function WarnNotice({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-start gap-2.5 rounded-lg bg-amber-50 px-4 py-3 text-sm text-amber-800 ring-1 ring-amber-200">
      <AlertCircle size={15} className="mt-0.5 flex-shrink-0" />
      <span>{children}</span>
    </div>
  );
}

function InfoNotice({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-start gap-2.5 rounded-lg bg-accent-soft px-4 py-3 text-sm text-accent-deep ring-1 ring-accent/25">
      <AlertCircle size={15} className="mt-0.5 flex-shrink-0" />
      <span>{children}</span>
    </div>
  );
}

function restoreConfirmText(
  report: RestoreReport,
  t: (key: TranslationKey, vars?: Record<string, string | number>) => string,
): string {
  const exportedAt = new Date(report.exported_at_ms).toLocaleString();
  const lines = [
    t("keymap.restore.summary", {
      device: report.source_device_name,
      date: exportedAt,
      write: report.will_write,
      unchanged: report.unchanged_skipped,
      blocked: report.blocked,
    }),
  ];
  if (report.behavior_verification === "skipped") lines.push(t("keymap.restore.verify_skipped"));
  const encoderTotal = report.encoder_will_write + report.encoder_unchanged_skipped + report.encoder_blocked;
  if (encoderTotal > 0) {
    lines.push(
      t("keymap.restore.encoder_summary", {
        write: report.encoder_will_write,
        unchanged: report.encoder_unchanged_skipped,
        blocked: report.encoder_blocked,
      }),
    );
  }
  for (const code of new Set(report.warnings.map((issue) => issue.code))) {
    lines.push(restoreIssueLabel(code, t));
  }
  for (const issue of report.errors) {
    lines.push(restoreIssueLabel(issue.code, t));
  }
  return lines.join("\n\n");
}

function restoreReportNotice(
  report: RestoreReport,
  t: (key: TranslationKey, vars?: Record<string, string | number>) => string,
): string | null {
  const hostlinkMissing = report.warnings.some((issue) => issue.code === "encoder_hostlink_missing");
  if (
    report.will_write === 0 &&
    report.blocked === 0 &&
    report.errors.length === 0 &&
    report.encoder_blocked === 0 &&
    !hostlinkMissing
  ) {
    return null;
  }
  const parts: string[] = [];
  if (report.behavior_verification === "skipped" && report.will_write > 0) {
    parts.push(t("keymap.restore.verify_skipped"));
  }
  if (report.blocked > 0) parts.push(t("keymap.restore.partial", { count: report.blocked }));
  if (hostlinkMissing) {
    parts.push(restoreIssueLabel("encoder_hostlink_missing", t));
  } else if (report.encoder_blocked > 0) {
    parts.push(t("keymap.restore.encoder_partial", { count: report.encoder_blocked }));
  }
  for (const issue of report.errors) parts.push(restoreIssueLabel(issue.code, t));
  return parts.length > 0 ? parts.join(" ") : null;
}

function restoreIssueLabel(
  code: string,
  t: (key: TranslationKey, vars?: Record<string, string | number>) => string,
): string {
  switch (code) {
    case "layer_count":
      return t("keymap.restore.abort.layer_count");
    case "position_count":
      return t("keymap.restore.abort.position_count");
    case "position_set":
      return t("keymap.restore.abort.position_set");
    case "encoder_hostlink_missing":
      return t("keymap.restore.encoder_hostlink_missing");
    case "encoder_layer_mismatch":
      return t("keymap.restore.encoder_layer_mismatch");
    case "encoder_out_of_range":
      return t("keymap.restore.encoder_out_of_range");
    case "behavior_missing":
    case "behavior_unverified":
    case "behavior_conflict":
      return t("keymap.restore.encoder_behavior_mismatch");
    case "key_apply_failed":
      return t("keymap.restore.key_apply_failed");
    case "encoder_apply_failed":
      return t("keymap.restore.encoder_apply_failed");
    case "state_refresh_failed":
      return t("keymap.restore.state_refresh_failed");
    default:
      return t("error.generic");
  }
}

function errorLabel(code: string, t: (key: TranslationKey, vars?: Record<string, string | number>) => string) {
  const normalized = code.trim().toLowerCase();
  const knownCodes = [
    "device_not_found", "hostlink_worker_unavailable", "hostlink_result_unknown",
    "hostlink_invalid_response", "locked", "timeout", "rpc_failed", "studio_read_failed",
    "disconnected", "invalid_location", "invalid_behavior", "invalid_parameters",
    "missing_behavior_role", "save_failed", "save_not_supported", "save_no_space",
    "save_result_unknown", "no_edit_session", "edit_session_exists", "unsaved_changes_exist",
    "session_device_mismatch", "port_busy", "editing_unsupported_for_ble", "add_layer_failed",
    "add_layer_no_space", "remove_layer_failed", "invalid_layer", "rename_layer_failed",
    "keymap_invalid_file", "keymap_unsupported_version", "keymap_file_too_large",
    "keymap_invalid_path", "restore_structure_mismatch", "keymap_export_failed",
    "keymap_restore_preview_failed", "keymap_restore_apply_failed", "encoder_behavior_ineligible",
    "encoder_behavior_unsupported_by_firmware", "encoder_bindings_incomplete",
    "config_rpc_status_invalid_argument", "config_rpc_status_storage_error",
    "config_rpc_status_internal_error",
  ] as const;
  const matched = knownCodes.find((candidate) => normalized === candidate || normalized.includes(candidate));
  return matched ? t(`keymap.error.${matched}` as TranslationKey) : t("error.generic");
}
