# Keylink Studio 技術仕様

## Scope

Keylink Studio は host 側アプリです。ZMK firmware 側の実装はこのリポジトリには含みません。

## Architecture

```text
Keylink-Studio/
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

Host Link はアプリ起動時から終了時まで単一の専用 worker thread が所有します。`start_monitoring` / `stop_monitoring` は worker を生成・終了せず、自動レイヤー切替、ホストアクション、統計収集などの自動処理だけを切り替えます。監視停止中も機器検出とキーマップ画面の Config RPC は利用できます。設定保存時は `MonitorCommand::UpdateConfig` を送り、HID manager と seq を維持したまま runner の設定を更新します。

Host Link worker は起動時に `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` ベースの foreground watcher を起動します。前面アプリが切り替わると `MonitorCommand::ForegroundChanged` が送られ、監視中は polling 間隔を待たずに tick が実行されます。監視停止中は通知を消費しますが自動処理を実行しません。

アプリはシングルインスタンスです。2 つ目の instance を起動すると、既存ウィンドウが前面化されます。`[app] start_monitoring_on_launch = true` の場合、アプリ起動時に監視を自動開始します。

AI usage collection は background worker が行います。`Runner::tick()` は外部 API 呼び出しや大量 JSONL scan を行わず、共有 snapshot の差分送信だけを担当します。

## Config

探索順:

1. CLI の `--config <path>`
2. カレントディレクトリの `keylink-studio.toml`
3. OS 標準ユーザー設定ディレクトリ内の `Keylink Studio/config.toml`

Tauri debug build では、プロジェクトルートの `keylink-studio.toml` を優先して読み込みます。

主な default:

```toml
[debug_log]
enabled = false
path = "keylink-studio-debug.log"
```

`[debug_log]` はデバッグ用のファイルログ設定です。既定では無効です。有効時、`path` が相対 path の場合は実行中の `.exe` と同じディレクトリからの相対 path として解決します。`path` が絶対 path の場合はその path へ出力します。`path` が未設定または空の場合は `.exe` と同じディレクトリの `keylink-studio-debug.log` を使います。Settings から有効 / 無効の切り替えと出力先ファイル選択を行えます。出力先を開けない場合は UI にエラーを表示し、ファイルログは無効化します。

ファイルログは原因切り分け用で、既存の in-memory log / UI log とは別の sink として扱います。access token、credentials JSON、Authorization header、HTTP request / response body、raw parse error、ユーザーが読み込んだ keymap backup JSON の本文は出力しません。Host Link / Config RPC のログは raw packet 全体ではなく、packet type、feature、op、status、seq、対象 id、timeout / retry / disconnect などの summary を出力します。

| Field | Default |
| --- | ---: |
| `app.start_monitoring_on_launch` | `false` |
| `polling.interval_ms` | `500` |
| `hid.usage_page` | `0xFF60` |
| `hid.usage` | `0x61` |
| `hid.hello_timeout_ms` | `750` |
| `debug_log.enabled` | `false` |
| `debug_log.path` | `"keylink-studio-debug.log"` |
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

Host Link は USB HID と BLE HOG のどちらも同じ HID candidate として扱います。`DeviceInfo.connection_type` は `hidapi::DeviceInfo::bus_type()` を一次情報として `usb` / `bluetooth` / `unknown` に分類します。`bus_type` が `Unknown` の場合のみ、Windows HID path に含まれる BLE HID Service UUID (`00001812-0000-1000-8000-00805f9b34fb`) や USB HID path 形式を補助判定に使います。この接続種別は UI 表示用で、runner の layer switch / time sync / uplink 処理は従来どおり capability と `device_uid_hash` に基づきます。

UI では、Devices 画面の Host Link 一覧を `device_uid_hash` 単位のキーボード表示として扱います。同じ `device_uid_hash` の USB / BLE HOG endpoint が同時に見えている場合は 1 台に集約し、接続中の transport アイコンを並べて表示します。集約カードの詳細には各 endpoint の HID path を表示します。`device_uid_hash` が取得できない endpoint は誤結合を避けるため path 単位で個別に扱います。

BLE HOG では scan ごとの `HOST_HELLO` 応答が一時的に揺れることがあるため、既に verified 済みで、かつ candidate としては見えている device は、連続 2 回までの HELLO miss では verified list に残します。candidate から消えた device、または write/read error が出た device は verified list から外し、次回以降に再検出します。

## Uplink (device → host)

キーボード起点の packet (`BATTERY_STATUS` / `HOST_ACTION` / `KEY_STATS` / `LAYER_STATE` / `KEY_PRESS`) を tick ごとに非ブロッキング読みでドレインします。

- 各 type は対応する capability bit を `DEVICE_HELLO` で立てた device からのみ受け付けます。bit なしは破棄します。
- HELLO 検証中に届いた uplink packet は読み飛ばさず保持し、検証後に通常経路へ流します。
- read error が出た device は write error と同様に verified list から外します。
- uplink は best-effort です。Config RPC 応答待機中に受信した uplink は一時キューへ退避し、監視中は応答処理後に通常経路へ戻します。監視停止中のuplinkは読み取って破棄し、後からホストアクションなどを実行しません。読み取り上限を超えるバーストは失われる可能性があります。
- `HOST_ACTION` の実行は config `[actions]` の許可リスト制 (既定 disabled)。バインディングは device 単位 (`actions.devices."uid:..."`、`layer_switch.devices` と同じキー) で、未定義 id・未設定 device はログのみ。`value` byte を path やコマンドとして解釈しません。同一 seq の連続受信は 1 回として扱います。
- `LAYER_STATE` は表示専用です。runner の managed layer 状態には影響せず、`APP_LAYER` としてエコーバックしません。
- `KEY_STATS` は `[stats]` 有効時に日別バケットでローカルファイル (`<data_dir>/stats/uid_*.json`) へ永続化します。書き込みは `flush_interval_sec` 間隔 + 監視停止時です。
- `KEY_PRESS` はキーテスターのリアルタイム表示専用です。累積カウントは保持せず、`KEY_STATS` とは独立しています。
- UI のキーテスターは同じ position の pressed event を、released event が来るまで重複表示しません。デバイス切り替えやリセット時は pressed / tested / 直近イベント表示をすべてクリアします。
- `BATTERY_STATUS` の `source=0` は central/self として `C`、`source=1..=3` は peripheral として `P1..P3` で表示します。`level = 0xFF` は unknown / not available / disconnected として扱い、通常表示ではその source を隠します。表示可能な source がない場合のみ、UI では `--`、tray では `?` と表示します。`0` は有効な 0% として扱います。
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

- アプリ初期化時に AI usage worker を起動し、monitoring の開始／停止とは独立して動作します。
- config update では worker を停止して新 config で再起動します。
- アプリ終了時は worker を停止します。
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

- `credentials_path` に加え、`credentials_auto_detect` が有効な場合は Windows default・各 WSL ディストロの `.claude/.credentials.json` (`include_wsl_credentials`)・`extra_credentials_paths` を候補に含めます。
- default は `%USERPROFILE%\\.claude\\.credentials.json` です。
- ディレクトリ再帰探索はしません。候補ファイルだけを読み取り専用で確認します。
- credentials file 内容を別ファイルへコピー保存しません。
- `claudeAiOauth.accessToken` が取れた場合、OAuth usage API を呼びます。
- Claude OAuth usage API は experimental / best-effort source として扱います。
- schema 変更、HTTP error、token 期限切れ、missing credentials は fixed error code に変換します。
- refresh token 更新は v1 では行いません。

セキュリティポリシー:

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
- `probe_studio_devices`
- `read_studio_keymap`
- `studio_export_keymap`
- `studio_preview_keymap_restore`
- `studio_apply_keymap_restore`
- `studio_key_catalog`
- `studio_begin_edit`
- `studio_set_key`
- `studio_add_layer`
- `studio_rename_layer`
- `studio_remove_layer`
- `studio_save_changes`
- `studio_discard_changes`
- `studio_reset_to_keymap`
- `studio_has_unsaved`
- `studio_end_edit`
- `read_encoder_info`
- `read_encoder_bindings`
- `read_encoder_layer_bindings`
- `studio_set_encoder_bindings`
- `studio_encoder_has_unsaved`
- `studio_encoder_save`
- `studio_encoder_discard`
- `studio_encoder_clear_override`
- `read_combo_info`
- `read_combo`
- `studio_set_combo`
- `studio_combo_has_unsaved`
- `studio_combo_save`
- `studio_combo_discard`
- `studio_combo_delete`
- `studio_combo_reset_to_keymap`
- `debug_inject_uplink` (debug build のみ動作)

`show_config_file_location` は config path だけを Explorer で表示します。credentials path の reveal は行いません。

`get_app_icons` は exe path のリストを受け取り、Windows Shell からアイコンを抽出して PNG data URL の map を返します。抽出できない exe は結果に含めません。

`get_launch_at_login` / `set_launch_at_login` は `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run` の `Keylink Studio` 値を読み書きします。管理者権限は不要です。

## Public Artifacts

GitHub には source、docs、examples、icon source、Tauri icons を含めます。`target/`、`ui/node_modules/`、`ui/dist/`、個人用 `keylink-studio.toml`、生成済み installer は含めません。

## 実装上の注記

- Host Link packet type は `0x01 HOST_HELLO`、`0x02 DEVICE_HELLO`、`0x03 ERROR`、`0x04 PING`、`0x05 PONG`、`0x10 AI_USAGE`、`0x20 TIME_SYNC`、`0x30 APP_LAYER`、`0x40 BATTERY_STATUS`、`0x50 HOST_ACTION`、`0x60 KEY_STATS`、`0x70 LAYER_STATE`、`0x80 KEY_PRESS`、`0x90 CONFIG_REQUEST`、`0x91 CONFIG_RESPONSE` です。
- `DEVICE_HELLO` の capability が送信を制御します。`APP_LAYER` packet は `APP_LAYER` capability を持つデバイスにのみ送信されます。
- `device_uid_hash = 0` は `None` に正規化されます。内部のデバイス設定が `Some(0)` を生成することはありません。
- Host Link の USB/BLE 差分は transport の検出と表示に閉じ込めます。packet wire format、protocol version、capability bit は USB/BLE で同一です。
- AI Usage provider は background worker でスナップショットを更新します。`Runner::tick()` は最新スナップショットを送信するだけで、provider の取得処理は行いません。
- Codex は利用可能な場合に `rate_limits` を quota source として使います。local history は fallback / activity estimate 専用です。
- Claude OAuth usage API は experimental / best-effort です。credentials 自動検出は明示パス・Windows デフォルト・WSL デフォルト・追加パスを試みます。
- Keymap Viewer は独立した ZMK Studio RPC client モジュールを使います。通常表示では USB serial / CDC ACM または BLE Studio transport から snapshot を取得します。KeymapタブとComboタブの編集ボタンは同じ編集モードを操作し、`StudioEditSession` が USB serial または BLE Studio transport を保持して `set_key_at` / layer management / save / discard を実行します。BLE Studio 編集では UI 側でキー書き込みを 1 件ずつ queue 処理し、pending 中は保存 / 破棄 / 編集終了 / レイヤー構造操作を止めます。未保存変更がある状態で他画面へ移動しようとした場合は、UI が遷移を止めて `保存して移動` / `破棄して移動` / `キャンセル` を提示します。保存または破棄が失敗した場合は遷移せず、ダイアログ内に失敗を表示します。pending 中は保存して移動 / 破棄して移動も止めます。保存／破棄時のConfig RPCはdirtyなEncoder／Combo featureだけを対象とし、Comboの未適用ローカルdraftだけを取り消す場合は不要なConfig RPCを送信しません。書き込み失敗時は未送信 queue を破棄し、復旧用の `studio_abort_edit` で編集セッションを破棄してから再読み込みします。
- `.keymap に戻す` は `StudioClient::reset_settings()` を実行してStudio保存状態を削除し、stock snapshotを再取得する。厳密なHost Link UIDがある場合は、mutation前に`ENCODER GET_INFO`で到達性を確認し、stock snapshotの実際の`Layer.id`と`encoder_id`全組合せへ`CLEAR_OVERRIDE`を送った後、`SAVE`を1回送る。`SAVE`はorphan overrideの削除も担う。Comboが同じUIDで利用可能な場合は`COMBO RESET_TO_KEYMAP`に続けて`SAVE`を送り、firmwareの初期Combo tableを永続化する。Studio RPCとHost Linkはtransactionを共有しないため、結果DTOはStudio、Config RPCのENCODER／COMBO各feature、snapshot再読込を別々に返し、部分成功をUIに表示する。
- Keymap Viewer のキー候補 catalog は、キータブの修飾子（任意）トグル行の下に、英字、数字、修飾キー、コントロール・スペース、記号、ナビゲーションを優先表示します。その他のカテゴリはナビゲーションの下に従来の相対順で表示します。
- Keymap backup は `schema = "keylink-studio.keymap-backup"` / `schema_version = 1` の JSON です。推奨拡張子は `-keymap.json` です。復元の真実は各 binding の `position`, `behavior_id`, `param1`, `param2` で、表示用の label と検証用の behavior 名も保持します。秘密情報、設定ファイル内容、ユーザー絶対パスは含めません。
- Restore は即保存しません。`StudioEditSession::apply_raw_writes` が `Behavior::Unknown { behavior_id, param1, param2 }` を使って既存 edit session の未保存変更として反映し、永続化は既存の `save_changes` に任せます。復元対象は backup と現在キーボードで共通する layer index と key position のみです。backup にしかない layer / position は書き込まず、現在キーボードにしかない layer / position は変更しません。device 名、connection type、layout 名、layer id/name は復元可否に使いません。復元対象の差分がない場合は UI で対象なしを表示し、apply は実行しません。復元または手動編集で実際に変更したキーは UI でハイライトします。
- Restore preview/apply では backup に出現する behavior id を対象 device へ問い合わせます。取得できた場合だけ behavior 名を正規化比較し、不一致 / missing / backup 側未解決の binding は書き込み対象から除外します。問い合わせ不能、BLE LayoutOnly、timeout、placeholder catalog などでは `behavior_verification = skipped` とし、UI で強警告を出したうえで raw 復元対象にします。
- Keymap backup file I/O は Rust command 側の `std::fs` に限定します。frontend に汎用 fs 権限は追加しません。読み込みは通常ファイルのみ、metadata と parser の両方で 1 MiB 上限を確認します。書き込みは親ディレクトリが存在する場合だけ行い、path と JSON 内容は log に出しません。
- Keymap Viewer の Host Link 紐付けは、ZMK Studio `get_device_info().serial_number` が 16 桁小文字 hex UID として返る場合、Host Link `device_uid_hash` の `uid:<hex>` と UID 優先で照合します。古い firmware 向けに従来の serial number 照合と、Bluetooth の product 名が一意な場合の fallback を残します。
- Codex JSONL session ファイルはメモリ使用を抑えるため 4 MB の末尾キャップで読み込みます。末尾ウィンドウの先頭行 (一部だけの可能性あり) は破棄されます。
- uplink packet (`0x40`〜`0x80`) は毎 tick 非ブロッキングでドレインし、`DEVICE_HELLO` の capability bits でゲートされます。`LAYER_STATE` は表示専用であり、ルールエンジンには渡されません。
- キー統計は device uid 単位の日別バケットとして `<data_dir>/stats/` へ永続化されます。記録するのは位置情報のみで、キーの内容は記録しません。
- `HOST_ACTION` はデバイス単位の許可リスト制 (`[actions]`、既定 disabled、`actions.devices."uid:..."` でキー指定) で実行されます。HID value byte をパスやコマンドとして解釈しません。
- デバイス固有の `unmatched_action` が設定されている場合、グローバルの `layer_switch.unmatched_action` より優先されます。
- 次のモジュールは Windows 専用処理を含みます: foreground watcher (`foreground.rs`)、exe アイコン抽出 (`icon.rs`)、起動時登録レジストリ管理 (`startup.rs`)、フォルダを開く処理 (`explorer.rs`、0.8.0〜)、アプリ起動/前面化処理 (`app_launch.rs`、0.8.1〜)。Windows 依存部分は他プラットフォームでは no-op スタブやエラーになります (`explorer.rs` は非対応エラー、`app_launch.rs` の起動自体は `Command::spawn` でクロスプラットフォーム)。
- `HOST_ACTION` の組み込みアクションのうち Windows 固有の挙動: `open_folder` は同じフォルダを開いている Explorer ウィンドウがあれば前面化し (なければ新規、`prefer_tab` で既存ウィンドウのタブ再利用を試行)、`launch` は実行ファイル名 (basename。`.lnk` はリンク先を解決、または `match_exe` で上書き) が一致する可視ウィンドウがあれば前面化し (最小化は復帰・最大化は保持)、なければ `ShellExecuteW` で起動します (exe / `.lnk` / 関連付け対応)。いずれもベストエフォートです。`show_window` はトレイ/最小化からの復帰を含みます。
