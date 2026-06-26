import { useEffect, useMemo, useRef, useState } from "react";
import {
  List,
  Zap,
  Clock,
  ChartColumn,
  Usb,
  Keyboard,
  Settings,
  type LucideIcon,
} from "lucide-react";
import { supportedDeviceCount } from "../lib/deviceCards";
import type { MonitorStatus, Page, StudioDeviceStatus } from "../types";
import { useLang, type TranslationKey } from "../i18n";

interface NavItem {
  id: Page;
  labelKey: TranslationKey;
  icon: LucideIcon;
}

const NAV_ITEMS: NavItem[] = [
  { id: "devices", labelKey: "nav.devices", icon: Usb },
  { id: "rules", labelKey: "nav.rules", icon: List },
  { id: "actions", labelKey: "nav.actions", icon: Zap },
  { id: "timesync", labelKey: "nav.timesync", icon: Clock },
  { id: "ai_usage", labelKey: "nav.ai_usage", icon: ChartColumn },
  { id: "keymap_viewer", labelKey: "nav.keymap_viewer", icon: Keyboard },
  { id: "settings", labelKey: "nav.settings", icon: Settings },
];

interface Props {
  currentPage: Page;
  onNavigate: (page: Page) => void;
  status: MonitorStatus;
  studioDevices: StudioDeviceStatus[];
}

export function Sidebar({ currentPage, onNavigate, status, studioDevices }: Props) {
  const { lang, setLang, t } = useLang();
  const connectedDeviceCount = useMemo(
    () => supportedDeviceCount(status.host_link_devices, studioDevices, status.device_battery),
    [status.device_battery, status.host_link_devices, studioDevices]
  );

  // Pulse the status dot briefly whenever the active layer changes.
  const [pulse, setPulse] = useState(false);
  const prevLayer = useRef(status.current_layer);
  useEffect(() => {
    if (status.current_layer === prevLayer.current) return;
    prevLayer.current = status.current_layer;
    if (status.current_layer === null) return;
    setPulse(true);
    const timer = setTimeout(() => setPulse(false), 700);
    return () => clearTimeout(timer);
  }, [status.current_layer]);

  return (
    <aside className="flex w-60 flex-col bg-surface text-ink select-none flex-shrink-0 border-r border-border">
      {/* Logo: white "power button" circle with the accent keyboard glyph */}
      <div className="flex items-center gap-3 px-5 py-5 border-b border-background">
        <div className="flex h-11 w-11 items-center justify-center rounded-full bg-surface shadow-neu-sel">
          <Keyboard size={22} className="text-accent" aria-hidden="true" />
        </div>
        <span className="text-sm font-medium">RawHID Host</span>
      </div>

      {/* Navigation */}
      <nav className="flex-1 px-3 py-4 space-y-0.5">
        {NAV_ITEMS.map((item) => {
          const active = currentPage === item.id;
          const Icon = item.icon;
          return (
            <button
              key={item.id}
              onClick={() => onNavigate(item.id)}
              className={`flex w-full items-center gap-3 rounded-pill px-3 py-2.5 text-sm font-medium ${
                active
                  ? "bg-plate text-accent-deep shadow-neu-sel-in"
                  : "nav-item text-muted hover:bg-plate hover:text-ink"
              }`}
            >
              <Icon
                size={16}
                className={active ? "text-accent" : "nav-icon text-faint"}
              />
              {t(item.labelKey)}
            </button>
          );
        })}
      </nav>

      {/* Footer */}
      <div className="border-t border-background px-4 py-4 space-y-3">
        {/* Status */}
        <div className="flex items-center gap-2.5">
          <span
            className={`h-2 w-2 rounded-full flex-shrink-0 ${
              status.running ? "bg-accent" : "bg-disabled"
            } ${pulse ? "animate-layer-pulse" : ""}`}
          />
          <div className="min-w-0">
            <div className="text-xs font-medium text-ink truncate">
              {status.running ? t("sidebar.running") : t("sidebar.stopped")}
            </div>
            {status.running && (
              <div className="text-[10px] text-faint truncate">
                {t("sidebar.devices_connected", {
                  n: connectedDeviceCount,
                })}
              </div>
            )}
            {status.last_error && !status.running && (
              <div
                className="text-[10px] text-red-500 truncate"
                title={status.last_error}
              >
                {t("sidebar.error")}
              </div>
            )}
          </div>
        </div>

        {/* Language toggle */}
        <div className="flex items-center gap-1">
          <button
            onClick={() => setLang("ja")}
            className={`rounded px-2 py-0.5 text-xs font-medium transition-colors ${
              lang === "ja"
                ? "bg-plate text-accent-deep shadow-neu-sel-in"
                : "text-faint hover:text-ink"
            }`}
          >
            JP
          </button>
          <span className="text-disabled text-xs">/</span>
          <button
            onClick={() => setLang("en")}
            className={`rounded px-2 py-0.5 text-xs font-medium transition-colors ${
              lang === "en"
                ? "bg-plate text-accent-deep shadow-neu-sel-in"
                : "text-faint hover:text-ink"
            }`}
          >
            EN
          </button>
        </div>
      </div>
    </aside>
  );
}
