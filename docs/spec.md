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
| `ai_usage.claude_code.enabled` | `true` |
| `ai_usage.claude_code.api_timeout_sec` | `10` |

`time.tz_offset_min` は省略可能です。指定する場合は `-1440..=1440` 分の範囲です。

## Layer Switching

各 tick で次を行います。

1. verified HID device を確保します。
2. 必要なら `TIME_SYNC` を送信します。
3. 更新された `AI_USAGE` snapshot があれば provider ごとに送信します。
4. active app を取得します。
5. rule matching を行います。
6. action が前回と同じで device generation も変わっていなければ送信を省略します。
7. `APP_LAYER set` または `APP_LAYER clear` を送信します。

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
- UI の「更新を要求しました。」メッセージは、provider status / updated time / error などの AI usage 状態が変わった時点で消します。
- `now - snapshot.updated_unix >= stale_after_sec` の場合は `stale=1` とします。
- 取得失敗時、前回成功値があれば valid を維持して `stale=1` と error code を立てます。
- 前回成功値がなければ valid を立てず、error code だけを返します。

### Codex Provider

- `.codex/sessions/**/*.jsonl` を mtime 降順で探索します。
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

`show_config_file_location` は config path だけを Explorer で表示します。credentials path の reveal は行いません。

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
