# RawHID Host セットアップガイド

## 前提

RawHID Host は Windows 上で動くホストアプリです。ZMK ファームウェア側には、Raw HID で `HL` protocol を受け取る実装が必要です。

## Host 側要件

| Item | Requirement |
| --- | --- |
| OS | Windows 11 64-bit |
| Rust | Windows native Rust toolchain |
| Node.js | UI / Tauri 開発時に必要 |
| WebView2 | Tauri アプリ実行に必要。Windows 11 では通常インストール済み |
| HID transport | Host Link は Windows / `hidapi` から HID device として見える USB 接続または BLE HOG 接続で使用 |

## Keyboard 側要件

- ZMK firmware が動作していること
- Raw HID が有効であること
- HID Usage Page / Usage が host 設定と一致していること
- `HOST_HELLO` / `DEVICE_HELLO` に対応していること
- BLE 接続で Host Link を使う場合は、Windows から BLE HID over GATT device として見えること
- レイヤー切り替えを使う場合は `APP_LAYER set` / `APP_LAYER clear` に対応していること
- 時刻表示を使う場合は `TIME_SYNC` に対応していること
- AI 使用量表示を使う場合は `AI_USAGE` に対応していること
- ヒートマップを使う場合は `KEY_STATS` に対応していること
- キーテスターを使う場合は `KEY_PRESS` に対応していること
- キーマップ表示・編集を使う場合は ZMK Studio USB serial RPC に対応していること

既定の HID 値は Usage Page `0xFF60`、Usage `0x61` です。

## 開発起動

プロジェクトルートで実行します。

```powershell
.\dev.ps1
```

`dev.ps1` は必要に応じて `ui/node_modules` を用意し、Tauri development app を起動します。通常、別ターミナルで `npm run dev` を起動する必要はありません。

## ビルド

| Purpose | Location | Command |
| --- | --- | --- |
| UI only | `ui` | `npm run build` |
| Rust / CLI | project root | `cargo build` |
| Tauri dev app | project root | `.\dev.ps1` |
| Release bundle | project root | `.\build-release.ps1` |

UI のみ:

```powershell
cd ui
npm run build
```

Rust / CLI:

```powershell
cargo build
```

配布用ビルド:

```powershell
.\build-release.ps1
```

生成物は `target/` と `ui/dist/` に作られます。リポジトリには含めません。

## CLI

プロジェクトルートで実行します。

```powershell
cargo run -p rawhid-host-cli -- config-path
cargo run -p rawhid-host-cli -- init-config --output rawhid-host.toml
cargo run -p rawhid-host-cli -- list-devices
cargo run -p rawhid-host-cli -- run
```

subcommand を省略すると `run` と同じ動作になります。

## 初回確認

1. キーボードを接続します。USB Host Link を確認する場合は USB 接続、BLE Host Link を確認する場合は Windows にペアリングして BLE 接続します。
2. `.\dev.ps1` でアプリを起動します。
3. `Devices` 画面で `Scan` を実行します。
4. 対象デバイスが表示され、`HOST_HELLO` / `DEVICE_HELLO` verification が成功することを確認します。Host Link デバイスカードのアイコンで USB / Bluetooth の接続種別を確認できます。
5. `Dashboard` で監視を開始します。

HELLO verification に失敗する場合は、ZMK 側の Raw HID、Usage Page / Usage、reserved bytes zero、`seq` の扱いを確認してください。
BLE 接続で不安定な場合は、`hid.hello_timeout_ms` を確認してください。既定は 750ms です。設定画面で変更した場合、監視中なら設定保存後に runner が再構築され、手動のアプリ再起動は不要です。

## 設定ファイル

開発中や portable 運用では、プロジェクトルートまたは起動ディレクトリに `rawhid-host.toml` を置くと場所が分かりやすくなります。

探索順:

1. CLI の `--config <path>`
2. カレントディレクトリの `rawhid-host.toml`
3. OS 標準ユーザー設定ディレクトリ内の `RawHID Host/config.toml`

Tauri debug build では、プロジェクトルートの `rawhid-host.toml` を優先して読み込みます。

設定例:

```toml
[app]
start_monitoring_on_launch = false

[polling]
interval_ms = 500

[hid]
usage_page = 65376
usage = 97
hello_timeout_ms = 750

[layer_switch]
enabled = true

# レイヤールールはデバイスごとに設定します。
# 設定がないキーボードはレイヤー切り替えの対象外です。
#[layer_switch.devices."uid:7a91c3e4d102ab55"]
#display_name = "Example Keyboard"
#
#[[layer_switch.devices."uid:7a91c3e4d102ab55".rules]]
#name = "Notepad"
#exe = "notepad.exe"
#layer = 1

[time]
enabled = false
format_hint = "time_hm"
clock_mode = "24h"
periodic_sync_sec = 60

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
api_timeout_sec = 10
```

`sessions_dir` と `credentials_path` を空欄にすると、Core default を使います。UI では `Default` / `Core default` として表示されます。

## AI Usage 利用時の注意

- AI Usage は既定で無効です。
- Codex は `rate_limits` を優先し、取れない場合のみ local history fallback を使います。
- local history fallback は activity estimate であり、実 quota ではありません。
- `activity_*_token_baseline` は fallback 用の仮分母です。実 quota limit ではありません。
- Claude Code OAuth usage API は experimental / best-effort source です。
- `.credentials.json` は環境によって存在しないことがあります。
- access token、credentials JSON、API response を UI や log に出さない方針です。

## GitHub 公開時に除外するもの

少なくとも次を除外してください。

```gitignore
/target/
/ui/node_modules/
/ui/dist/

rawhid-host.toml
*.log
*.tmp
*.pdb

.DS_Store
Thumbs.db
```

`rawhid-host.toml` は個人設定になりやすいため、公開する場合は `examples/` にサンプルを置く運用を推奨します。

## 実装上の注記

- `[app] start_monitoring_on_launch = true` はアプリ起動時に監視を自動開始します。設定の「ログイン時に起動」トグル (HKCU Run レジストリキー) と組み合わせると、Windows ログイン後に自動で監視を開始できます。
- アプリはシングルインスタンスです。2 つ目の起動では既存ウィンドウが前面化されます。
- Host Link は USB HID と BLE HOG の両方を同じ `HL` packet protocol として扱います。`hidapi` が接続種別を返せる場合はその値を使い、返せない場合は Windows HID path から USB / Bluetooth を補助判定します。
- `hid.rescan_interval_sec` を設定すると、監視中の Host Link HID 再スキャン間隔を変更できます。既定は 5 秒です。
- AI Usage の収集は監視とは独立して動きます。Raw HID 送信には監視が必要です。
- Codex sessions 自動検出 (`sessions_auto_detect`、既定オン) は Windows デフォルト・各 WSL ディストロの `~/.codex/sessions` (`include_wsl_sessions`)・`extra_sessions_paths` を読み込み、WSL 上の Codex 使用分もまとめて反映します。`rate_limits` は全ディレクトリ中の最新値を使用し、history fallback はトークンを全ディレクトリ合算します。
- Claude Code credentials 自動検出は Windows デフォルト・WSL デフォルト・追加 credentials パスに対応しています。refresh token 更新は v1 では行いません。
- ZMK Studio Keymap Viewer は ZMK Studio USB serial RPC transport を必要とします。ZMK Studio の BLE transport は v1 の対象外です。編集には実機側で `&studio_unlock` を実行して Studio を unlocked にする必要があります。Host 側から unlock は行いません。
- デバイス単位の App Layer ルールを使う場合は、firmware が `DEVICE_HELLO` で `APP_LAYER` capability と安定した非ゼロの `device_uid_hash` を返す必要があります。
