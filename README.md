# RawHID Host

RawHID Host は、Windows 上で動く ZMK キーボード向けのホストアプリです。
前面にあるアプリに応じてキーボードのレイヤーを切り替えたり、キーボードの表示用に時刻や AI 使用量の情報を Raw HID で送信できます。

このリポジトリに含まれるのは **PC 側のホストアプリのみ** です。キーボード側の ZMK ファームウェアには、`HL` packet protocol を Raw HID で受け取る実装が別途必要です。

## 主な機能

- Windows の前面アプリ / プロセス監視
- `path` / `exe` / `title` によるアプリ判定
- アプリごとの ZMK レイヤー切り替え (キーボード単位でルールを設定)
- Raw HID デバイス列挙と `HOST_HELLO` / `DEVICE_HELLO` 検証
- 検証済みデバイスへの packet 送信
- キーボード表示向けの `TIME_SYNC`
- Codex / Claude Code 使用量を送る `AI_USAGE`
- キーボードからの上り通信 (バッテリー残量、タイピング統計、レイヤー逆同期、PC 操作トリガー)
- タイピング統計のヒートマップ表示 (キーマップビューアー内)
- Tauri + React の GUI (アクセント色のカスタマイズ対応)
- CLI によるデバッグ / スクリプト実行

## 構成

```text
rawhid-host/
├─ crates/
│  ├─ rawhid-host-core/   # 設定、packet、HID、runner、AI usage などの中核処理
│  ├─ rawhid-host-cli/    # CLI
│  └─ rawhid-host-tauri/  # Tauri コマンドと監視スレッド
├─ ui/                    # React + TypeScript + Vite UI
├─ docs/                  # 詳細ドキュメント
├─ design-mock/           # UI デザイン方針 (ui-redesign-direction.md) と過去モック
├─ examples/              # 設定例
├─ create-icons.ps1       # アプリアイコン生成
├─ dev.ps1
└─ build-release.ps1
```

## クイックスタート

GUI を開発モードで起動:

```powershell
.\dev.ps1
```

Raw HID 候補を確認:

```powershell
cargo run -p rawhid-host-cli -- list-devices
```

CLI で監視を開始:

```powershell
cargo run -p rawhid-host-cli -- run
```

設定ファイル例を作成:

```powershell
cargo run -p rawhid-host-cli -- init-config --output rawhid-host.toml
```

設定ファイルの探索先を確認:

```powershell
cargo run -p rawhid-host-cli -- config-path
```

## 設定例

```toml
[app]
start_monitoring_on_launch = false

[polling]
interval_ms = 500

[hid]
usage_page = 65376 # 0xFF60
usage = 97         # 0x61
hello_timeout_ms = 200

[layer_switch]
enabled = true

# レイヤールールはデバイスごとに設定します (DEVICE_HELLO の uid 単位)。
# 設定がないキーボードはレイヤー切り替えの対象外です。
[layer_switch.devices."uid:7a91c3e4d102ab55"]
display_name = "Example Keyboard"
enabled = true

[[layer_switch.devices."uid:7a91c3e4d102ab55".rules]]
name = "Notepad"
exe = "notepad.exe"
layer = 1

[time]
enabled = false
format_hint = "time_hm"
clock_mode = "24h"
periodic_sync_sec = 60
# tz_offset_min = 540

[ai_usage]
enabled = false
poll_interval_sec = 300
stale_after_sec = 900

[ai_usage.codex]
enabled = true
# sessions_dir = "C:\\Users\\<user>\\.codex\\sessions"
history_fallback_enabled = true
allow_activity_baseline = false
activity_five_hour_token_baseline = 0
activity_seven_day_token_baseline = 0

[ai_usage.claude_code]
enabled = true
# credentials_path = "C:\\Users\\<user>\\.claude\\.credentials.json"
api_timeout_sec = 10
```

`activity_*_token_baseline` は Codex local history fallback の割合表示に使う仮の分母です。実際の quota limit ではありません。

## GUI 画面

- Dashboard: 監視開始 / 停止、接続状況、現在レイヤー、ログ、AI Usage 簡易サマリ
- Layer Rules: アプリごとのレイヤールール編集。変更は自動保存です。アプリ一覧には実行ファイルから抽出した実アイコンを表示します。
- Actions: キーボードのキーから PC 側操作を実行する `HOST_ACTION` バインディング設定 (画面表示 / 監視停止 / AI 使用量更新 / アプリ起動 / フォルダを開く)。バインディングはアクションID順に並び、パスを持つ動作は編集できます。「アプリを起動」は起動済みなら前面化・未起動なら起動し、起動中アプリピッカー / 参照ボタン / `.lnk` 起動に対応します。既定では無効で、許可リスト制です。
- Time Sync: `TIME_SYNC` の有効化、表示形式、同期間隔などの設定
- AI Usage: Codex / Claude Code 使用量送信の設定、状態表示、手動更新
- Keymap Viewer: キーマップ表示 + タイピング統計ヒートマップ (対応キーボードのみ)
- Devices: Raw HID 候補のスキャンと HELLO 検証結果、バッテリー残量表示
- Settings: 外観 (アクセント色のプリセット / カスタム色)、polling と HID の基本設定、アプリ起動時の自動監視、Windows ログイン時の自動起動

UI は日本語 / 英語切り替えに対応しています。

### UI デザイン

UI は「スタジオ・ガジェット」デザイン (青みグレー背景 × 白カード × アクセント 1 色、操作できる要素だけニューモーフィズムの凹凸) で統一しています。配色・立体ルール・マイクロインタラクションの詳細は [UI デザイン方針](design-mock/ui-redesign-direction.md) を参照してください。

- アクセント色は設定 > 外観から変更できます (プリセット 6 色 + カスタム色)
- フォントは Zen Kaku Gothic New / Spline Sans Mono を同梱しており、実行時に外部からフォントを取得しません

## AI Usage について

AI Usage は既定で無効です。有効にすると Codex / Claude Code の 5h / 7d 使用率と reset 時刻を `AI_USAGE` packet として送信します。

- Codex は session history 内の `rate_limits` を優先します。取得できた場合は quota source として扱います。
- Codex local history fallback は activity estimate です。quota ではないため reset 時刻は送りません。
- Claude Code は OAuth usage API を experimental / best-effort source として扱います。schema 変更、認証失敗、token 期限切れ、credentials 不在が起こり得ます。
- access token、credentials JSON、Authorization header、HTTP body、API response、raw parse error は UI / log / status / Raw HID packet に出しません。

## ビルド

UI のみ:

```powershell
cd ui
npm run build
```

Rust / CLI:

```powershell
cargo build
```

Tauri 開発起動:

```powershell
.\dev.ps1
```

配布用ビルド:

```powershell
.\build-release.ps1
```

生成物は `target/` と `ui/dist/` に作られます。これらはリポジトリに含めず、配布物は GitHub Releases に添付する運用を想定しています。

## 詳細ドキュメント

- [変更履歴](CHANGELOG.md)
- [互換性情報](docs/compatibility.md)
- [セットアップガイド](docs/manual-setup.md)
- [アプリ操作マニュアル](docs/manual-app-usage.md)
- [技術スタックと仕組み](docs/technology-overview.md)
- [技術仕様](docs/spec.md)
- [Packet 仕様](docs/packet-spec.md)
- [UI デザイン方針](design-mock/ui-redesign-direction.md)

---

## English Summary

RawHID Host is a Windows host application for ZMK keyboards. It monitors the foreground application, sends app-layer packets over Raw HID, can synchronize time for keyboard displays, and can optionally send Codex / Claude Code usage snapshots.

This repository contains the host-side app only. The ZMK firmware side must implement the compatible `HL` packet receiver.

See [CHANGELOG.md](CHANGELOG.md), [Compatibility](docs/compatibility.md), and the documentation under `docs/` for setup, usage, architecture, and packet details.

## Current implementation notes

- Foreground app changes are detected instantly via `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` while monitoring is running. Polling remains as a fallback.
- `[app] start_monitoring_on_launch` starts monitoring automatically when the app launches. Launch at Windows login is managed from Settings via the HKCU Run registry key.
- The app is single-instance. Launching a second instance focuses the existing window. Showing the window (tray menu, tray left-click, second-instance launch, or the `show_window` host action) restores it from a minimized or tray-hidden state.
- The tray menu includes start/stop monitoring items in addition to show/quit. While monitoring a BATTERY-capable keyboard, the tray tooltip shows per-device battery levels (e.g. `L 90% / R 88%`) on hover.
- `hid.rescan_interval_sec` controls periodic Host Link HID rescan while monitoring is running. The default is 5 seconds.
- AI Usage collection runs independently from monitoring. The UI can refresh usage while monitoring is stopped; Raw HID sending still happens only while monitoring is running. After a manual refresh completes, the backend emits a `status-update` event even when monitoring is stopped.
- Dashboard quick toggles are auto-saved. A brief "saved" indicator is shown on success. On save failure, an error is shown and the UI state rolls back to the previous config.
- Pages with a Save button do not show a success message. They only show an error when saving fails.
- Claude Code credentials can be auto-detected from Windows default, WSL default, and extra credentials paths. On API 401 / 403, the next valid candidate is tried.
- Keymap Viewer is a read-only ZMK Studio RPC viewer. It uses USB serial / CDC ACM transport in v1 and does not edit, write, save, restore, or unlock Studio state.
- App Layer rules are configured per device, keyed by the non-zero `device_uid_hash` from `DEVICE_HELLO`. There is no global rule set: devices without a device-specific config are not layer-managed. Devices without the `APP_LAYER` capability are listed but do not receive `APP_LAYER` packets.
- Device-initiated (uplink) packets are supported while monitoring: battery status, host actions, key statistics, and layer-state reports, each gated by its own `DEVICE_HELLO` capability bit. Uplink is best-effort; packets sent while monitoring is stopped are lost.
- Key statistics record per-position press counts only (never key contents) and are stored locally per device. The heatmap lives in the Keymap Viewer.
- `HOST_ACTION` execution is disabled by default and allowlist-based (`[actions]` in the config file).
- The UI follows the "studio gadget" design language (see `design-mock/ui-redesign-direction.md`). The accent color is user-configurable from Settings > Appearance; it is a UI-only preference stored in localStorage, not in the config file. UI fonts (Zen Kaku Gothic New, Spline Sans Mono) are bundled; nothing is fetched at runtime.
