# 互換性情報

Keylink Studio のアプリバージョンと Host Link protocol version は別管理です。

- アプリバージョンは PC 側アプリ、UI、CLI、配布物のバージョンです。
- Host Link protocol version は firmware 側と Host Link packet で合意する通信仕様のバージョンです。
- 現在の main は Host Link v2 を前提にします。v1 firmware との後方互換はありません。

## 互換性一覧

| Host app | Host Link protocol | Firmware requirement | Notes |
| --- | --- | --- | --- |
| `main` | `v2` | Host Link v2 Common Header / 64 byte packet と Config RPC `ENCODER` feature 対応 firmware | `HOST_HELLO` / `DEVICE_HELLO` / 既存 packet と Config RPC を v2 layout で送受信 |
| `1.1.2` 以前 | `v1` | Host Link v1 / 32 byte packet 対応 firmware | 旧仕様。v2 host とは wire compatible ではない |

## Host Link v2

| 項目 | 値 |
| --- | --- |
| magic | `HL` |
| protocol version byte | `0x02` |
| HID Usage Page | `0xFF60` |
| HID Usage | `0x0061` |
| Host Link packet size | 64 bytes |
| HID write size | 65 bytes |
| Report ID | `0x00` |
| Header | 12 byte Common Header |
| Payload budget | 52 bytes |

Host Link v2 は USB HID と BLE HOG で同じ Host Link packet format を使います。BLE chunking が必要な場合も、Host Link parser へ渡す前に transport 層で 64 byte packet へ復元します。

## Host Link v1

Host Link v1 は 32 byte packet と packet type ごとの個別 layout を使う旧仕様です。v2 host は v1 packet を `UnsupportedVersion(0x01)` として扱い、HELLO 成功とはみなしません。

## 機能ごとの必要対応

| 機能 | Host app 側 | Firmware 側 | 補足 |
| --- | --- | --- | --- |
| `APP_LAYER` | v2 `APP_LAYER` packet 送信と rule 管理 | `APP_LAYER` packet 受信、`APP_LAYER` capability | device ごとの UID 設定がある場合のみ送信 |
| `TIME_SYNC` | v2 `TIME_SYNC` packet 送信 | `TIME_SYNC` packet 受信、`TIME_SYNC` capability | 毎秒送信はしない |
| `AI_USAGE` | background worker の snapshot を v2 `AI_USAGE` packet で送信 | `AI_USAGE` packet 受信、`AI_USAGE` capability | raw credentials や API response は送信しない |
| `BATTERY_STATUS` | v2 uplink 受信と UI / tray 表示 | `BATTERY_STATUS` uplink 送信 | `source=0` は central/self、`1..=3` は peripheral |
| `HOST_ACTION` | allowlist に基づく PC 側 action 実行 | `HOST_ACTION` uplink 送信、`HOST_ACTION` capability | header `seq` で duplicate を抑制 |
| `KEY_STATS` | 押下回数差分の保存と表示 | `KEY_STATS` uplink 送信、`KEY_STATS` capability | 記録するのは position と回数のみ |
| `LAYER_STATE` | 表示専用の layer state 受信 | `LAYER_STATE` uplink 送信、`LAYER_STATE` capability | `APP_LAYER` として echo back しない |
| `KEY_PRESS` | key tester のリアルタイム表示 | `KEY_PRESS` uplink 送信、`KEY_PRESS` capability | 累積記録はしない |
| Config RPC / encoder | `ENCODER` の情報取得、CW / CCW override 編集、保存、破棄、override解除 | `CONFIG_RPC` capability と `ENCODER GET_INFO` / `GET_BINDINGS` / `SET_BINDINGS` / `GET_DIRTY` / `SAVE` / `DISCARD` / `CLEAR_OVERRIDE` | Host は `seq + feature + op` で応答を照合し、timeout 時は1回再試行 |
| Config RPC / combo | core／CLI／Tauri／Keymap Viewerで全8 operation対応 | `COMBO GET_INFO` / `GET_COMBO` / `SET_COMBO` / `GET_DIRTY` / `SAVE` / `DISCARD` / `DELETE_COMBO` / `RESET_TO_KEYMAP`、Settings load／保存 | CLI mutationは対象UID必須。ComboのExport／Restoreは未実装 |
| ZMK Studio keymap editing | ZMK Studio RPC client | ZMK Studio 対応 firmware と unlocked Studio state | Host Link v2 とは別 transport |

## 補足

- `DEVICE_HELLO` v2 は `capabilities` と `device_uid_hash` を返します。v1 の `protocol_min` / `protocol_max` はありません。
- `device_uid_hash = 0` は host 側で `None` に正規化します。
- Config RPCは`ENCODER` / `COMBO` featureをHost／Firmwareとも実装済みです。ComboはKeymap Viewerの共通保存／破棄／`.keymapに戻す`へ統合済みで、Export／Restoreだけが未実装です。tap danceなど52 byte payloadを超えるデータの分割方式は将来拡張です。
