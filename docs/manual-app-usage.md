# RawHID Host アプリ操作マニュアル

このドキュメントは、GUI で何ができるかを画面ごとに説明します。設定は `rawhid-host.toml` に保存され、監視中に保存した変更は runner に反映されます。

## 共通

- 左の Sidebar から画面を切り替えます。
- Sidebar 下部の `JP / EN` で日本語 / 英語表示を切り替えられます。
- 監視中は Sidebar に接続デバイス数が表示されます。
- `保存` ボタンがあるページは、押した時点で設定ファイルへ保存します。
- Layer Rules は自動保存です。ルール追加、削除、変更の操作時に保存されます。

## Dashboard

Dashboard は監視状態を確認し、監視を開始 / 停止する画面です。

- `Start Monitoring` / `Stop Monitoring` で監視を切り替えます。
- 接続済み Raw HID デバイス数を表示します。
- 現在適用中の layer と rule を表示します。
- 直近ログを表示します。
- AI Usage が有効な場合は Codex / Claude Code の簡易サマリを表示します。
- AI Usage 全体、Codex、Claude Code の有効 / 無効を切り替えられます。

Dashboard から詳細ページへの導線は置いていません。他の機能と同様に、詳細設定は Sidebar から開きます。

## Layer Rules

前面アプリと ZMK layer の対応を設定します。

GUI で追加するルールは `exe` 条件を使います。`path` や `title` を使いたい場合は `rawhid-host.toml` を直接編集してください。

matching priority:

| Priority | Condition | Description |
| ---: | --- | --- |
| 1 | `path` | 実行ファイルのフルパス。大文字小文字は区別しません。 |
| 2 | `exe` | 実行ファイル名。大文字小文字は区別しません。 |
| 3 | `title` | ウィンドウタイトルの部分一致。大文字小文字は区別しません。 |

同じ優先度で複数一致した場合は、設定ファイル上の順番が優先されます。どのルールにも一致しない場合は `APP_LAYER clear` を送信します。

Windows では Desktop、Taskbar、File Explorer が `explorer.exe` として見えることがあります。意図しない layer 切り替えを避けたい場合、通常は `explorer.exe` ルールを作らない方が扱いやすいです。

## Time Sync

キーボードの表示用に時刻情報を送る設定です。

| UI | Config | Description |
| --- | --- | --- |
| Enabled | `time.enabled` | `TIME_SYNC` を送るかどうか |
| Display Format | `time.format_hint` | 表示形式のヒント |
| Clock Mode | `time.clock_mode` | `24h` または `12h` |
| 同期間隔 / Sync interval | `time.periodic_sync_sec` | 定期補正間隔。`0` で無効 |
| Timezone Offset | `time.tz_offset_min` | 空欄なら OS の現在 offset。指定時は分単位 |

`time_hms` でも毎秒 packet を送るわけではありません。ZMK 側は `TIME_SYNC` 受信時の uptime を保存し、uptime 差分で秒を進める想定です。

## AI Usage

Codex / Claude Code の使用率を Raw HID で送る設定です。既定では無効です。

### Dashboard

Dashboard では次の操作だけを行えます。

- AI Usage 全体の有効 / 無効
- Codex の有効 / 無効
- Claude Code の有効 / 無効
- provider ごとの簡易サマリ表示

### AI Usage 詳細ページ

詳細ページでは、使用率バー、状態、基本設定、Advanced 設定を確認 / 編集できます。

基本設定:

| UI | Config | Description |
| --- | --- | --- |
| AI Usage | `ai_usage.enabled` | AI usage の取得と送信を有効化 |
| Poll interval | `ai_usage.poll_interval_sec` | provider を取得する間隔 |
| Stale threshold | `ai_usage.stale_after_sec` | snapshot を stale とみなす秒数 |
| Codex enabled | `ai_usage.codex.enabled` | Codex provider の有効化 |
| Claude Code enabled | `ai_usage.claude_code.enabled` | Claude Code provider の有効化 |

Advanced:

| UI | Config | Description |
| --- | --- | --- |
| Codex sessions dir | `ai_usage.codex.sessions_dir` | 空欄なら Core default。通常は `%USERPROFILE%\\.codex\\sessions` |
| History fallback | `ai_usage.codex.history_fallback_enabled` | `rate_limits` がない場合に local history を読む |
| Allow activity baseline | `ai_usage.codex.allow_activity_baseline` | fallback を割合表示するための baseline 使用を許可 |
| 5h activity baseline | `activity_five_hour_token_baseline` | fallback 用の仮分母。実 quota limit ではない |
| 7d activity baseline | `activity_seven_day_token_baseline` | fallback 用の仮分母。実 quota limit ではない |
| Claude credentials path | `ai_usage.claude_code.credentials_path` | 空欄なら Core default。通常は `%USERPROFILE%\\.claude\\.credentials.json` |
| API timeout | `ai_usage.claude_code.api_timeout_sec` | Claude OAuth usage API の timeout |

Codex は session history の `rate_limits` を優先します。取れた場合は quota source です。local history fallback は activity estimate であり、実 quota ではありません。そのため fallback 時は reset 時刻を表示しません。

Claude Code OAuth usage API は experimental / best-effort source です。`.credentials.json` が存在しない環境や、schema 変更、認証失敗、token 期限切れがあり得ます。UI には access token、credentials JSON、API response、raw error は表示しません。

使用率バーは `used` です。80% 以上でオレンジ、90% 以上で赤になります。値が invalid の場合は `no data` と表示します。

`更新` ボタンは監視中のみ使えます。押すと worker に更新要求を送り、取得処理は背景で行われます。要求直後に表示される「更新を要求しました。」は、AI 使用量の status / updated time / error などが更新されたら自動で消えます。worker 側では多重取得を防ぎます。

## Devices

Raw HID 候補を列挙し、HELLO 検証を行います。

- Usage Page / Usage が設定値と一致する HID を候補にします。
- host は候補へ `HOST_HELLO` を送ります。
- 同じ `seq` の `DEVICE_HELLO` が返った device だけが verified になります。
- verified device だけが監視時の送信対象になります。

HELLO 検証に失敗する場合は、ZMK 側の Raw HID、Usage Page / Usage、reserved bytes zero、`seq` の返し方を確認してください。

## Settings

Polling と HID の基本設定を編集します。

| UI | Config | Default |
| --- | --- | ---: |
| Poll interval | `polling.interval_ms` | `500` ms |
| Usage Page | `hid.usage_page` | `0xFF60` |
| Usage | `hid.usage` | `0x61` |
| HELLO timeout | `hid.hello_timeout_ms` | `200` ms |

`更新` は設定ファイルを再読み込みします。`保存` は現在の設定をファイルに書き込みます。

## System tray

ウィンドウを閉じてもアプリは完全終了せず、システムトレイに残ります。完全に終了する場合はトレイメニューの `Quit` を使います。
