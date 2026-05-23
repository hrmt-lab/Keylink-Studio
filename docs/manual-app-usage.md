# RawHID Host アプリ操作マニュアル / App Usage Manual

## 日本語

### 概要

RawHID Host の GUI は、設定編集、デバイス確認、監視開始、ログ確認を行うための Tauri + React アプリです。設定は `rawhid-host.toml` に保存され、監視中に保存した変更も即時反映されます。

### Dashboard

`Dashboard` は監視状態の確認と開始 / 停止を行う画面です。

- `Start Monitoring` で監視を開始します。
- `Stop Monitoring` で監視を停止します。
- 接続デバイス数、現在のレイヤー、適用中ルール、ログを確認できます。
- 時刻同期が有効な場合、監視開始後に HELLO 成功済みデバイスへ TIME_SYNC が送られます。

### Layer Rules

アクティブアプリとレイヤー番号の対応を設定します。

GUI での追加手順:

1. 起動中アプリ一覧から対象アプリを選びます。
2. レイヤー番号を選びます。
3. ルールを追加します。

GUI で追加したルールは `exe` 条件を使います。`path` や `title` を使う場合は設定ファイルを直接編集してください。

マッチング優先順位:

| 優先度 | 条件 | 説明 |
| --- | --- | --- |
| 1 | `path` | 実行ファイルのフルパス完全一致。大文字小文字は区別しません。 |
| 2 | `exe` | 実行ファイル名の完全一致。大文字小文字は区別しません。 |
| 3 | `title` | ウィンドウタイトルの部分一致。大文字小文字は区別しません。 |

同じ優先度で複数一致した場合は設定順が優先されます。どのルールにも一致しない場合は `clear` を送信します。

Windows では Desktop、Taskbar、File Explorer が `explorer.exe` として見えることがあります。`explorer.exe` ルールを作ると、デスクトップクリックでもそのルールに一致する場合があります。通常運用では Explorer ルールは作らない方が扱いやすいです。

### Time Sync

キーボード側ディスプレイへ日時情報を送る設定です。

| 設定 | 説明 |
| --- | --- |
| Enabled | TIME_SYNC を送るかどうか |
| Display Format | 表示形式のヒント |
| Clock Mode | `24h` または `12h` |
| Periodic Sync | 定期補正間隔。`0` で無効 |
| Timezone Offset | 空欄ならホストの現在 offset。指定時は分単位 |

`time_hms` でも毎秒送信はしません。ZMK 側が TIME_SYNC 受信時の uptime を基準に秒を進める想定です。

### Devices

Raw HID 候補を列挙し、HELLO の成否を確認します。

- Usage Page / Usage が設定値と一致する HID を候補にします。
- 候補へ HELLO を送ります。
- HELLO に成功したデバイスだけが監視時の送信対象になります。

### Settings

Polling と HID の基本値を編集します。

| 設定 | 既定値 |
| --- | --- |
| Poll Interval | `500` ms |
| Usage Page | `0xFF60` |
| Usage | `0x61` |
| HELLO Timeout | `200` ms |

設定保存後、監視中であれば runner が新しい設定で再構築されます。ユーザーが別途「反映」操作をする必要はありません。

### システムトレイ

ウィンドウを閉じてもアプリは終了せず、トレイに残ります。完全に終了する場合はトレイメニューの `Quit` を使います。

---

## English

### Overview

The GUI is a Tauri + React app for editing settings, checking devices, starting monitoring, and reading logs. Settings are saved to `rawhid-host.toml`. Changes saved while monitoring is running are applied immediately.

### Dashboard

- Start / stop monitoring
- Check connected device count
- Check current layer and matched rule
- Read recent logs
- Send initial TIME_SYNC after devices pass HELLO when time sync is enabled

### Layer Rules

GUI-created rules use the `exe` condition. Edit `rawhid-host.toml` directly when you need `path` or `title`.

Matching priority:

| Priority | Condition | Description |
| --- | --- | --- |
| 1 | `path` | Full process path, case-insensitive exact match |
| 2 | `exe` | Executable filename, case-insensitive exact match |
| 3 | `title` | Window title, case-insensitive substring match |

If nothing matches, the host sends `clear`.

On Windows, Desktop, Taskbar, and File Explorer can all appear as `explorer.exe`. Avoid an Explorer rule unless you explicitly want desktop focus to select a layer.

### Time Sync

TIME_SYNC sends time information for keyboard displays.

- It is disabled by default.
- It is sent on monitoring start, verified device changes, display-relevant boundaries, and periodic correction.
- It is not sent every second. ZMK advances seconds from uptime.

### Devices

The Devices page lists Raw HID candidates and checks HELLO. Only HELLO-verified devices are used while monitoring.

### Settings

Basic settings:

- Poll interval
- HID Usage Page
- HID Usage
- HELLO timeout

Saved settings are applied to the running monitor automatically.
