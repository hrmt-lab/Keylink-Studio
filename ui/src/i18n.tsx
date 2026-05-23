import {
  createContext,
  useContext,
  useState,
  useEffect,
  type ReactNode,
} from "react";

export type Lang = "ja" | "en";

const T = {
  ja: {
    // App
    "app.loading": "読み込み中...",

    // Sidebar
    "nav.dashboard": "ダッシュボード",
    "nav.rules": "レイヤールール",
    "nav.timesync": "時刻同期",
    "nav.devices": "デバイス",
    "nav.settings": "設定",
    "sidebar.subtitle": "キーボードレイヤー管理",
    "sidebar.running": "監視中",
    "sidebar.stopped": "停止中",
    "sidebar.devices_connected": "{n} デバイス接続",
    "sidebar.error": "エラー",

    // Dashboard
    "dashboard.title": "ダッシュボード",
    "dashboard.subtitle": "キーボードレイヤーの監視状況",
    "dashboard.start": "監視開始",
    "dashboard.stop": "監視停止",
    "dashboard.status.label": "ステータス",
    "dashboard.status.running": "監視中",
    "dashboard.status.stopped": "停止中",
    "dashboard.devices.label": "接続デバイス",
    "dashboard.devices.unit": "台",
    "dashboard.devices.none": "なし",
    "dashboard.tz.auto": "システム自動",
    "dashboard.layer_switch": "レイヤー切替",
    "dashboard.timesync": "時刻同期",
    "dashboard.feature.rules_count": "ルール数",
    "dashboard.feature.rules_unset": "未設定",
    "dashboard.feature.rules_others": "他 {n} 件...",
    "dashboard.feature.polling": "ポーリング間隔",
    "dashboard.feature.format": "表示フォーマット",
    "dashboard.feature.clock_mode": "時計モード",
    "dashboard.feature.clock_24h": "24時間",
    "dashboard.feature.clock_12h": "12時間",
    "dashboard.feature.periodic_sync": "定期同期",
    "dashboard.feature.periodic_sync.change": "表示変化時のみ",
    "dashboard.feature.periodic_sync.seconds": "{n} 秒ごと",
    "dashboard.feature.timezone": "タイムゾーン",
    "dashboard.layer_switch.disabled":
      "レイヤー切替は無効です。「レイヤールール」ページで設定できます。",
    "dashboard.timesync.disabled":
      "時刻同期は無効です。「時刻同期」ページで設定できます。",
    "dashboard.error.title": "エラー",
    "dashboard.log.title": "アクティビティログ",
    "dashboard.log.empty": "ログがありません",
    "dashboard.log.count": "{n} 件",

    // Rules
    "rules.title": "レイヤールール",
    "rules.subtitle": "実行中のアプリをクリックして追加",
    "rules.running_warn": "監視中 — 再起動で反映",
    "rules.toggle_label": "レイヤー切替",
    "rules.search_placeholder": "検索...",
    "rules.not_found": "見つかりません",
    "rules.select_hint": "← 左のリストからアプリを選択してください",
    "rules.layer": "レイヤー",
    "rules.add": "追加",
    "rules.duplicate": "同じルールがすでに存在します",
    "rules.count": "設定済みルール ({n})",
    "rules.empty.title": "ルールがまだありません",
    "rules.empty.hint": "左のリストからアプリを選んで追加しましょう",

    // TimeSync
    "timesync.title": "時刻同期",
    "timesync.subtitle": "キーボードへ現在時刻を送信します",
    "timesync.save": "保存",
    "timesync.saved": "保存しました!",
    "timesync.enable": "時刻同期を有効にする",
    "timesync.enable.desc": "定期的に PC の現在時刻をキーボードへ送信します",
    "timesync.format": "表示フォーマット",
    "timesync.clock_mode": "12時間表示",
    "timesync.clock_mode.desc": "オフの場合は 24 時間表示になります",
    "timesync.sync_interval": "定期同期間隔",
    "timesync.sync_interval.desc":
      "0 に設定すると表示が変わる瞬間にのみ送信します",
    "timesync.sync_interval.unit": "秒",
    "timesync.timezone": "タイムゾーンオフセット",
    "timesync.timezone.desc":
      "空白の場合はシステムのタイムゾーンを使用します (分単位, 例: 540 = UTC+9)",
    "timesync.timezone.unit": "分",
    "timesync.timezone.auto": "自動",
    "timesync.disabled.hint":
      "有効にすると、キーボードのディスプレイに時刻を表示できます。",

    // Format hints (used in both TimeSync and Dashboard)
    "format.time_hm": "時:分",
    "format.time_hms": "時:分:秒",
    "format.date_ymd": "年-月-日",
    "format.date_md": "月-日",
    "format.datetime_hm": "年-月-日 時:分",
    "format.weekday_hm": "曜日 時:分",

    // Devices
    "devices.title": "デバイス",
    "devices.subtitle": "接続されている Raw HID デバイスを確認します",
    "devices.scan": "スキャン",
    "devices.scanning": "スキャン中...",
    "devices.scanning.hint": "デバイスをスキャン中...",
    "devices.empty": "デバイスが見つかりません",
    "devices.empty.hint":
      "Usage Page 0xFF60 / Usage 0x61 に対応したデバイスを確認してください",
    "devices.ok": "応答あり",
    "devices.ng": "応答なし",
    "devices.summary": "{ok} / {total} デバイスが応答しました",

    // Settings
    "settings.title": "設定",
    "settings.subtitle": "ポーリングと HID デバイスの設定",
    "settings.reload": "再読み込み",
    "settings.save": "保存",
    "settings.saved": "保存しました!",
    "settings.polling.section": "ポーリング設定",
    "settings.polling.interval": "ポーリング間隔",
    "settings.polling.interval.desc": "アクティブアプリを確認する頻度",
    "settings.hid.section": "HID デバイス設定",
    "settings.hid.usage_page": "Usage Page",
    "settings.hid.usage_page.desc": "HID Usage Page (16進数表示)",
    "settings.hid.usage": "Usage",
    "settings.hid.usage.desc": "HID Usage ID (16進数表示)",
    "settings.hid.timeout": "HELLO タイムアウト",
    "settings.hid.timeout.desc": "デバイス検証の待機時間",
    "settings.note1": "設定は {file} に保存されます。",
    "settings.note2":
      "デフォルト Usage Page は {up}、Usage は {u} (ZMK/QMK 標準) です。",
  },
  en: {
    // App
    "app.loading": "Loading...",

    // Sidebar
    "nav.dashboard": "Dashboard",
    "nav.rules": "Layer Rules",
    "nav.timesync": "Time Sync",
    "nav.devices": "Devices",
    "nav.settings": "Settings",
    "sidebar.subtitle": "Keyboard Layer Manager",
    "sidebar.running": "Monitoring",
    "sidebar.stopped": "Stopped",
    "sidebar.devices_connected": "{n} device(s) connected",
    "sidebar.error": "Error",

    // Dashboard
    "dashboard.title": "Dashboard",
    "dashboard.subtitle": "Keyboard layer monitoring status",
    "dashboard.start": "Start Monitoring",
    "dashboard.stop": "Stop Monitoring",
    "dashboard.status.label": "Status",
    "dashboard.status.running": "Active",
    "dashboard.status.stopped": "Stopped",
    "dashboard.devices.label": "Connected Devices",
    "dashboard.devices.unit": "",
    "dashboard.devices.none": "None",
    "dashboard.tz.auto": "System auto",
    "dashboard.layer_switch": "Layer Switch",
    "dashboard.timesync": "Time Sync",
    "dashboard.feature.rules_count": "Rules",
    "dashboard.feature.rules_unset": "Not set",
    "dashboard.feature.rules_others": "+{n} more...",
    "dashboard.feature.polling": "Poll interval",
    "dashboard.feature.format": "Display format",
    "dashboard.feature.clock_mode": "Clock mode",
    "dashboard.feature.clock_24h": "24h",
    "dashboard.feature.clock_12h": "12h",
    "dashboard.feature.periodic_sync": "Periodic sync",
    "dashboard.feature.periodic_sync.change": "On display change only",
    "dashboard.feature.periodic_sync.seconds": "Every {n}s",
    "dashboard.feature.timezone": "Timezone",
    "dashboard.layer_switch.disabled":
      'Layer switch is disabled. Configure it on the "Layer Rules" page.',
    "dashboard.timesync.disabled":
      'Time sync is disabled. Configure it on the "Time Sync" page.',
    "dashboard.error.title": "Error",
    "dashboard.log.title": "Activity Log",
    "dashboard.log.empty": "No log entries",
    "dashboard.log.count": "{n} entries",

    // Rules
    "rules.title": "Layer Rules",
    "rules.subtitle": "Click a running app to add a rule",
    "rules.running_warn": "Monitoring — restart to apply",
    "rules.toggle_label": "Layer Switch",
    "rules.search_placeholder": "Search...",
    "rules.not_found": "No results",
    "rules.select_hint": "← Select an app from the left list",
    "rules.layer": "Layer",
    "rules.add": "Add",
    "rules.duplicate": "This rule already exists",
    "rules.count": "Configured rules ({n})",
    "rules.empty.title": "No rules yet",
    "rules.empty.hint": "Select an app from the left and add a rule",

    // TimeSync
    "timesync.title": "Time Sync",
    "timesync.subtitle": "Send current time to keyboard",
    "timesync.save": "Save",
    "timesync.saved": "Saved!",
    "timesync.enable": "Enable time sync",
    "timesync.enable.desc":
      "Periodically send the PC's current time to the keyboard",
    "timesync.format": "Display format",
    "timesync.clock_mode": "12-hour clock",
    "timesync.clock_mode.desc": "When off, 24-hour format is used",
    "timesync.sync_interval": "Sync interval",
    "timesync.sync_interval.desc":
      "Set to 0 to sync only when the displayed value changes",
    "timesync.sync_interval.unit": "sec",
    "timesync.timezone": "Timezone offset",
    "timesync.timezone.desc":
      "Leave blank to use the system timezone (minutes, e.g. 540 = UTC+9)",
    "timesync.timezone.unit": "min",
    "timesync.timezone.auto": "Auto",
    "timesync.disabled.hint":
      "Enable it to display the current time on your keyboard.",

    // Format hints
    "format.time_hm": "H:M",
    "format.time_hms": "H:M:S",
    "format.date_ymd": "Y-M-D",
    "format.date_md": "M-D",
    "format.datetime_hm": "Y-M-D H:M",
    "format.weekday_hm": "Weekday H:M",

    // Devices
    "devices.title": "Devices",
    "devices.subtitle": "Discover connected Raw HID devices",
    "devices.scan": "Scan",
    "devices.scanning": "Scanning...",
    "devices.scanning.hint": "Scanning for devices...",
    "devices.empty": "No devices found",
    "devices.empty.hint":
      "Make sure a device with Usage Page 0xFF60 / Usage 0x61 is connected",
    "devices.ok": "Responded",
    "devices.ng": "No response",
    "devices.summary": "{ok} / {total} device(s) responded",

    // Settings
    "settings.title": "Settings",
    "settings.subtitle": "Polling and HID device configuration",
    "settings.reload": "Reload",
    "settings.save": "Save",
    "settings.saved": "Saved!",
    "settings.polling.section": "Polling",
    "settings.polling.interval": "Poll interval",
    "settings.polling.interval.desc": "How often to check the active application",
    "settings.hid.section": "HID Device",
    "settings.hid.usage_page": "Usage Page",
    "settings.hid.usage_page.desc": "HID Usage Page (hex)",
    "settings.hid.usage": "Usage",
    "settings.hid.usage.desc": "HID Usage ID (hex)",
    "settings.hid.timeout": "HELLO timeout",
    "settings.hid.timeout.desc": "Timeout for device verification",
    "settings.note1": "Config is saved to {file}.",
    "settings.note2":
      "Default Usage Page is {up}, Usage is {u} (ZMK/QMK standard).",
  },
} as const;

export type TranslationKey = keyof typeof T.ja;

interface LangContextType {
  lang: Lang;
  setLang: (l: Lang) => void;
  t: (key: TranslationKey, params?: Record<string, string | number>) => string;
}

const LangContext = createContext<LangContextType>({
  lang: "ja",
  setLang: () => {},
  t: (key) => key,
});

const STORAGE_KEY = "rawhid-host-lang";

export function LangProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(() => {
    const stored = localStorage.getItem(STORAGE_KEY);
    return stored === "en" ? "en" : "ja";
  });

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, lang);
  }, [lang]);

  const setLang = (l: Lang) => setLangState(l);

  const t = (key: TranslationKey, params?: Record<string, string | number>): string => {
    let str: string = T[lang][key] ?? T.ja[key] ?? key;
    if (params) {
      for (const [k, v] of Object.entries(params)) {
        str = str.replace(`{${k}}`, String(v));
      }
    }
    return str;
  };

  return (
    <LangContext.Provider value={{ lang, setLang, t }}>
      {children}
    </LangContext.Provider>
  );
}

export function useLang() {
  return useContext(LangContext);
}
