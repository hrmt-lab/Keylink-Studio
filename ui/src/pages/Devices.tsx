import { useState, useEffect, useCallback } from "react";
import { RefreshCw, CheckCircle2, XCircle, Usb, AlertCircle, Keyboard } from "lucide-react";
import { probeDevices } from "../api";
import { useLang, type TranslationKey } from "../i18n";
import type { ProbeResult, StudioDeviceStatus } from "../types";

interface DevicesProps {
  studioDevices: StudioDeviceStatus[];
  studioScanning: boolean;
  studioError: string | null;
  refreshStudioDevices: () => Promise<StudioDeviceStatus[]>;
}

export default function Devices({ studioDevices, studioScanning, studioError, refreshStudioDevices }: DevicesProps) {
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

  useEffect(() => { handleProbe(); }, [handleProbe]);

  return (
    <div className="p-6 max-w-3xl mx-auto space-y-5">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold text-gray-800">{t("devices.title")}</h1>
          <p className="text-sm text-gray-500 mt-0.5">{t("devices.subtitle")}</p>
        </div>
        <button
          onClick={handleProbe}
          disabled={loading}
          className="flex items-center gap-2 rounded-lg bg-primary px-4 py-2.5 text-sm font-medium text-white hover:bg-primary-dark disabled:opacity-60 transition-colors"
        >
          <RefreshCw size={15} className={loading ? "animate-spin" : ""} />
          {loading ? t("devices.scanning") : t("devices.scan")}
        </button>
      </div>

      {(error || studioError) && (
        <div className="flex items-start gap-2.5 rounded-lg bg-red-50 px-4 py-3 text-sm text-red-700 ring-1 ring-red-200">
          <AlertCircle size={15} className="mt-0.5 flex-shrink-0" />
          <span>{error ?? studioError}</span>
        </div>
      )}

      <DeviceSection title={t("devices.host_link.section")} count={results?.length ?? 0}>
        {results === null || hostLinkLoading ? (
          <LoadingCard text={t("devices.scanning.hint")} />
        ) : results.length === 0 ? (
          <EmptyCard title={t("devices.empty")} body={t("devices.empty.hint")} />
        ) : (
          <div className="space-y-3">{results.map((result, idx) => <HostLinkDeviceCard key={idx} result={result} />)}</div>
        )}
      </DeviceSection>

      <DeviceSection title={t("devices.studio.section")} count={studioDevices.length}>
        {studioScanning ? (
          <LoadingCard text={t("devices.scanning.hint")} />
        ) : studioDevices.length === 0 ? (
          <EmptyCard title={t("devices.studio.empty")} body={t("devices.studio.empty.hint")} />
        ) : (
          <div className="space-y-3">{studioDevices.map((device) => <StudioDeviceCard key={device.id} device={device} />)}</div>
        )}
      </DeviceSection>

      {results !== null && (
        <p className="text-xs text-gray-400 text-center">
          {t("devices.summary", {
            ok: results.filter((r) => r.verified).length,
            total: results.length,
          })}
        </p>
      )}
    </div>
  );
}

function DeviceSection({ title, count, children }: { title: string; count: number; children: React.ReactNode }) {
  return (
    <section className="space-y-3">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-gray-700">{title}</h2>
        <span className="text-xs text-gray-400">{count}</span>
      </div>
      {children}
    </section>
  );
}

function LoadingCard({ text }: { text: string }) {
  return (
    <div className="rounded-xl bg-white shadow-card ring-1 ring-border px-6 py-10 text-center">
      <div className="mx-auto mb-3 h-8 w-8 animate-spin rounded-full border-2 border-border border-t-primary" />
      <p className="text-sm text-gray-400">{text}</p>
    </div>
  );
}

function EmptyCard({ title, body }: { title: string; body: string }) {
  return (
    <div className="rounded-xl bg-white shadow-card ring-1 ring-border px-6 py-10 text-center">
      <XCircle size={36} className="mx-auto text-gray-200 mb-3" />
      <p className="text-sm text-gray-500 font-medium">{title}</p>
      <p className="text-xs text-gray-400 mt-1">{body}</p>
    </div>
  );
}

function HostLinkDeviceCard({ result }: { result: ProbeResult }) {
  const { t } = useLang();
  const { device, verified, error } = result;
  const name = device.product ?? device.manufacturer ?? "Unknown Device";

  return (
    <div className={`rounded-xl bg-white shadow-card ring-1 transition-all ${verified ? "ring-emerald-200 bg-emerald-50/30" : "ring-border"}`}>
      <div className="flex items-start gap-4 px-5 py-4">
        <IconBox ok={verified} icon={<Usb size={18} />} />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="font-medium text-gray-800 text-sm">{name}</span>
            <StatusPill ok={verified} label={verified ? t("devices.host_link.ok") : t("devices.host_link.not_supported")} />
          </div>
          {device.manufacturer && <div className="text-xs text-gray-400 mt-0.5">{device.manufacturer}</div>}
          <DeviceBadges device={device} />
          {verified && <HostLinkCapabilityBadges device={device} />}
          <div className="mt-1.5 font-mono text-[10px] text-gray-400 truncate" title={device.path}>{device.path}</div>
          {error && <div className="mt-2 rounded-md bg-red-50 px-3 py-1.5 text-xs text-red-600">{error}</div>}
        </div>
      </div>
    </div>
  );
}

function HostLinkCapabilityBadges({ device }: { device: ProbeResult["device"] }) {
  const { t } = useLang();
  const capabilities = capabilityLabels(device.capabilities);
  const appLayerSupported = (device.capabilities & 1) !== 0;
  return (
    <div className="mt-2 flex flex-wrap gap-2">
      <Badge label="UID" value={device.device_uid_hash === null ? t("devices.host_link.uid_none" as TranslationKey) : device.device_uid_hash} />
      <Badge label={t("devices.host_link.capabilities" as TranslationKey)} value={capabilities.length === 0 ? t("devices.host_link.capabilities_none" as TranslationKey) : capabilities.join(", ")} />
      <span className={`inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-[11px] ring-1 ${appLayerSupported ? "bg-emerald-50 text-emerald-700 ring-emerald-200" : "bg-gray-50 text-gray-500 ring-border"}`}>
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
  return labels;
}

function StudioDeviceCard({ device }: { device: StudioDeviceStatus }) {
  const { t } = useLang();
  const supported = device.rpc_status === "ok";
  const locked = device.lock_state === "locked";
  return (
    <div className={`rounded-xl bg-white shadow-card ring-1 transition-all ${supported ? "ring-blue-200 bg-blue-50/20" : "ring-border"}`}>
      <div className="flex items-start gap-4 px-5 py-4">
        <IconBox ok={supported} icon={<Keyboard size={18} />} />
        <div className="flex-1 min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium text-gray-800 text-sm">{device.display_name}</span>
            <StudioPill label={studioSupportedLabel(device, t)} tone={supported ? (locked ? "warn" : "ok") : "muted"} />
            <StudioPill label={t(`keymap.viewer.${device.keymap_viewer_status}` as TranslationKey)} tone={device.keymap_viewer_status === "available" ? "ok" : device.keymap_viewer_status === "locked" ? "warn" : "muted"} />
          </div>
          <div className="mt-0.5 text-xs text-gray-400">{t("devices.studio.connection")}: {t("keymap.connection_usb_serial")}</div>
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

function IconBox({ ok, icon }: { ok: boolean; icon: React.ReactNode }) {
  return <div className={`mt-0.5 flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-lg ${ok ? "bg-emerald-100 text-emerald-600" : "bg-gray-100 text-gray-400"}`}>{icon}</div>;
}

function StatusPill({ ok, label }: { ok: boolean; label: string }) {
  const Icon = ok ? CheckCircle2 : XCircle;
  return <span className={`flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium ${ok ? "bg-emerald-100 text-emerald-700" : "bg-gray-100 text-gray-500"}`}><Icon size={10} /> {label}</span>;
}

function StudioPill({ label, tone }: { label: string; tone: "ok" | "warn" | "muted" }) {
  const color = tone === "ok" ? "bg-emerald-100 text-emerald-700" : tone === "warn" ? "bg-amber-100 text-amber-700" : "bg-gray-100 text-gray-500";
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
  return <span className="inline-flex items-center gap-1 rounded-md bg-background px-2 py-0.5 text-[11px] ring-1 ring-border"><span className="text-gray-400">{label}:</span><span className="font-mono text-gray-600">{value}</span></span>;
}