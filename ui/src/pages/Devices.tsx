import { useState, useEffect, useCallback } from "react";
import { RefreshCw, CheckCircle2, XCircle, Usb, AlertCircle, BatteryMedium, Keyboard, Bluetooth } from "lucide-react";
import { probeDevices } from "../api";
import { RollingNumber } from "../components/RollingNumber";
import { friendlyError } from "../lib/errors";
import { useLang, type TranslationKey } from "../i18n";
import type { DeviceBatteryStatus, MonitorStatus, ProbeResult, StudioDeviceStatus } from "../types";

interface DevicesProps {
  studioDevices: StudioDeviceStatus[];
  studioScanning: boolean;
  studioError: string | null;
  refreshStudioDevices: () => Promise<StudioDeviceStatus[]>;
  status: MonitorStatus;
}

export default function Devices({ studioDevices, studioScanning, studioError, refreshStudioDevices, status }: DevicesProps) {
  const { t } = useLang();
  const [results, setResults] = useState<ProbeResult[] | null>(null);
  const [hostLinkLoading, setHostLinkLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  const loading = hostLinkLoading || studioScanning;
  const sortedResults = results === null ? null : [...results].sort(compareProbeResults);
  const sortedStudioDevices = [...studioDevices].sort(compareStudioDevices);
  const hostLinkOkCount = results?.filter((result) => result.verified).length ?? 0;
  const studioOkCount = studioDevices.filter((device) => device.rpc_status === "ok").length;

  useEffect(() => { handleProbe(); }, [handleProbe]);

  return (
    <div className="p-6 max-w-3xl mx-auto space-y-5">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-medium text-ink">{t("devices.title")}</h1>
          <p className="text-sm text-muted mt-0.5">{t("devices.subtitle")}</p>
        </div>
        <button
          onClick={handleProbe}
          disabled={loading}
          className="btn-neu flex items-center gap-2 rounded-full px-4 py-2.5 text-sm font-medium text-ink disabled:opacity-60"
        >
          <RefreshCw size={15} className={loading ? "animate-spin" : ""} />
          {loading ? t("devices.scanning") : t("devices.scan")}
        </button>
      </div>

      {(error || studioError) && (
        <div className="flex items-start gap-2.5 rounded-lg bg-red-50 px-4 py-3 text-sm text-red-700 ring-1 ring-red-200">
          <AlertCircle size={15} className="mt-0.5 flex-shrink-0" />
          <span>{friendlyError(error ?? studioError, t)}</span>
        </div>
      )}

      <DeviceSection title={t("devices.host_link.section")} count={hostLinkOkCount}>
        {results === null || hostLinkLoading ? (
          <LoadingCard text={t("devices.scanning.hint")} />
        ) : results.length === 0 ? (
          <EmptyCard title={t("devices.empty")} body={t("devices.empty.hint")} />
        ) : (
          <div className="space-y-3">
            {sortedResults!.map((result) => (
              <HostLinkDeviceCard
                key={result.device.path}
                result={result}
                battery={findBatteryForDevice(status.device_battery, result.device)}
              />
            ))}
          </div>
        )}
      </DeviceSection>

      <DeviceSection title={t("devices.studio.section")} count={studioOkCount}>
        {studioScanning ? (
          <LoadingCard text={t("devices.scanning.hint")} />
        ) : studioDevices.length === 0 ? (
          <EmptyCard title={t("devices.studio.empty")} body={t("devices.studio.empty.hint")} />
        ) : (
          <div className="space-y-3">{sortedStudioDevices.map((device) => <StudioDeviceCard key={device.id} device={device} />)}</div>
        )}
      </DeviceSection>

      {results !== null && (
        <p className="text-xs text-faint text-center font-mono">
          {t("devices.summary", {
            ok: results.filter((r) => r.verified).length,
            total: results.length,
          })}
        </p>
      )}
    </div>
  );
}

function compareProbeResults(a: ProbeResult, b: ProbeResult): number {
  return (
    Number(b.verified) - Number(a.verified) ||
    compareConnectionType(a.device.connection_type, b.device.connection_type) ||
    deviceDisplayName(a.device).localeCompare(deviceDisplayName(b.device)) ||
    a.device.path.localeCompare(b.device.path)
  );
}

function compareStudioDevices(a: StudioDeviceStatus, b: StudioDeviceStatus): number {
  return (
    Number(b.rpc_status === "ok") - Number(a.rpc_status === "ok") ||
    studioViewerRank(a) - studioViewerRank(b) ||
    a.display_name.localeCompare(b.display_name) ||
    a.port_name.localeCompare(b.port_name)
  );
}

function compareConnectionType(a: ProbeResult["device"]["connection_type"], b: ProbeResult["device"]["connection_type"]): number {
  return connectionTypeRank(a) - connectionTypeRank(b);
}

function connectionTypeRank(connectionType: ProbeResult["device"]["connection_type"]): number {
  if (connectionType === "usb") return 0;
  if (connectionType === "bluetooth") return 1;
  return 2;
}

function studioViewerRank(device: StudioDeviceStatus): number {
  if (device.keymap_viewer_status === "available") return 0;
  if (device.keymap_viewer_status === "locked") return 1;
  if (device.keymap_viewer_status === "unsupported") return 2;
  return 3;
}

function deviceDisplayName(device: ProbeResult["device"]): string {
  return device.product ?? device.manufacturer ?? device.serial_number ?? device.device_uid_hash ?? "Unknown Device";
}

function findBatteryForDevice(
  batteries: DeviceBatteryStatus[],
  device: ProbeResult["device"]
): DeviceBatteryStatus | null {
  return (
    batteries.find(
      (entry) =>
        (device.device_uid_hash !== null && entry.device_key === device.device_uid_hash) ||
        serialsMatch(entry.serial_number, device.serial_number) ||
        (entry.product !== null && entry.product === device.product)
    ) ?? null
  );
}

function serialsMatch(a: string | null, b: string | null): boolean {
  if (!a || !b) return false;
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

function DeviceSection({ title, count, children }: { title: string; count: number; children: React.ReactNode }) {
  return (
    <section className="space-y-3">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-medium text-ink">{title}</h2>
        <span className="text-xs text-faint font-mono">{count}</span>
      </div>
      {children}
    </section>
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
      <XCircle size={36} className="mx-auto text-disabled mb-3" />
      <p className="text-sm text-muted font-medium">{title}</p>
      <p className="text-xs text-faint mt-1">{body}</p>
    </div>
  );
}

function HostLinkDeviceCard({ result, battery }: { result: ProbeResult; battery: DeviceBatteryStatus | null }) {
  const { t } = useLang();
  const { device, verified, error } = result;
  const name = device.product ?? device.manufacturer ?? "Unknown Device";
  const connectionLabel = hostLinkConnectionLabel(device.connection_type);

  return (
    <div className="rounded-card bg-surface">
      <div className="flex items-start gap-4 px-5 py-4">
        <IconBox ok={verified} icon={hostLinkConnectionIcon(device.connection_type)} title={connectionLabel} />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="font-medium text-ink text-sm">{name}</span>
            <StatusPill ok={verified} label={verified ? t("devices.host_link.ok") : t("devices.host_link.not_supported")} />
          </div>
          {device.manufacturer && <div className="text-xs text-faint mt-0.5">{device.manufacturer}</div>}
          <DeviceBadges device={device} />
          {verified && <HostLinkCapabilityBadges device={device} />}
          {battery && battery.sources.length > 0 && (
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <BatteryMedium size={13} className="text-faint" />
              {battery.sources.map((source) => (
                <span
                  key={source.source}
                  className="inline-flex items-center gap-1 rounded-md bg-plate px-2 py-0.5 text-[11px]"
                >
                  <span className="text-muted">
                    {t(`battery.source.${source.source}` as TranslationKey)}:
                  </span>
                  <span className={`font-mono ${batteryTextClass(source.level)}`}>
                    <RollingNumber value={source.level} format={(v) => `${Math.round(v)}%`} />
                  </span>
                </span>
              ))}
            </div>
          )}
          <div className="mt-1.5 font-mono text-[10px] text-faint truncate" title={device.path}>{device.path}</div>
          {error && <div className="mt-2 rounded-md bg-red-50 px-3 py-1.5 text-xs text-red-600">{error}</div>}
        </div>
      </div>
    </div>
  );
}

function batteryTextClass(level: number | null): string {
  if (level === null) return "text-faint";
  if (level <= 15) return "text-red-600";
  if (level <= 30) return "text-amber-600";
  return "text-ink";
}

function HostLinkCapabilityBadges({ device }: { device: ProbeResult["device"] }) {
  const { t } = useLang();
  const capabilities = capabilityLabels(device.capabilities);
  const appLayerSupported = (device.capabilities & 1) !== 0;
  return (
    <div className="mt-2 flex flex-wrap gap-2">
      <Badge label="UID" value={device.device_uid_hash === null ? t("devices.host_link.uid_none" as TranslationKey) : device.device_uid_hash} />
      <Badge label={t("devices.host_link.capabilities" as TranslationKey)} value={capabilities.length === 0 ? t("devices.host_link.capabilities_none" as TranslationKey) : capabilities.join(", ")} />
      <span className={`inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-[11px] ${appLayerSupported ? "bg-accent-soft text-accent-deep" : "bg-plate text-muted"}`}>
        {t("devices.host_link.app_layer" as TranslationKey)}: {appLayerSupported ? t("devices.host_link.supported" as TranslationKey) : t("devices.host_link.not_supported" as TranslationKey)}
      </span>
    </div>
  );
}

function capabilityLabels(bits: number) {
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

function StudioDeviceCard({ device }: { device: StudioDeviceStatus }) {
  const { t } = useLang();
  const supported = device.rpc_status === "ok";
  const locked = device.lock_state === "locked";
  return (
    <div className="rounded-card bg-surface">
      <div className="flex items-start gap-4 px-5 py-4">
        <IconBox ok={supported} icon={<Keyboard size={18} />} />
        <div className="flex-1 min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium text-ink text-sm">{device.display_name}</span>
            <StudioPill label={studioSupportedLabel(device, t)} tone={supported ? (locked ? "warn" : "ok") : "muted"} />
            <StudioPill label={t(`keymap.viewer.${device.keymap_viewer_status}` as TranslationKey)} tone={device.keymap_viewer_status === "available" ? "ok" : device.keymap_viewer_status === "locked" ? "warn" : "muted"} />
          </div>
          <div className="mt-0.5 text-xs text-faint">{t("devices.studio.connection")}: {t("keymap.connection_usb_serial")}</div>
          <div className="mt-2 flex flex-wrap gap-2">
            <Badge label="Port" value={device.port_name} />
            {device.vid !== null && <Badge label="VID" value={hex(device.vid, 4)} />}
            {device.pid !== null && <Badge label="PID" value={hex(device.pid, 4)} />}
            {device.serial_number && <Badge label="S/N" value={device.serial_number} />}
          </div>
          {device.error_code !== "none" && <div className="mt-2 rounded-md bg-amber-50 px-3 py-1.5 text-xs text-amber-700">{t(`devices.studio.error.${device.error_code}` as TranslationKey)}</div>}
        </div>
      </div>
    </div>
  );
}

function hostLinkConnectionIcon(connectionType: ProbeResult["device"]["connection_type"]) {
  if (connectionType === "bluetooth") return <Bluetooth size={18} />;
  if (connectionType === "usb") return <Usb size={18} />;
  return <Keyboard size={18} />;
}

function hostLinkConnectionLabel(connectionType: ProbeResult["device"]["connection_type"]) {
  if (connectionType === "bluetooth") return "Bluetooth";
  if (connectionType === "usb") return "USB";
  return "Unknown";
}

function IconBox({ ok, icon, title }: { ok: boolean; icon: React.ReactNode; title?: string }) {
  return <div title={title} aria-label={title} className={`mt-0.5 flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-lg ${ok ? "bg-accent-soft text-accent-deep" : "bg-plate text-disabled"}`}>{icon}</div>;
}

function StatusPill({ ok, label }: { ok: boolean; label: string }) {
  const Icon = ok ? CheckCircle2 : XCircle;
  return <span className={`flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium ${ok ? "bg-accent-soft text-accent-deep" : "bg-plate text-muted"}`}><Icon size={10} /> {label}</span>;
}

function StudioPill({ label, tone }: { label: string; tone: "ok" | "warn" | "muted" }) {
  const color = tone === "ok" ? "bg-accent-soft text-accent-deep" : tone === "warn" ? "bg-amber-100 text-amber-700" : "bg-plate text-muted";
  return <span className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${color}`}>{label}</span>;
}

function DeviceBadges({ device }: { device: ProbeResult["device"] }) {
  return (
    <div className="mt-2 flex flex-wrap gap-2">
      <Badge label="VID" value={hex(device.vendor_id, 4)} />
      <Badge label="PID" value={hex(device.product_id, 4)} />
      <Badge label="Usage Page" value={hex(device.usage_page, 4)} />
      <Badge label="Usage" value={hex(device.usage, 2)} />
      {device.serial_number && <Badge label="S/N" value={device.serial_number} />}
    </div>
  );
}

function studioSupportedLabel(device: StudioDeviceStatus, t: (key: TranslationKey) => string) {
  if (device.rpc_status !== "ok") return t("devices.studio.not_detected");
  if (device.lock_state === "locked") return t("devices.studio.locked");
  if (device.lock_state === "unlocked") return t("devices.studio.unlocked");
  return t("devices.studio.supported");
}

function hex(n: number, digits: number) {
  return `0x${n.toString(16).toUpperCase().padStart(digits, "0")}`;
}

function Badge({ label, value }: { label: string; value: string }) {
  return <span className="inline-flex items-center gap-1 rounded-md bg-plate px-2 py-0.5 text-[11px]"><span className="text-muted">{label}:</span><span className="font-mono text-ink">{value}</span></span>;
}
