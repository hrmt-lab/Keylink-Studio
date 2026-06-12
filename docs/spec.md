# RawHID Host 技術仕様

## Scope

RawHID Host は host 側アプリです。ZMK firmware 側の実装はこのリポジトリには含みません。

## Architecture

```text
rawhid-host/
├─ crates/
│  ├─ rawhid-host-core/
│  ├─ rawhid-host-cli/
│  └─ rawhid-host-tauri/
├─ ui/
└─ docs/
```

| Component | Role |
| --- | --- |
| `rawhid-host-core` | config、active app、rule matching、packet、HID、runner、time sync、AI usage |
| `rawhid-host-cli` | core を使う CLI |
| `rawhid-host-tauri` | Tauri command、monitor thread、UI への event 発行 |
| `ui` | React + TypeScript + Vite frontend |

## Runtime

UI は Tauri `invoke()` で command を呼びます。Tauri から UI へは主に次の event を送ります。

- `status-update`
- `log-added`

監視処理は専用 thread で動きます。設定保存時、監視中であれば `MonitorCommand::UpdateConfig` を送り、runner を新しい設定で再構築します。ユーザーが手動再起動する必要はありません。

監視 thread は起動時に `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` ベースの foreground watcher を起動します。前面アプリが切り替わると `MonitorCommand::ForegroundChanged` が即時送られ、polling 間隔を待たずに tick が実行されます。watcher が使えない環境でも `recv_timeout` による polling で動作は継続します。

アプリはシングルインスタンスです。2 つ目の instance を起動すると、既存ウィンドウが前面化されます。`[app] start_monitoring_on_launch = true` の場合、アプリ起動時に監視を自動開始します。

AI usage collection は background worker が行います。`Runner::tick()` は外部 API 呼び出しや大量 JSONL scan を行わず、共有 snapshot の差分送信だけを担当します。

## Config

探索順:

1. CLI の `--config <path>`
2. カレントディレクトリの `rawhid-host.toml`
3. OS 標準ユーザー設定ディレクトリ内の `RawHID Host/config.toml`

Tauri debug build では、プロジェクトルートの `rawhid-host.toml` を優先して読み込みます。

主な default:

| Field | Default |
| --- | ---: |
| `app.start_monitoring_on_launch` | `false` |
| `polling.interval_ms` | `500` |
| `hid.usage_page` | `0xFF60` |
| `hid.usage` | `0x61` |
| `hid.hello_timeout_ms` | `200` |
| `layer_switch.enabled` | `true` |
| `time.enabled` | `false` |
| `time.format_hint` | `time_hm` |
| `time.clock_mode` | `24h` |
| `time.periodic_sync_sec` | `60` |
| `ai_usage.enabled` | `false` |
| `ai_usage.poll_interval_sec` | `300` |
| `ai_usage.stale_after_sec` | `900` |
| `ai_usage.codex.enabled` | `true` |
| `ai_usage.codex.sessions_auto_detect` | `true` |
| `ai_usage.codex.include_wsl_sessions` | `true` |
| `ai_usage.claude_code.enabled` | `true` |
| `ai_usage.claude_code.api_timeout_sec` | `10` |
| `stats.enabled` | `true` |
| `stats.flush_interval_sec` | `60` |
| `actions.enabled` | `false` |

`time.tz_offset_min` は省略可能です。指定する場合は `-1440..=1440` 分の範囲です。

## Layer Switching

レイヤールールはデバイス単位 (`layer_switch.devices."uid:..."`) でのみ設定します。グローバルな共通ルールはありません。デバイス専用設定を持たないデバイスはレイヤー切り替えの対象外で、`APP_LAYER` packet を送信しません。

各 tick で次を行います。

1. verified HID device を確保します。
2. device-initiated (uplink) packet を非ブロッキングでドレインします。
3. 必要なら `TIME_SYNC` を送信します。
4. 更新された `AI_USAGE` snapshot があれば provider ごとに送信します。
5. active app を取得します。
6. device ごとに、その device の専用 rules で rule matching を行います。
7. action が前回と同じで device generation も変わっていなければ送信を省略します。
8. `APP_LAYER set` または `APP_LAYER clear` を送信します。

matching priority:

1. `path`
2. `exe`
3. `title`

同じ優先度では設定順が優先されます。前面ウィンドウがない場合は `Unchanged` とし、意図しない `clear` は送りません。

## HID Device Management

1. `hidapi` で HID を列挙します。
2. Usage Page / Usage で候補を絞ります。
3. 候補 device へ `HOST_HELLO` を送ります。
4. 同じ `seq` の `DEVICE_HELLO` が返ることと、magic / version / type / reserved bytes を検証します。
5. 成功した device だけを verified device として保持します。

write error が出た device は verified list から外し、次回以降に再検出します。

## Uplink (device → host)

キーボード起点の packet (`BATTERY_STATUS` / `HOST_ACTION` / `KEY_STATS` / `LAYER_STATE`) を tick ごとに非ブロッキング読みでドレインします。

- 各 type は対応する capability bit を `DEVICE_HELLO` で立てた device からのみ受け付けます。bit なしは破棄します。
- HELLO 検証中に届いた uplink packet は読み飛ばさず保持し、検証後に通常経路へ流します。
- read error が出た device は write error と同様に verified list から外します。
- uplink は best-effort です。監視停止中や読み取り間隔中のバースト超過分は失われます (`KEY_STATS` はアンダーカウント許容、seq gap を警告ログ)。
- `HOST_ACTION` の実行は config `[actions]` の許可リスト制 (既定 disabled)。バインディングは device 単位 (`actions.devices."uid:..."`、`layer_switch.devices` と同じキー) で、未定義 id・未設定 device はログのみ。`value` byte を path やコマンドとして解釈しません。同一 seq の連続受信は 1 回として扱います。
- `LAYER_STATE` は表示専用です。runner の managed layer 状態には影響せず、`APP_LAYER` としてエコーバックしません。
- `KEY_STATS` は `[stats]` 有効時に日別バケットでローカルファイル (`<data_dir>/stats/uid_*.json`) へ永続化します。書き込みは `flush_interval_sec` 間隔 + 監視停止時です。
- 応答性: 受信は tick 駆動のため最大 `polling.interval_ms` 遅延します。即時化 (専用リーダースレッド) は将来課題です。

## TIME_SYNC

`time.enabled = true` の場合だけ送信します。

送信条件:

- 初回 tick
- device generation の変化
- 表示に必要な値の変化
- `periodic_sync_sec` による定期補正

`time_hms` でも毎秒送信はしません。ZMK 側は `TIME_SYNC` 受信時の uptime を保存し、uptime 差分で秒を進めます。

## AI Usage

AI Usage は任意機能で、既定では無効です。

### Worker

- monitoring start で AI usage worker を起動します。
- config update では worker を停止して新 config で再起動します。
- stop / loop 終了時は worker を停止します。
- `refresh_ai_usage` command は worker に即時更新を依頼し、取得完了までは待ちません。
- refresh request 中は UI button を disabled にし、Tauri / worker 側でも多重実行を防ぎます。
- refresh 完了は watcher thread が snapshot generation の変化で検知し、監視停止中でも `status-update` event を発行します。UI 側での状態差分の推測は行いません。
- `now - snapshot.updated_unix >= stale_after_sec` の場合は `stale=1` とします。
- 取得失敗時、前回成功値があれば valid を維持して `stale=1` と error code を立てます。
- 前回成功値がなければ valid を立てず、error code だけを返します。

### Codex Provider

- `sessions_dir` に加え、`sessions_auto_detect` が有効な場合は Windows default・各 WSL ディストロの `~/.codex/sessions` (`include_wsl_sessions`)・`extra_sessions_paths` を読み込み対象に含めます。WSL 上の Codex CLI 使用分もここで合算されます。
- 対象ディレクトリ全体の `.codex/sessions/**/*.jsonl` を mtime 降順で探索します。
- 同じ mtime の場合は path 文字列昇順で安定ソートします。
- `rate_limits` は新しい file から順に、file 内では末尾行から先頭行へ探します。
- 最初に見つかった parse 可能な `rate_limits` を採用します。
- `window_minutes = 300` を 5h、`10080` を 7d として扱います。
- `rate_limits.used_percent` と `resets_at` が取れた window は quota source として送ります。
- `rate_limits` が取れない場合だけ、設定により local history fallback を使います。
- local history fallback は activity estimate です。quota ではありません。
- fallback 時は `estimated=1`, `local_history_source=1`, `quota_source=0`, `reset_unix=0` です。
- `token_count` がない古いログは `no_usage_data` として扱います。
- `last_token_usage` があればそれを優先します。
- `last_token_usage` がない場合は、同一 session 内で `total_token_usage` の正の差分だけを加算します。
- duplicate `token_count` や unchanged `total_token_usage` は二重計上しません。

`activity_five_hour_token_baseline` と `activity_seven_day_token_baseline` は fallback の割合表示用の仮分母です。実 quota limit ではありません。

### Claude Code Provider

- `credentials_path` で指定された単一ファイルだけを読み取り専用で読みます。
- default は `%USERPROFILE%\\.claude\\.credentials.json` です。
- ディレクトリ再帰探索はしません。
- credentials file 内容を別ファイルへコピー保存しません。
- `claudeAiOauth.accessToken` が取れた場合、OAuth usage API を呼びます。
- Claude OAuth usage API は experimental / best-effort source として扱います。
- schema 変更、HTTP error、token 期限切れ、missing credentials は fixed error code に変換します。
- refresh token 更新は v1 では行いません。

Security policy:

- access token、credentials JSON、Authorization header、HTTP request body、HTTP response body、raw parse error を log / UI / status / Raw HID packet に出しません。
- `reqwest` error や request / response 構造体を Debug 出力しません。
- provider error は enum/code に変換して扱います。
- UI 表示は sanitize 済み固定文言のみ使います。

## Tauri Commands

主な command:

- `get_config`
- `save_config`
- `reload_config`
- `get_config_path`
- `show_config_file_location`
- `get_status`
- `get_log_entries`
- `get_running_apps`
- `probe_devices`
- `start_monitoring`
- `stop_monitoring`
- `refresh_ai_usage`
- `get_app_icons`
- `get_launch_at_login`
- `set_launch_at_login`
- `get_key_stats`
- `list_key_stats_devices`
- `debug_inject_uplink` (debug build のみ動作)

`show_config_file_location` は config path だけを Explorer で表示します。credentials path の reveal は行いません。

`get_app_icons` は exe path のリストを受け取り、Windows Shell からアイコンを抽出して PNG data URL の map を返します。抽出できない exe は結果に含めません。

`get_launch_at_login` / `set_launch_at_login` は `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run` の `RawHID Host` 値を読み書きします。管理者権限は不要です。

## Public Artifacts

GitHub には source、docs、examples、icon source、Tauri icons を含めます。`target/`、`ui/node_modules/`、`ui/dist/`、個人用 `rawhid-host.toml`、生成済み installer は含めません。

## Current implementation notes

- Host Link packet types are `0x01 HOST_HELLO`, `0x02 DEVICE_HELLO`, `0x03 ERROR`, `0x04 PING`, `0x05 PONG`, `0x10 AI_USAGE`, `0x20 TIME_SYNC`, and `0x30 APP_LAYER`.
- `DEVICE_HELLO` capabilities gate sending: `APP_LAYER` packets are sent only to devices that advertise `APP_LAYER`.
- `device_uid_hash = 0` is normalized to None. Internal device settings do not create `Some(0)`.
- AI Usage providers update snapshots in a background worker. `Runner::tick()` sends latest snapshots but does not perform provider fetches.
- Codex uses `rate_limits` as quota source when available. Local history is fallback/activity estimate only.
- Claude OAuth usage API is experimental/best-effort. Credentials auto-detect can try explicit path, Windows default, WSL default, and extra paths.
- Keymap Viewer uses a separate ZMK Studio RPC client module and is read-only in v1.
- Codex JSONL session files are read with a 4 MB tail cap to bound memory usage. The first (possibly partial) line of the tail window is dropped.
- Uplink packets (`0x40`-`0x70`) are drained non-blocking each tick and gated by `DEVICE_HELLO` capability bits. `LAYER_STATE` is display-only and never feeds the rule engine.
- Key statistics are persisted per device uid as daily buckets in `<data_dir>/stats/`; positions only, never key contents.
- `HOST_ACTION` execution is allowlist-based (`[actions]`, default disabled) with per-device bindings (`actions.devices."uid:..."`, keyed like `layer_switch.devices`); the HID value byte is never interpreted as a path or command.
- Device-specific `unmatched_action` overrides the global `layer_switch.unmatched_action` when set.
- Foreground watcher (`foreground.rs`), exe icon extraction (`icon.rs`), and launch-at-login registry handling (`startup.rs`) are Windows-only modules with no-op stubs on other platforms.
