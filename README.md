# RawHID Host

RawHID Host は、Windows 上で動く ZMK キーボード向けのホストアプリです。
前面アプリに応じたレイヤー切り替え、時刻同期、AI 使用量送信、キーボードから PC へのアクション実行、ZMK Studio RPC 経由のキーマップ閲覧・編集を扱います。

このリポジトリに含まれるのは **PC 側のホストアプリのみ** です。Host Link 機能を使うには、ZMK firmware 側に `HL` packet protocol を受け取る実装が必要です。Host Link は Windows / `hidapi` から HID device として見える USB 接続または BLE HOG 接続を扱います。ZMK Studio キーマップ機能は Host Link とは別経路の Studio RPC transport を使い、USB serial / CDC ACM と BLE Studio の読み取り・編集に対応します。

## 主な機能

- Windows の前面アプリ / プロセス監視
- `path` / `exe` / `title` によるアプリ判定
- デバイス単位の ZMK レイヤールール
- Host Link HID デバイス検出と `HOST_HELLO` / `DEVICE_HELLO` 検証
  - USB / Bluetooth 接続種別の表示
  - 同じ `device_uid_hash` の USB / BLE endpoint 集約表示
- `APP_LAYER` / `TIME_SYNC` / `AI_USAGE` packet 送信
- キーボードからの uplink 受信
  - バッテリー残量
  - PC 操作トリガー (`HOST_ACTION`)
  - タイピング統計 (`KEY_STATS`)
  - レイヤー逆同期 (`LAYER_STATE`)
  - キーテスター用リアルタイム押下イベント (`KEY_PRESS`)
- ZMK Studio キーマップ表示・編集
  - USB serial / CDC ACM と BLE Studio transport
  - 通常キー、透過、無効
  - `MO` / `TG` / `TO`
  - `MT` / `LT`
  - Sticky、Bluetooth、Output、Mouse、Utility、System 系 behavior
  - レイヤー追加 / 名前変更 / 削除
  - 保存 / 破棄による staged edit
  - `-keymap.json` へのバックアップ書き出し / 共通するキー位置への復元
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
hello_timeout_ms = 750
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

- Dashboard: 監視開始 / 停止、接続状況、現在レイヤー、ログ、AI Usage 簡易サマリ。Host Link デバイスは `device_uid_hash` 単位で集約し、USB / Bluetooth の接続経路アイコンを表示
- Layer Rules: アプリごとのレイヤールール編集。変更は自動保存です。
- Actions: キーボードのキーから PC 側操作を実行する `HOST_ACTION` バインディング設定
- Time Sync: `TIME_SYNC` の有効化、表示形式、同期間隔などの設定
- AI Usage: Codex / Claude Code 使用量送信の設定、状態表示、手動更新
- Keymap Viewer: ZMK Studio キーマップ表示、ヒートマップ、キーテスター、キーマップ編集。統計とリアルタイム押下イベントは Studio の `serial_number` が Host Link の `device_uid_hash` と同じ 16 桁 hex UID を返す場合、UID 優先で紐付け
- Devices: Host Link / ZMK Studio の検出結果、USB / Bluetooth 接続種別、capability、バッテリー状態表示。Host Link は `device_uid_hash` 単位で集約し、同じキーボードが USB と BLE HOG の両方で見えている場合は 1 カードにまとめて両方のアイコンを表示
- Settings: 外観、Polling、HID、起動設定

UI は日本語 / 英語の切り替えに対応しています。外観のアクセント色は Settings から変更できます。

## ZMK Studio キーマップ編集

Keymap Viewer の編集モードでは、ZMK Studio RPC を使って実機上のキーマップを staged edit します。変更はキー選択後すぐデバイス上の未保存状態へ反映されますが、永続化するには `保存` が必要です。`変更を破棄` で staged changes を戻せます。

未保存変更がある状態で他の画面へ移動しようとすると、移動前に確認ダイアログを表示します。`保存して移動` は Studio の変更を保存してから編集セッションを閉じ、`破棄して移動` は未保存変更を破棄してから移動します。`キャンセル` はキーマップ編集画面に残ります。キー書き込み待ちや保存 / 破棄 / 終了処理中は、処理が終わるまで保存して移動 / 破棄して移動は選べません。

ZMK Studio で保存されるキーマップは firmware の `.keymap` ソースではなく、デバイスの settings / NVS 側の状態です。firmware をフルイレース、または settings reset 付きで焼き直すと Studio で編集したキーマップは戻ることがあります。

Keymap Viewer の `Export` は、現在デバイス上にある ZMK Studio/NVS 状態を `-keymap.json` として書き出します。`Restore` は現在キーボードにも存在する layer index と key position の raw binding だけを未保存変更として読み込みます。backup にしかない layer / position は書き込まず、現在キーボードにしかない layer / position は変更しません。復元対象の差分がない場合は、その旨を画面上に表示し、未保存変更は作りません。復元直後はまだ永続化されていないため、実機へ保存するには既存の `保存` を押してください。取り消す場合は `変更を破棄` を使います。

このバックアップは運用復旧用であり、`.keymap` 生成や firmware ソースへの反映は行いません。レイヤー名、レイヤー数、レイヤー順、物理レイアウト選択は復元対象外です。behavior 名検証ができない接続では強警告を出し、同一 firmware / 近い構成への復元を前提に raw binding を復元します。BLE Studio 由来のバックアップも復元対象ですが、検証できない場合は USB より安全確認が弱くなります。復元または手動編集で変更したキーは、編集セッション中に色付きで表示されます。

編集できる内容:

- 通常キー、透過、無効
- レイヤー系: `MO` / `TG` / `TO`
- タップホールド系: `MT` / `LT`
- Sticky、Bluetooth、Output、Mouse、Utility、System 系 behavior
- レイヤー追加 / 名前変更 / 削除

キー割り当てポップオーバーのキー候補は、修飾子（任意）のトグル行の下に、英字、数字、修飾キー、コントロール・スペース、記号、ナビゲーションの順で表示します。その他のカテゴリはナビゲーションの下に従来の相対順で表示します。

制約:

- Studio が locked の場合は編集できません。キーボード側で `&studio_unlock` を実行してください。Host 側から unlock は行いません。
- 編集中は Studio RPC session を保持するため、Studio device の再スキャンや別デバイスの読み取りは `port_busy` で拒否されます。
- ZMK Studio transport は Host Link HID transport とは別経路です。
- BLE Studio transport でも編集できますが、USB より応答待ちが長くなります。キー変更中は下部バーに `書き込み中 N件` が表示され、保存 / 破棄 / 編集終了 / レイヤー操作は一時的に無効になります。
- BLE 編集中に切断や timeout が起きた場合は、下部バーの `再読み込み` で壊れた編集セッションを破棄してから実機状態を読み直します。未保存変更は失われるため、再接続後に再確認してください。

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

RawHID Host is a Windows host application for ZMK keyboards. It monitors the foreground application, sends app-layer packets over Host Link HID, synchronizes time for keyboard displays, optionally sends Codex / Claude Code usage snapshots, handles keyboard-initiated host actions, and provides ZMK Studio keymap viewing and editing.

The repository contains the host-side app only. Host Link features require compatible ZMK firmware and can use USB HID or BLE HOG when exposed through Windows HID APIs. ZMK Studio keymap features use Studio RPC separately from Host Link HID, with USB serial / CDC ACM and BLE Studio read/edit support.
