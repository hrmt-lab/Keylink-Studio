# RawHID Host 技術仕様書 / Technical Specification

## 日本語

### スコープ

RawHID Host はホスト側アプリです。ZMK ファームウェア側の実装はこのリポジトリには含めません。

### アーキテクチャ

```text
raw-hid-host/
├── crates/
│   ├── rawhid-host-core/
│   ├── rawhid-host-cli/
│   └── rawhid-host-tauri/
├── ui/
└── docs/
```

`rawhid-host-core` は次を担当します。

- TOML 設定の読み込み
- active app 情報の取得抽象
- app rule matching
- packet encode / decode
- HID device probe / HELLO verification
- runner
- time sync state

`rawhid-host-cli` は core を使う CLI です。`rawhid-host-tauri` は Tauri command と monitor thread を持つ GUI shell です。`ui` は React + TypeScript + Vite です。

### Tauri の状態管理

UI は Tauri `invoke()` で command を呼びます。Tauri から UI へは次の event を送ります。

- `status-update`
- `log-added`

監視は専用 thread で動きます。設定保存時、監視中であれば `MonitorCommand::UpdateConfig` を送り、runner を新しい設定で再構築します。これにより、UI 上の保存操作だけで反映されます。

### 設定

探索順:

1. CLI の `--config <path>`
2. カレントディレクトリの `rawhid-host.toml`
3. OS 標準ユーザー設定ディレクトリ内の `RawHID Host/config.toml`

Tauri debug build はプロジェクトルートの `rawhid-host.toml` を優先します。

主な既定値:

| Field | Default |
| --- | --- |
| `polling.interval_ms` | `500` |
| `hid.usage_page` | `0xFF60` |
| `hid.usage` | `0x61` |
| `hid.hello_timeout_ms` | `200` |
| `layer_switch.enabled` | `true` |
| `time.enabled` | `false` |
| `time.format_hint` | `time_hm` |
| `time.clock_mode` | `24h` |
| `time.periodic_sync_sec` | `60` |

`time.tz_offset_min` は省略可能です。指定する場合は `-1440..=1440` 分の範囲です。

### レイヤー切り替え

各 tick で次を行います。

1. 検証済み HID device を確保します。
2. TIME_SYNC が必要なら送信します。
3. active app を取得します。
4. rule matching を行います。
5. action が前回と同じで device generation も変わっていなければ送信を省略します。
6. `set_layer` または `clear` を送信します。

matching priority:

1. `path`
2. `exe`
3. `title`

同じ優先度では設定順が優先です。前面ウィンドウがない場合は `Unchanged` とし、意図しない `clear` は送りません。

### HID device 管理

1. `hidapi` で HID を列挙します。
2. Usage Page / Usage で候補を絞ります。
3. 候補 device へ `hello` を送ります。
4. `hello_response` の magic / version / type / seq / reserved bytes を検証します。
5. 成功した device だけを verified device として保持します。

write error が出た device は verified list から外し、次回以降に再検出します。

### TIME_SYNC

`time.enabled = true` の場合だけ送信します。

送信条件:

- 初回 tick
- device generation の変化
- 表示に必要な値の変化
- `periodic_sync_sec` による定期補正

`time_hms` でも毎秒送信はしません。ZMK 側は TIME_SYNC 受信時の uptime を保存し、uptime 差分で秒を進めます。

### 公開と配布

GitHub リポジトリにはソース、設定例、ドキュメント、アイコン元画像を含めます。`target/`、`ui/node_modules/`、`ui/dist/`、個人設定、生成済み installer は含めません。

配布物はプロジェクトルートで `.\build-release.ps1` を実行して作成し、GitHub Releases へ添付します。

---

## English

### Scope

RawHID Host is the host-side application. ZMK firmware implementation is not included in this repository.

### Architecture

- `rawhid-host-core`: config, active app abstraction, matching, packets, HID management, runner, time sync
- `rawhid-host-cli`: CLI wrapper around core
- `rawhid-host-tauri`: Tauri commands and monitor thread
- `ui`: React + TypeScript + Vite frontend

### Runtime

The UI calls Tauri commands through `invoke()`. Tauri emits `status-update` and `log-added`.

Monitoring runs in a dedicated thread. Saving settings while monitoring sends `MonitorCommand::UpdateConfig`, rebuilds the runner, and applies the new config without a manual restart.

### Config

Lookup order:

1. CLI `--config <path>`
2. `rawhid-host.toml` in the current working directory
3. `RawHID Host/config.toml` in the OS user config directory

Tauri debug builds prefer project-root `rawhid-host.toml`.

### Layer Switching

Each tick verifies devices, sends due TIME_SYNC, reads the active app, matches rules, and sends `set_layer` or `clear` if the action changed.

Priority is `path`, then `exe`, then `title`. Config order wins within the same priority. No foreground window returns `Unchanged` and does not send `clear`.

### Device Management

HID devices are filtered by Usage Page / Usage. Candidates must pass HELLO before they receive layer or time packets. Devices with write errors are removed and retried later.

### Time Sync

TIME_SYNC is sent only when enabled. It is sent on initial tick, device generation changes, display-relevant value changes, and periodic correction. It is not sent every second.
