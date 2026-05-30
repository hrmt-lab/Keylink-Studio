import { useState, useEffect, useCallback } from "react";
import { RefreshCw, CheckCircle2, XCircle, Usb, AlertCircle } from "lucide-react";
import { probeDevices } from "../api";
import { useLang } from "../i18n";
import type { ProbeResult } from "../types";

export default function Devices() {
  const { t } = useLang();
  const [results, setResults] = useState<ProbeResult[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleProbe = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await probeDevices();
      setResults(res);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

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

      {error && (
        <div className="flex items-start gap-2.5 rounded-lg bg-red-50 px-4 py-3 text-sm text-red-700 ring-1 ring-red-200">
          <AlertCircle size={15} className="mt-0.5 flex-shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {results === null ? (
        <div className="rounded-xl bg-white shadow-card ring-1 ring-border px-6 py-14 text-center">
          <div className="mx-auto mb-3 h-8 w-8 animate-spin rounded-full border-2 border-border border-t-primary" />
          <p className="text-sm text-gray-400">{t("devices.scanning.hint")}</p>
        </div>
      ) : results.length === 0 ? (
        <div className="rounded-xl bg-white shadow-card ring-1 ring-border px-6 py-14 text-center">
          <XCircle size={36} className="mx-auto text-gray-200 mb-3" />
          <p className="text-sm text-gray-500 font-medium">{t("devices.empty")}</p>
          <p className="text-xs text-gray-400 mt-1">{t("devices.empty.hint")}</p>
        </div>
      ) : (
        <div className="space-y-3">
          {results.map((result, idx) => (
            <DeviceCard key={idx} result={result} />
          ))}
        </div>
      )}

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

function DeviceCard({ result }: { result: ProbeResult }) {
  const { t } = useLang();
  const { device, verified, error } = result;
  const name = device.product ?? device.manufacturer ?? "Unknown Device";

  return (
    <div
      className={`rounded-xl bg-white shadow-card ring-1 transition-all ${
        verified ? "ring-emerald-200 bg-emerald-50/30" : "ring-border"
      }`}
    >
      <div className="flex items-start gap-4 px-5 py-4">
        <div
          className={`mt-0.5 flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-lg ${
            verified ? "bg-emerald-100" : "bg-gray-100"
          }`}
        >
          <Usb size={18} className={verified ? "text-emerald-600" : "text-gray-400"} />
        </div>

        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="font-medium text-gray-800 text-sm">{name}</span>
            {verified ? (
              <span className="flex items-center gap-1 rounded-full bg-emerald-100 px-2 py-0.5 text-[11px] font-medium text-emerald-700">
                <CheckCircle2 size={10} /> {t("devices.ok")}
              </span>
            ) : (
              <span className="flex items-center gap-1 rounded-full bg-gray-100 px-2 py-0.5 text-[11px] font-medium text-gray-500">
                <XCircle size={10} /> {t("devices.ng")}
              </span>
            )}
          </div>

          {device.manufacturer && (
            <div className="text-xs text-gray-400 mt-0.5">{device.manufacturer}</div>
          )}

          <div className="mt-2 flex flex-wrap gap-2">
            <Badge label="VID" value={hex(device.vendor_id, 4)} />
            <Badge label="PID" value={hex(device.product_id, 4)} />
            <Badge label="Usage Page" value={hex(device.usage_page, 4)} />
            <Badge label="Usage" value={hex(device.usage, 2)} />
            {device.serial_number && (
              <Badge label="S/N" value={device.serial_number} />
            )}
          </div>

          <div className="mt-1.5 font-mono text-[10px] text-gray-400 truncate" title={device.path}>
            {device.path}
          </div>

          {error && (
            <div className="mt-2 rounded-md bg-red-50 px-3 py-1.5 text-xs text-red-600">
              {error}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function hex(n: number, digits: number) {
  return `0x${n.toString(16).toUpperCase().padStart(digits, "0")}`;
}

function Badge({ label, value }: { label: string; value: string }) {
  return (
    <span className="inline-flex items-center gap-1 rounded-md bg-background px-2 py-0.5 text-[11px] ring-1 ring-border">
      <span className="text-gray-400">{label}:</span>
      <span className="font-mono text-gray-600">{value}</span>
    </span>
  );
}
