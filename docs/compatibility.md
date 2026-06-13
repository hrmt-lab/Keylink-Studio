# 互換性情報

RawHID Host のアプリバージョンと Host Link プロトコルバージョンは別管理です。

- アプリバージョンは PC 側アプリ、UI、CLI、配布物のバージョンです。
- Host Link プロトコルバージョンは、ZMK firmware 側と Raw HID packet で合意する通信仕様のバージョンです。
- RawHID Host `0.6.0` 時点の Host Link プロトコルは `v1` です。

## 互換性一覧

| ホストアプリバージョン | Host Link プロトコル | 必要 firmware 側対応 | 主な機能 |
| --- | --- | --- | --- |
| `0.8.0` | `v1` | 0.6.0 と同じ (firmware 側変更なし) | 0.7.0 の機能 + HOST_ACTION に `open_folder` (Explorer でフォルダを開く / 既存ウィンドウ前面化 / `prefer_tab` でタブ再利用) を追加。アクション画面のバインディング編集・ID順表示、システムトレイのツールチップにバッテリー残量表示、`show_window` の最小化/トレイからの復帰修正 |
| `0.7.0` | `v1` | 0.6.0 と同じ (firmware 側変更なし) | 0.6.0 の機能 + UI デザイン全面刷新 (Studio Gadget)、アクセント色カスタマイズ、マニュアル画像更新 |
| `0.6.0` | `v1` | 従来対応 + 任意で uplink capability (`BATTERY` / `HOST_ACTION` / `KEY_STATS` / `LAYER_STATE`) | 0.5.0 の機能 + バッテリー表示、キーボードからの PC 操作、タイピング統計ヒートマップ、レイヤー逆同期 |
| `0.5.0` | `v1` | `HOST_HELLO` / `DEVICE_HELLO`, capability 情報, `APP_LAYER`, `TIME_SYNC`, `AI_USAGE` | デバイス単位のレイヤールール (Global fallback 廃止)、時刻同期、AI 使用量送信、Keymap Viewer、自動起動 |
| `0.4.0` | `v1` | `HOST_HELLO` / `DEVICE_HELLO`, capability 情報, `APP_LAYER`, `TIME_SYNC`, `AI_USAGE` | アプリ別レイヤー切り替え (即時検知)、時刻同期、AI 使用量送信、Keymap Viewer、自動起動 |
| `0.3.1` | `v1` | `HOST_HELLO` / `DEVICE_HELLO`, capability 情報, `APP_LAYER`, `TIME_SYNC`, `AI_USAGE` | アプリ別レイヤー切り替え、時刻同期、AI 使用量送信、Keymap Viewer |
| `0.3.0` | `v1` | `HOST_HELLO` / `DEVICE_HELLO`, capability 情報, `APP_LAYER`, `TIME_SYNC`, `AI_USAGE` | アプリ別レイヤー切り替え、時刻同期、AI 使用量送信、Keymap Viewer |
| `0.2.x` | `v1` | `HOST_HELLO` / `DEVICE_HELLO`, `APP_LAYER`, `TIME_SYNC`, `AI_USAGE` | アプリ別レイヤー切り替え、時刻同期、AI 使用量送信 |
| `0.1.x` | `v1` | `HOST_HELLO` / `DEVICE_HELLO`, `APP_LAYER`, `TIME_SYNC` | アプリ別レイヤー切り替え、時刻同期 |

## Host Link v1

| 項目 | 値 |
| --- | --- |
| magic | `HL` |
| プロトコルバージョン byte | `0x01` |
| HID Usage Page | `0xFF60` |
| HID Usage | `0x0061` |
| payload サイズ | 32 bytes |
| Report ID | `0x00` |

## 機能ごとの必要対応

| 機能 | ホストアプリ側 | Firmware 側 | 補足 |
| --- | --- | --- | --- |
| `APP_LAYER` | Raw HID packet 送信とルール振り分け | `APP_LAYER` packet の受信と layer set / clear 処理 | RawHID Host は `APP_LAYER` capability を返したデバイスにだけ送信します。 |
| `TIME_SYNC` | ローカル時刻 snapshot の packet 送信 | `TIME_SYNC` packet の受信と表示状態の更新 | Host は毎秒送信しません。Firmware 側は uptime 差分で表示時刻を進める想定です。 |
| `AI_USAGE` | Codex / Claude Code 使用量の取得と packet 送信 | `AI_USAGE` packet の受信と表示処理 | error / status は固定 code です。機密情報の raw data は送信しません。 |
| Keymap Viewer / ZMK Studio 読み取り専用ビューア | USB serial / CDC ACM の ZMK Studio RPC client | ZMK Studio USB serial RPC と unlocked Studio state | Host Link Raw HID とは別経路です。編集、書き込み、保存、復元、unlock は行いません。 |

## Uplink 機能 (0.6.0+)

| 機能 | capability bit | packet | Firmware 側対応 |
| --- | --- | --- | --- |
| バッテリー表示 | `BATTERY (bit4)` | `BATTERY_STATUS 0x40` | 自分+ペリフェラルの残量を変化時+定期送信 |
| キーボードからの PC 操作 | `HOST_ACTION (bit5)` | `HOST_ACTION 0x50` | action_id + value を wrapping seq 付きで送る behavior |
| タイピング統計 | `KEY_STATS (bit6)` | `KEY_STATS 0x60` | 位置別カウンタ (u16×キー数) を定期送信して 0 クリア |
| レイヤー逆同期 | `LAYER_STATE (bit7)` | `LAYER_STATE 0x70` | layer-state-changed イベントで最上位レイヤー+mask 送信 |

- いずれも任意機能です。capability bit を立てた機能だけ host が受け付けます (段階実装可)。
- uplink は best-effort です。host が読んでいない間の packet は失われます。
- host はこれらの packet を表示・記録に使い、`LAYER_STATE` を `APP_LAYER` としてエコーバックしません。

## 補足

- Host Link `v1` は、`DEVICE_HELLO` で返る capability bits を使って機能ごとの送信可否を判断します。
- 対応 capability がないデバイスも Host Link device として表示される場合がありますが、その機能の packet は送信されないことがあります。
- Keymap Viewer は ZMK Studio transport を使います。Host Link Raw HID transport ではありません。
- Firmware 更新 / 書き換えは Host Link `v1` には含まれません。
