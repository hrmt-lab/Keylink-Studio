# 互換性情報

RawHID Host のアプリバージョンと Host Link プロトコルバージョンは別管理です。

- アプリバージョンは PC 側アプリ、UI、CLI、配布物のバージョンです。
- Host Link プロトコルバージョンは、ZMK firmware 側と Raw HID packet で合意する通信仕様のバージョンです。
- RawHID Host `0.9.0` 時点の Host Link プロトコルは `v1` です。

## 互換性一覧

| ホストアプリバージョン | Host Link プロトコル | 必要な firmware 側対応 | 主な機能 |
| --- | --- | --- | --- |
| `0.9.0` | `v1` | Host Link は 0.6.0 と同系統。キーテスターには `KEY_PRESS` capability が必要。ZMK Studio 編集には firmware 側の ZMK Studio 対応と実機での `&studio_unlock` が必要 | ZMK Studio RPC 経由のキーマップ編集を拡張。MO / TG / TO、MT / LT、Sticky、Bluetooth、Output、Mouse、Utility、System、レイヤー追加 / 名前変更 / 削除、キーテスター。README / docs を現状仕様へ更新 |
| `0.8.5` | `v1` | 0.6.0 と同じ (Host Link firmware 側変更なし)。ZMK Studio 編集 v1 には firmware 側の ZMK Studio 対応と実機での `&studio_unlock` が必要 | 0.8.1 の機能 + ZMK Studio RPC 経由のキーマップ編集 v1 (通常キー / 透過 / 無効、保存 / 破棄、キーコードピッカー)。キーマップ表示の自動縮小、左右余白調整、キーマップ画面・ダッシュボードのデバイス名順表示 |
| `0.8.1` | `v1` | 0.6.0 と同じ | HOST_ACTION `launch` の focus-or-launch 化、起動中アプリピッカー、参照ボタン、`.lnk` / 関連付け起動、`match_exe` |
| `0.8.0` | `v1` | 0.6.0 と同じ | HOST_ACTION `open_folder`、アクション画面のバインディング編集、ID 順表示、トレイのバッテリー残量表示、`show_window` の最小化 / トレイ復帰修正 |
| `0.7.0` | `v1` | 0.6.0 と同じ | UI デザイン全面刷新、アクセント色カスタマイズ、マニュアル画像更新 |
| `0.6.0` | `v1` | 従来対応 + 任意で uplink capability (`BATTERY` / `HOST_ACTION` / `KEY_STATS` / `LAYER_STATE`) | バッテリー表示、キーボードからの PC 操作、タイピング統計ヒートマップ、レイヤー逆同期 |
| `0.5.0` | `v1` | `HOST_HELLO` / `DEVICE_HELLO`, capability 情報, `APP_LAYER`, `TIME_SYNC`, `AI_USAGE` | デバイス単位のレイヤールール、時刻同期、AI 使用量送信、Keymap Viewer、自動起動 |
| `0.4.0` | `v1` | `HOST_HELLO` / `DEVICE_HELLO`, capability 情報, `APP_LAYER`, `TIME_SYNC`, `AI_USAGE` | アプリ別レイヤー切り替え、時刻同期、AI 使用量送信、Keymap Viewer、自動起動 |
| `0.3.x` | `v1` | `HOST_HELLO` / `DEVICE_HELLO`, capability 情報, `APP_LAYER`, `TIME_SYNC`, `AI_USAGE` | アプリ別レイヤー切り替え、時刻同期、AI 使用量送信、Keymap Viewer |
| `0.2.x` | `v1` | `HOST_HELLO` / `DEVICE_HELLO`, `APP_LAYER`, `TIME_SYNC`, `AI_USAGE` | アプリ別レイヤー切り替え、時刻同期、AI 使用量送信 |
| `0.1.x` | `v1` | `HOST_HELLO` / `DEVICE_HELLO`, `APP_LAYER`, `TIME_SYNC` | アプリ別レイヤー切り替え、時刻同期 |

## Host Link v1

| 項目 | 値 |
| --- | --- |
| magic | `HL` |
| protocol version byte | `0x01` |
| HID Usage Page | `0xFF60` |
| HID Usage | `0x0061` |
| payload size | 32 bytes |
| HID write size | 33 bytes |
| Report ID | `0x00` |

## 機能ごとの必要対応

| 機能 | ホストアプリ側 | Firmware 側 | 補足 |
| --- | --- | --- | --- |
| `APP_LAYER` | Raw HID packet 送信とルール振り分け | `APP_LAYER` packet の受信と layer set / clear 処理 | `APP_LAYER` capability を返したデバイスにだけ送信します。 |
| `TIME_SYNC` | ローカル時刻 snapshot の packet 送信 | `TIME_SYNC` packet の受信と表示状態更新 | Host は毎秒送信しません。firmware 側は uptime 差分で表示時刻を進める想定です。 |
| `AI_USAGE` | Codex / Claude Code 使用量の取得と packet 送信 | `AI_USAGE` packet の受信と表示処理 | error / status は固定 code です。機密情報の raw data は送信しません。 |
| BATTERY 表示 | `BATTERY_STATUS` uplink 受信と UI / tray 表示 | `BATTERY_STATUS 0x40` 送信 | `BATTERY` capability が必要です。 |
| HOST_ACTION | 許可リストに基づく PC 側アクション実行 | `HOST_ACTION 0x50` 送信 | `HOST_ACTION` capability が必要です。既定 disabled です。 |
| KEY_STATS | 押下回数の記録とヒートマップ表示 | `KEY_STATS 0x60` 送信 | 記録するのは position と回数のみです。 |
| LAYER_STATE | アクティブレイヤー表示 | `LAYER_STATE 0x70` 送信 | 表示専用で、`APP_LAYER` としてエコーバックしません。 |
| KEY_PRESS | キーテスターのリアルタイム表示 | `KEY_PRESS 0x80` 送信 + `KEY_PRESS` capability | 押下 / 離しの一時表示のみで、累積記録はしません。 |
| Keymap Viewer / ZMK Studio | USB serial / CDC ACM の ZMK Studio RPC client | ZMK Studio USB serial RPC と unlocked Studio state | Host Link Raw HID とは別経路です。BLE transport は対象外です。 |

## ZMK Studio 編集の互換性

ZMK Studio 編集は Host Link protocol ではなく、`zmk-studio-api` 経由の Studio RPC を使います。Host Link firmware を変更しなくても、firmware 側が ZMK Studio に対応していれば利用できます。

必要条件:

- USB serial / CDC ACM の ZMK Studio transport が使えること
- 実機で `&studio_unlock` を実行して Studio が unlocked であること
- 編集対象 behavior が firmware 側に存在すること

編集できる behavior はアプリ側の UI に段階的に追加されています。対応していない behavior や firmware 側に role がない behavior は、`missing_behavior_role` などのエラーになります。

## 補足

- Host Link `v1` は `DEVICE_HELLO` の capability bits を使って機能ごとの送受信可否を判断します。
- 対応 capability がないデバイスも Host Link device として表示される場合がありますが、その機能の packet は送信または実行されません。
- uplink は best-effort です。host が読んでいない間の packet は失われます。
- firmware 更新 / 書き換えは Host Link `v1` には含まれません。
