import { useState, useEffect, useCallback, useMemo, type ReactNode } from "react";
import {
  RefreshCw,
  XCircle,
  Usb,
  AlertCircle,
  BatteryMedium,
  Keyboard,
  Bluetooth,
  ChevronRight,
  Play,
  Square,
  Info,
  AlertTriangle,
} from "lucide-react";
import { probeDevices, startMonitoring, stopMonitoring } from "../api";
import { RollingNumber } from "../components/RollingNumber";
import { ErrorNotice, SpinnerIcon } from "../components/Ui";
import { displayBatterySources } from "../lib/battery";
import { formatClockTime } from "../lib/format";
import { friendlyError } from "../lib/errors";
import {
  buildDeviceCards,
  groupProbeResults,
  groupVerifiedHostLinkDevices,
  hasKnownConnectionType,
  isKnownStudioConnectionType,
  knownHostLinkConnectionTypes,
  uniqueConnectionTypes,
  type DeviceCardModel,
} from "../lib/deviceCards";
import { useLang, type Lang, type TranslationKey } from "../i18n";
import type {
  DeviceBatteryStatus,
  DeviceInfo,
  LogEntry,
  MonitorStatus,
  ProbeResult,
  StudioDeviceStatus,
} from "../types";

interface DevicesProps {
  studioDevices: StudioDeviceStatus[];
  studioScanning: boolean;
  studioError: string | null;
  refreshStudioDevices: () => Promise<StudioDeviceStatus[]>;
  status: MonitorStatus;
  logs: LogEntry[];
}

export default function Devices({
  studioDevices,
  studioScanning,
  studioError,
  refreshStudioDevices,
  status,
  logs,
}: DevicesProps) {
  const { t, lang } = useLang();
  const [results, setResults] = useState<ProbeResult[] | null>(null);
  const [hostLinkLoading, setHostLinkLoading] = useState(false);
  const [actionLoading, setActionLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const scanHostLink = useCallback(async () => {
    setHostLinkLoading(true);
    setError(null);
    try {
      setResults(await probeDevices());
    } catch (e) {
      setError(String(e));
    } finally {
      setHostLinkLoading(false);
    }
  }, []);

  const scanStudio = useCallback(async () => {
    setError(null);
    try {
      await refreshStudioDevices();
    } catch (e) {
      setError(String(e));
    }
  }, [refreshStudioDevices]);

  const handleProbe = useCallback(() => {
    void scanHostLink();
    void scanStudio();
  }, [scanHostLink, scanStudio]);

  const handleMonitoringToggle = async () => {
    setActionLoading(true);
    setActionError(null);
    try {
      if (status.running) {
        await stopMonitoring();
      } else {
        await startMonitoring();
      }
    } catch (e) {
      setActionError(friendlyError(e, t));
    } finally {
      setActionLoading(false);
    }
  };

  const loading = hostLinkLoading || studioScanning;
  const hostLinkGroups = useMemo(
    () =>
      mergeHostLinkGroups(
        results === null ? [] : groupProbeResults(results),
        groupVerifiedHostLinkDevices(status.host_link_devices)
      ),
    [results, status.host_link_devices]
  );
  const cards = useMemo(
    () => buildDeviceCards(hostLinkGroups, studioDevices, status.device_battery),
    [hostLinkGroups, studioDevices, status.device_battery]
  );
  const supportedCards = useMemo(() => cards.filter((card) => card.supported), [cards]);
  const otherCards = useMemo(() => cards.filter((card) => !card.supported), [cards]);
  const warningCount = supportedCards.filter(cardHasWarning).length;
  const sortedLogs = useMemo(
    () =>
      [...logs].sort((a, b) => {
        if (a.timestamp_ms !== b.timestamp_ms) return b.timestamp_ms - a.timestamp_ms;
        return b.id - a.id;
      }),
    [logs]
  );

  useEffect(() => {
    handleProbe();
  }, [handleProbe]);

  return (
    <div className="mx-auto max-w-4xl space-y-5 p-6">
      <div className="flex items-center justify-between gap-4">
        <div>
          <h1 className="text-xl font-medium text-ink">{t("devices.title")}</h1>
          <p className="mt-0.5 text-sm text-muted">{t("devices.subtitle")}</p>
        </div>
        <button
          onClick={handleMonitoringToggle}
          disabled={actionLoading}
          className="btn-neu flex items-center gap-2 rounded-full px-5 py-2.5 text-sm font-medium text-ink disabled:opacity-60"
        >
          {actionLoading ? (
            <SpinnerIcon />
          ) : status.running ? (
            <Square size={15} className="text-accent" />
          ) : (
            <Play size={15} />
          )}
          {status.running ? t("devices.monitoring.stop") : t("devices.monitoring.start")}
        </button>
      </div>

      {actionError && <ErrorNotice message={t("devices.action_error")} details={actionError} />}
      {(error || studioError) && (
        <ErrorNotice message={t("devices.scan_error")} details={friendlyError(error ?? studioError, t)} />
      )}
      {status.last_error && (
        <ErrorNotice message={t("devices.status_error")} details={friendlyError(status.last_error, t)} />
      )}

      <div className="flex flex-wrap items-center justify-between gap-3 rounded-card bg-surface px-5 py-4">
        <div className="flex flex-wrap items-center gap-3">
          <Metric label={t("devices.connected_count")} value={t("devices.connected_count.value", { n: supportedCards.length })} />
          {warningCount > 0 && (
            <span className="inline-flex items-center gap-1.5 rounded-full bg-amber-100 px-2.5 py-1 text-xs font-medium text-amber-700">
              <AlertTriangle size={13} />
              {t("devices.warning_count", { n: warningCount })}
            </span>
          )}
        </div>
        <button
          onClick={handleProbe}
          disabled={loading}
          className="row-lift flex items-center gap-2 rounded-full border border-border bg-surface px-4 py-2.5 text-sm font-medium text-muted hover:text-ink disabled:opacity-60"
        >
          <RefreshCw size={15} className={loading ? "animate-spin" : ""} />
          {loading ? t("devices.scanning") : t("devices.scan")}
        </button>
      </div>

      {loading && cards.length === 0 ? (
        <LoadingCard text={t("devices.scanning.hint")} />
      ) : supportedCards.length === 0 ? (
        <EmptyCard title={t("devices.empty")} body={t("devices.empty.hint")} />
      ) : (
        <div className="space-y-3">
          {supportedCards.map((card) => (
            <DeviceCard key={card.key} card={card} />
          ))}
        </div>
      )}

      {otherCards.length > 0 && <OtherDevices cards={otherCards} />}

      <ActivityLog logs={sortedLogs} lang={lang} />
    </div>
  );
}

function cardHasWarning(card: DeviceCardModel): boolean {
  return (
    (card.hostLink !== null && (!card.hostLink.verified || card.hostLink.errors.length > 0)) ||
    (card.studio !== null && (card.studio.rpc_status !== "ok" || card.studio.lock_state === "locked" || card.studio.error_code !== "none"))
  );
}

function mergeHostLinkGroups(
  scannedGroups: ReturnType<typeof groupProbeResults>,
  monitoredGroups: ReturnType<typeof groupVerifiedHostLinkDevices>
): ReturnType<typeof groupProbeResults> {
  const merged = new Map<string, ReturnType<typeof groupProbeResults>[number]>();
  for (const group of scannedGroups) {
    merged.set(group.key, { ...group, devices: [...group.devices], errors: [...group.errors] });
  }
  for (const group of monitoredGroups) {
    const existing = merged.get(group.key);
    if (!existing) {
      merged.set(group.key, { ...group, devices: [...group.devices], errors: [...group.errors] });
      continue;
    }
    const devices = new Map(existing.devices.map((device) => [device.path, device]));
    for (const device of group.devices) devices.set(device.path, device);
    merged.set(group.key, {
      key: existing.key,
      name: existing.name === "Unknown Device" ? group.name : existing.name,
      devices: [...devices.values()],
      verified: existing.verified || group.verified,
      errors: existing.errors,
    });
  }
  return [...merged.values()];
}

function DeviceCard({ card }: { card: DeviceCardModel }) {
  const { t } = useLang();
  const [expanded, setExpanded] = useState(false);
  const connectionTypes = card.hostLink ? uniqueConnectionTypes(card.hostLink.devices) : [];
  const knownConnectionTypes = card.hostLink ? knownHostLinkConnectionTypes(card.hostLink.devices) : [];
  const connectionLabel =
    knownConnectionTypes.length > 0
      ? knownConnectionTypes.map(hostLinkConnectionShortLabel).join("/")
      : connectionTypes.length > 0 && !isKnownStudioConnectionType(card.studio)
        ? connectionTypes.map(hostLinkConnectionShortLabel).join("/")
      : card.studio
        ? studioConnectionLabel(card.studio, t)
        : "--";
  const warnings = deviceWarnings(card, t);

  return (
    <div className="overflow-hidden rounded-card bg-surface">
      <div className="px-5 py-4">
        <div className="flex items-start gap-4">
          <IconBox active={hasKnownConnectionType(card)} icon={<CardIcon card={card} />} title={connectionLabel} />
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="min-w-0">
                <h2 className="truncate text-base font-medium text-ink">{card.name}</h2>
                <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted">
                  <span>{connectionLabel}</span>
                  {card.battery && <BatterySummary battery={card.battery} />}
                </div>
              </div>
            </div>

            <div className="mt-3 flex flex-wrap gap-2">
              <StatusPill ok={card.hostLink?.verified ?? false} label={hostLinkStatusLabel(card, t)} />
              <StudioPill label={studioStatusLabel(card.studio, t)} tone={studioTone(card.studio)} />
            </div>

            <CapabilityChips card={card} />

            {warnings.length > 0 && (
              <div className="mt-3 space-y-1.5">
                {warnings.map((warning) => (
                  <div key={warning} className="rounded-md bg-amber-50 px-3 py-1.5 text-xs text-amber-700">
                    {warning}
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>
      <div className="border-t border-background">
        <button
          className="flex w-full items-center gap-2 px-5 py-3 text-xs font-medium text-muted hover:bg-background hover:text-ink"
          onClick={() => setExpanded((open) => !open)}
        >
          <ChevronRight
            size={13}
            className={`transition-transform ${expanded ? "rotate-90" : ""}`}
          />
          {t("devices.details")}
        </button>
        {expanded && <DeviceDetails card={card} />}
      </div>
    </div>
  );
}

function OtherDevices({ cards }: { cards: DeviceCardModel[] }) {
  const { t } = useLang();
  const [open, setOpen] = useState(false);
  return (
    <div className="overflow-hidden rounded-card bg-surface">
      <button
        className="flex w-full items-center justify-between gap-3 px-5 py-3 text-sm font-medium text-muted hover:bg-background hover:text-ink"
        onClick={() => setOpen((value) => !value)}
      >
        <span className="flex items-center gap-2">
          <ChevronRight
            size={14}
            className={`transition-transform ${open ? "rotate-90" : ""}`}
          />
          {t("devices.other")}
        </span>
        <span className="font-mono text-xs text-faint">
          {t("devices.other.count", { n: cards.length })}
        </span>
      </button>
      {open && (
        <div className="space-y-3 border-t border-background p-3">
          {cards.map((card) => (
            <DeviceCard key={card.key} card={card} />
          ))}
        </div>
      )}
    </div>
  );
}

function CardIcon({ card }: { card: DeviceCardModel }) {
  if (card.hostLink && knownHostLinkConnectionTypes(card.hostLink.devices).length > 0) {
    return <HostLinkTransportIcons devices={card.hostLink.devices} />;
  }
  if (card.studio?.connection_type === "ble_studio") return <Bluetooth size={18} />;
  if (card.studio?.connection_type === "usb_serial") return <Usb size={18} />;
  if (card.hostLink) return <HostLinkTransportIcons devices={card.hostLink.devices} />;
  return <Keyboard size={18} />;
}

function CapabilityChips({ card }: { card: DeviceCardModel }) {
  const { t } = useLang();
  const chips = capabilityChips(card, t);
  if (chips.length === 0) return null;
  return (
    <div className="mt-3 flex flex-wrap gap-2">
      {chips.map((chip) => (
        <span key={chip} className="rounded-full bg-plate px-2.5 py-1 text-[11px] font-medium text-ink">
          {chip}
        </span>
      ))}
    </div>
  );
}

function capabilityChips(card: DeviceCardModel, t: (key: TranslationKey) => string): string[] {
  const chips: string[] = [];
  const capabilities = card.hostLink?.devices.reduce((bits, device) => bits | device.capabilities, 0) ?? 0;
  const add = (label: string) => {
    if (!chips.includes(label)) chips.push(label);
  };

  if ((capabilities & 1) !== 0) add(t("devices.capability.app_layer"));
  if (card.studio?.keymap_viewer_status === "available" || card.studio?.keymap_viewer_status === "locked") {
    add(t("devices.capability.keymap"));
  }
  if ((capabilities & 2) !== 0) add(t("devices.capability.time_sync"));
  if ((capabilities & 4) !== 0) add(t("devices.capability.ai_usage"));
  if ((capabilities & 16) !== 0 || card.battery !== null) add(t("devices.capability.battery"));
  if ((capabilities & 32) !== 0) add(t("devices.capability.host_action"));
  if ((capabilities & 64) !== 0) add(t("devices.capability.key_stats"));
  if ((capabilities & 128) !== 0) add(t("devices.capability.layer_state"));

  return chips;
}

function DeviceDetails({ card }: { card: DeviceCardModel }) {
  const { t } = useLang();
  return (
    <div className="border-t border-background px-5 py-4">
      <div className="grid gap-4 md:grid-cols-2">
        <DetailGroup title="Host Link">
          {card.hostLink ? (
            <>
              {card.hostLink.devices.map((device) => (
                <div key={device.path} className="space-y-1.5 rounded-lg bg-plate px-3 py-2">
                  <DetailRow label="VID" value={hex(device.vendor_id, 4)} />
                  <DetailRow label="PID" value={hex(device.product_id, 4)} />
                  <DetailRow label="Usage Page" value={hex(device.usage_page, 4)} />
                  <DetailRow label="Usage" value={hex(device.usage, 2)} />
                  <DetailRow label="Serial" value={device.serial_number ?? "--"} />
                  <DetailRow label="UID" value={device.device_uid_hash ?? "--"} />
                  <DetailRow label="Capabilities" value={rawCapabilityLabels(device.capabilities).join(", ") || "--"} />
                  <DetailRow label="Path" value={device.path} />
                </div>
              ))}
              {card.hostLink.errors.map((error, index) => (
                <div key={`${error}-${index}`} className="rounded-md bg-red-50 px-3 py-1.5 text-xs text-red-600">
                  {error}
                </div>
              ))}
            </>
          ) : (
            <p className="text-xs text-faint">{t("devices.host_link.none")}</p>
          )}
        </DetailGroup>

        <DetailGroup title="Studio">
          {card.studio ? (
            <div className="space-y-1.5 rounded-lg bg-plate px-3 py-2">
              <DetailRow label="Port" value={card.studio.port_name} />
              <DetailRow label="VID" value={card.studio.vid === null ? "--" : hex(card.studio.vid, 4)} />
              <DetailRow label="PID" value={card.studio.pid === null ? "--" : hex(card.studio.pid, 4)} />
              <DetailRow label="Serial" value={card.studio.serial_number ?? "--"} />
              <DetailRow label="Manufacturer" value={card.studio.manufacturer ?? "--"} />
              <DetailRow label="Product" value={card.studio.product ?? "--"} />
              <DetailRow label="RPC" value={card.studio.rpc_status} />
              <DetailRow label="Lock" value={card.studio.lock_state} />
              <DetailRow label="Viewer" value={card.studio.keymap_viewer_status} />
              <DetailRow label="Error" value={card.studio.error_code} />
            </div>
          ) : (
            <p className="text-xs text-faint">{t("devices.studio.none")}</p>
          )}
        </DetailGroup>
      </div>
    </div>
  );
}

function ActivityLog({ logs, lang }: { logs: LogEntry[]; lang: Lang }) {
  const { t } = useLang();
  return (
    <div className="overflow-hidden rounded-card bg-surface">
      <div className="flex items-center justify-between border-b border-background px-5 py-3.5">
        <h2 className="text-sm font-medium text-ink">{t("devices.log.title")}</h2>
        <span className="font-mono text-xs text-faint">{t("devices.log.count", { n: logs.length })}</span>
      </div>
      <div className="max-h-60 overflow-y-auto">
        {logs.length === 0 ? (
          <div className="px-5 py-8 text-center text-sm text-faint">{t("devices.log.empty")}</div>
        ) : (
          <ul className="divide-y divide-background">
            {logs.slice(0, 50).map((entry) => (
              <li key={entry.id} className="flex items-start gap-3 px-5 py-2.5">
                <LogIcon level={entry.level} />
                <div className="min-w-0 flex-1">
                  <span className="font-mono text-sm text-ink">{entry.message}</span>
                </div>
                <span className="flex-shrink-0 font-mono text-[11px] text-faint">
                  {formatClockTime(entry.timestamp_ms, lang)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-xs text-muted">{label}</div>
      <div className="mt-0.5 font-mono text-sm font-medium text-ink">{value}</div>
    </div>
  );
}

function BatterySummary({ battery }: { battery: DeviceBatteryStatus }) {
  const sources = displayBatterySources(battery.sources);
  return (
    <span className="inline-flex items-center gap-1.5">
      <BatteryMedium size={13} className="text-faint" />
      {sources.length === 0 ? (
        <span className="font-mono text-faint">--</span>
      ) : (
        sources.map((source) => (
          <span key={source.source} className={`font-mono ${batteryTextClass(source.level)}`}>
            {source.label}:
            <RollingNumber value={source.level} format={(v) => `${Math.round(v)}%`} />
          </span>
        ))
      )}
    </span>
  );
}

function LoadingCard({ text }: { text: string }) {
  return (
    <div className="rounded-card bg-surface px-6 py-10 text-center">
      <div className="mx-auto mb-3 h-8 w-8 animate-spin rounded-full border-2 border-border border-t-accent" />
      <p className="text-sm text-faint">{text}</p>
    </div>
  );
}

function EmptyCard({ title, body }: { title: string; body: string }) {
  return (
    <div className="rounded-card bg-surface px-6 py-10 text-center">
      <XCircle size={36} className="mx-auto mb-3 text-disabled" />
      <p className="text-sm font-medium text-muted">{title}</p>
      <p className="mt-1 text-xs text-faint">{body}</p>
    </div>
  );
}

function DetailGroup({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="space-y-2">
      <h3 className="text-xs font-medium text-muted">{title}</h3>
      {children}
    </section>
  );
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid grid-cols-[88px_minmax(0,1fr)] gap-2 text-[11px]">
      <span className="text-muted">{label}</span>
      <span className="truncate font-mono text-ink" title={value}>
        {value}
      </span>
    </div>
  );
}

function batteryTextClass(level: number): string {
  if (level <= 15) return "text-red-600";
  if (level <= 30) return "text-amber-600";
  return "text-ink";
}

function deviceWarnings(card: DeviceCardModel, t: (key: TranslationKey) => string): string[] {
  const warnings: string[] = [];
  if (card.hostLink && !card.hostLink.verified) warnings.push(t("devices.warning.host_link"));
  if (card.studio?.lock_state === "locked") warnings.push(t("devices.warning.studio_locked"));
  if (card.studio && card.studio.rpc_status !== "ok") warnings.push(t("devices.warning.studio_rpc"));
  if (card.studio && card.studio.error_code !== "none") {
    warnings.push(t(`devices.studio.error.${card.studio.error_code}` as TranslationKey));
  }
  return [...new Set([...warnings, ...(card.hostLink?.errors ?? [])])];
}

function hostLinkStatusLabel(card: DeviceCardModel, t: (key: TranslationKey) => string): string {
  if (!card.hostLink) return t("devices.host_link.disconnected");
  return card.hostLink.verified ? t("devices.host_link.connected") : t("devices.host_link.failed");
}

function studioStatusLabel(device: StudioDeviceStatus | null, t: (key: TranslationKey) => string): string {
  if (!device) return t("devices.studio.unsupported");
  if (device.rpc_status !== "ok") return t("devices.studio.not_detected");
  if (device.keymap_viewer_status === "available") return t("devices.studio.editable");
  if (device.keymap_viewer_status === "locked" || device.lock_state === "locked") return t("devices.studio.locked");
  if (device.keymap_viewer_status === "unsupported") return t("devices.studio.unsupported");
  return t("devices.studio.supported");
}

function studioTone(device: StudioDeviceStatus | null): "ok" | "warn" | "muted" {
  if (!device) return "muted";
  if (device.rpc_status !== "ok" || device.lock_state === "locked" || device.keymap_viewer_status === "locked") return "warn";
  if (device.keymap_viewer_status === "available") return "ok";
  return "muted";
}

function studioConnectionLabel(device: StudioDeviceStatus, t: (key: TranslationKey) => string): string {
  if (device.connection_type === "ble_studio") return t("keymap.connection_ble_studio");
  return t("keymap.connection_usb_serial");
}

function hostLinkConnectionShortLabel(connectionType: DeviceInfo["connection_type"]) {
  if (connectionType === "bluetooth") return "BLE";
  if (connectionType === "usb") return "USB";
  return "Unknown";
}

function HostLinkTransportIcons({ devices }: { devices: DeviceInfo[] }) {
  const types = uniqueConnectionTypes(devices);
  return (
    <div className="flex items-center gap-1">
      {types.map((type) => (
        <span key={type} className="flex items-center">
          {hostLinkConnectionIcon(type, types.length > 1 ? 15 : 18)}
        </span>
      ))}
    </div>
  );
}

function hostLinkConnectionIcon(connectionType: DeviceInfo["connection_type"], size = 18) {
  if (connectionType === "bluetooth") return <Bluetooth size={size} />;
  if (connectionType === "usb") return <Usb size={size} />;
  return <Keyboard size={size} />;
}

function IconBox({ active, icon, title }: { active: boolean; icon: ReactNode; title?: string }) {
  return (
    <div
      title={title}
      aria-label={title}
      className={`mt-0.5 flex h-9 min-w-9 flex-shrink-0 items-center justify-center rounded-lg px-2 ${
        active ? "bg-accent-soft text-accent-deep" : "bg-plate text-disabled"
      }`}
    >
      {icon}
    </div>
  );
}

function StatusPill({ ok, label }: { ok: boolean; label: string }) {
  return (
    <span
      className={`flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium ${
        ok ? "bg-accent-soft text-accent-deep" : "bg-plate text-muted"
      }`}
    >
      {label}
    </span>
  );
}

function StudioPill({ label, tone }: { label: string; tone: "ok" | "warn" | "muted" }) {
  const color =
    tone === "ok"
      ? "bg-accent-soft text-accent-deep"
      : tone === "warn"
        ? "bg-amber-100 text-amber-700"
        : "bg-plate text-muted";
  return <span className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${color}`}>{label}</span>;
}

function LogIcon({ level }: { level: string }) {
  if (level === "error") return <AlertCircle size={14} className="mt-0.5 flex-shrink-0 text-red-400" />;
  if (level === "warn") return <AlertTriangle size={14} className="mt-0.5 flex-shrink-0 text-amber-400" />;
  return <Info size={14} className="mt-0.5 flex-shrink-0 text-faint" />;
}

function rawCapabilityLabels(bits: number) {
  const labels: string[] = [];
  if ((bits & 1) !== 0) labels.push("APP_LAYER");
  if ((bits & 2) !== 0) labels.push("TIME_SYNC");
  if ((bits & 4) !== 0) labels.push("AI_USAGE");
  if ((bits & 8) !== 0) labels.push("THEME");
  if ((bits & 16) !== 0) labels.push("BATTERY");
  if ((bits & 32) !== 0) labels.push("HOST_ACTION");
  if ((bits & 64) !== 0) labels.push("KEY_STATS");
  if ((bits & 128) !== 0) labels.push("LAYER_STATE");
  return labels;
}

function hex(n: number, digits: number) {
  return `0x${n.toString(16).toUpperCase().padStart(digits, "0")}`;
}
