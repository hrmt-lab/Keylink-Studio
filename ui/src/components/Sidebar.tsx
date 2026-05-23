import {
  LayoutDashboard,
  List,
  Clock,
  Usb,
  Settings,
  type LucideIcon,
} from "lucide-react";
import appIcon from "../assets/app-icon.png";
import type { MonitorStatus, Page } from "../types";
import { useLang, type TranslationKey } from "../i18n";

interface NavItem {
  id: Page;
  labelKey: TranslationKey;
  icon: LucideIcon;
}

const NAV_ITEMS: NavItem[] = [
  { id: "dashboard", labelKey: "nav.dashboard", icon: LayoutDashboard },
  { id: "rules", labelKey: "nav.rules", icon: List },
  { id: "timesync", labelKey: "nav.timesync", icon: Clock },
  { id: "devices", labelKey: "nav.devices", icon: Usb },
  { id: "settings", labelKey: "nav.settings", icon: Settings },
];

interface Props {
  currentPage: Page;
  onNavigate: (page: Page) => void;
  status: MonitorStatus;
}

export function Sidebar({ currentPage, onNavigate, status }: Props) {
  const { lang, setLang, t } = useLang();

  return (
    <aside className="flex w-60 flex-col bg-primary text-white select-none flex-shrink-0">
      {/* Logo */}
      <div className="flex items-center gap-3 px-5 py-5 border-b border-white/10">
        <div className="flex h-12 w-12 items-center justify-center overflow-hidden rounded-xl bg-white/15 shadow-sm ring-1 ring-white/10">
          <img
            src={appIcon}
            alt=""
            aria-hidden="true"
            className="h-12 w-12 object-cover"
          />
        </div>
        <span className="text-sm font-semibold">RawHID Host</span>
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
              className={`flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-sm font-medium transition-all ${
                active
                  ? "bg-white/20 text-white"
                  : "text-white/60 hover:bg-white/10 hover:text-white/90"
              }`}
            >
              <Icon
                size={16}
                className={active ? "text-white" : "text-white/60"}
              />
              {t(item.labelKey)}
            </button>
          );
        })}
      </nav>

      {/* Footer */}
      <div className="border-t border-white/10 px-4 py-4 space-y-3">
        {/* Status */}
        <div className="flex items-center gap-2.5">
          <span
            className={`h-2 w-2 rounded-full flex-shrink-0 ${
              status.running
                ? "bg-emerald-400 shadow-[0_0_6px_rgba(52,211,153,0.6)]"
                : "bg-white/30"
            }`}
          />
          <div className="min-w-0">
            <div className="text-xs font-medium text-white/80 truncate">
              {status.running ? t("sidebar.running") : t("sidebar.stopped")}
            </div>
            {status.running && (
              <div className="text-[10px] text-white/40 truncate">
                {t("sidebar.devices_connected", {
                  n: status.connected_devices,
                })}
              </div>
            )}
            {status.last_error && !status.running && (
              <div
                className="text-[10px] text-red-300 truncate"
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
                ? "bg-white/20 text-white"
                : "text-white/40 hover:text-white/70"
            }`}
          >
            JP
          </button>
          <span className="text-white/20 text-xs">/</span>
          <button
            onClick={() => setLang("en")}
            className={`rounded px-2 py-0.5 text-xs font-medium transition-colors ${
              lang === "en"
                ? "bg-white/20 text-white"
                : "text-white/40 hover:text-white/70"
            }`}
          >
            EN
          </button>
        </div>
      </div>
    </aside>
  );
}

