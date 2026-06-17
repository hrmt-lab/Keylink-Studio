# RawHID Host

RawHID Host は、Windows 上で動く ZMK キーボード向けのホストアプリです。
前面アプリに応じたレイヤー切り替え、時刻同期、AI 使用量送信、キーボードから PC へのアクション実行、ZMK Studio RPC 経由のキーマップ閲覧・編集を扱います。

このリポジトリに含まれるのは **PC 側のホストアプリのみ** です。Raw HID の Host Link 機能を使うには、ZMK firmware 側に `HL` packet protocol を受け取る実装が必要です。ZMK Studio キーマップ機能は Host Link とは別経路の USB serial / CDC ACM transport を使います。

## 主な機能

- Windows の前面アプリ / プロセス監視
- `path` / `exe` / `title` によるアプリ判定
- デバイス単位の ZMK レイヤールール
- Raw HID デバイス検出と `HOST_HELLO` / `DEVICE_HELLO` 検証
- `APP_LAYER` / `TIME_SYNC` / `AI_USAGE` packet 送信
- キーボードからの uplink 受信
  - バッテリー残量
  - PC 操作トリガー (`HOST_ACTION`)
  - タイピング統計 (`KEY_STATS`)
  - レイヤー逆同期 (`LAYER_STATE`)
  - キーテスター用リアルタイム押下イベント (`KEY_PRESS`)
- ZMK Studio キーマップ表示・編集
  - 通常キー、透過、無効
  - `MO` / `TG` / `TO`
  - `MT` / `LT`
  - Sticky、Bluetooth、Output、Mouse、Utility、System 系 behavior
  - レイヤー追加 / 名前変更 / 削除
  - 保存 / 破棄による staged edit
- Tauri + React の GUI
- CLI によるデバッグ / スクリプト実行

## 構成

```text
rawhid-host/
├─ crates/
│  ├─ rawhid-host-core/   # 設定、packet、HID、runner、AI usage、ZMK Studio などの中核処理
│  ├─ rawhid-host-cli/    # CLI
│  └─ rawhid-host-tauri/  # Tauri command と監視スレッド
├─ ui/                    # React + TypeScript + Vite UI
├─ docs/                  # 詳細ドキュメント
├─ examples/              # 設定例
├─ create-icons.ps1
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
uplink_interval_ms = 20

[hid]
usage_page = 65376 # 0xFF60
usage = 97         # 0x61
hello_timeout_ms = 200
rescan_interval_sec = 5

[studio]
probe_timeout_ms = 1000
keymap_read_timeout_ms = 8000

[layer_switch]
enabled = true
unmatched_action = "clear_managed"

# レイヤールールはデバイスごとに設定します。
# 設定がないキーボードはレイヤー切り替えの対象外です。
#[layer_switch.devices."uid:7a91c3e4d102ab55"]
#display_name = "Example Keyboard"
#enabled = true
#
#[[layer_switch.devices."uid:7a91c3e4d102ab55".rules]]
#name = "VS Code"
#exe = "Code.exe"
#layer = 3

[time]
enabled = false
format_hint = "time_hm"
clock_mode = "24h"
periodic_sync_sec = 60
# tz_offset_min = 540

[stats]
enabled = true
flush_interval_sec = 60

[actions]
enabled = false

#[actions.devices."uid:7a91c3e4d102ab55"]
#display_name = "Example Keyboard"
#enabled = true
#
#[[actions.devices."uid:7a91c3e4d102ab55".bindings]]
#action_id = 1
#action = "show_window"

[ai_usage]
enabled = false
poll_interval_sec = 300
stale_after_sec = 900

[ai_usage.codex]
enabled = true
# sessions_dir = "C:\\Users\\<user>\\.codex\\sessions"
sessions_auto_detect = true
include_wsl_sessions = true
extra_sessions_paths = []
history_fallback_enabled = true
allow_activity_baseline = false
activity_five_hour_token_baseline = 0
activity_seven_day_token_baseline = 0

[ai_usage.claude_code]
enabled = true
# credentials_path = "C:\\Users\\<user>\\.claude\\.credentials.json"
credentials_auto_detect = true
include_wsl_credentials = true
extra_credentials_paths = []
api_timeout_sec = 10
```

`activity_*_token_baseline` は Codex local history fallback の割合表示に使う仮の分母です。実際の quota limit ではありません。

## GUI 画面

- Dashboard: 監視開始 / 停止、接続状況、現在レイヤー、ログ、AI Usage 簡易サマリ
- Layer Rules: アプリごとのレイヤールール編集。変更は自動保存です。
- Actions: キーボードのキーから PC 側操作を実行する `HOST_ACTION` バインディング設定
- Time Sync: `TIME_SYNC` の有効化、表示形式、同期間隔などの設定
- AI Usage: Codex / Claude Code 使用量送信の設定、状態表示、手動更新
- Keymap Viewer: ZMK Studio キーマップ表示、ヒートマップ、キーテスター、キーマップ編集
- Devices: Raw HID / ZMK Studio の検出結果、capability、バッテリー状態表示
- Settings: 外観、Polling、HID、起動設定

UI は日本語 / 英語の切り替えに対応しています。外観のアクセント色は Settings から変更できます。

## ZMK Studio キーマップ編集

Keymap Viewer の編集モードでは、ZMK Studio RPC を使って実機上のキーマップを staged edit します。変更はキー選択後すぐデバイス上の未保存状態へ反映されますが、永続化するには `保存` が必要です。`変更を破棄` で staged changes を戻せます。

編集できる内容:

- 通常キー、透過、無効
- レイヤー系: `MO` / `TG` / `TO`
- タップホールド系: `MT` / `LT`
- Sticky、Bluetooth、Output、Mouse、Utility、System 系 behavior
- レイヤー追加 / 名前変更 / 削除

制約:

- Studio が locked の場合は編集できません。キーボード側で `&studio_unlock` を実行してください。Host 側から unlock は行いません。
- 編集中は USB serial / CDC ACM port を保持するため、Studio device の再スキャンや別デバイスの読み取りは `port_busy` で拒否されます。
- ZMK Studio transport は Host Link Raw HID transport とは別経路です。
- BLE transport は対象外です。

## AI Usage について

AI Usage は既定で無効です。有効にすると Codex / Claude Code の 5h / 7d 使用率と reset 時刻を `AI_USAGE` packet として送信します。

- Codex は session history 内の `rate_limits` を優先します。取得できた場合は quota source として扱います。
- Codex local history fallback は activity estimate です。quota ではないため reset 時刻は送りません。
- Claude Code は OAuth usage API を experimental / best-effort source として扱います。
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

生成物は `target/` と `ui/dist/` に作られます。これらはリポジトリに含めません。

## 詳細ドキュメント

- [互換性情報](docs/compatibility.md)
- [セットアップガイド](docs/manual-setup.md)
- [アプリ操作マニュアル](docs/manual-app-usage.md)
- [技術スタックと仕組み](docs/technology-overview.md)
- [技術仕様](docs/spec.md)
- [Packet 仕様](docs/packet-spec.md)

---

## English Summary

RawHID Host is a Windows host application for ZMK keyboards. It monitors the foreground application, sends app-layer packets over Raw HID, synchronizes time for keyboard displays, optionally sends Codex / Claude Code usage snapshots, handles keyboard-initiated host actions, and provides ZMK Studio keymap viewing and editing.

The repository contains the host-side app only. Host Link features require compatible ZMK firmware. ZMK Studio keymap features use USB serial / CDC ACM Studio RPC separately from Host Link Raw HID.
