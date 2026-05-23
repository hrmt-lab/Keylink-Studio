# RawHID Host Packet Specification

## 日本語

この仕様は、ホスト側 `rawhid-host` と ZMK 側 Raw HID 受信処理の間で使う packet を定義します。

### Transport

| 項目 | 値 |
| --- | --- |
| HID Usage Page | `0xFF60` |
| HID Usage | `0x0061` |
| HID write size | 33 bytes |
| Report ID | `0x00` |
| Payload size | 32 bytes |
| Multi-byte values | little-endian |

ホスト側が `hidapi` で write する buffer は、先頭 1 byte の Report ID `0x00` と 32 byte payload です。

### Common Header

| Offset | Size | Field | Value |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | ASCII `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | packet type |

byte `4` 以降は packet type ごとに異なります。

### Packet Types

| Type | Name | Direction | ACK |
| ---: | --- | --- | --- |
| `0x01` | `set_layer` | Host -> ZMK | none |
| `0x02` | `clear` | Host -> ZMK | none |
| `0x10` | `hello` | Host -> ZMK | `hello_response` |
| `0x11` | `hello_response` | ZMK -> Host | none |
| `0x20` | `time_sync` | Host -> ZMK | none |

### Layer / Hello Layout

`set_layer`、`clear`、`hello`、`hello_response` は同じ layout です。

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x01`, `0x02`, `0x10`, `0x11` |
| `4` | 1 | layer | `set_layer` で `0..31` |
| `5` | 1 | flags | v1 では `0` |
| `6` | 1 | seq | host sequence |
| `7..31` | 25 | reserved | must be zero |

`hello_response` は `hello` と同じ `seq` を返します。`set_layer` と `clear` は ACK なしです。

### TIME_SYNC Layout

`time_sync` は type `0x20` です。

| Offset | Size | Field | Type / Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x20` |
| `4..7` | 4 | unix_time_sec | `uint32`, little-endian |
| `8..9` | 2 | tz_offset_min | `int16`, little-endian |
| `10` | 1 | weekday | ISO weekday, `1=Mon ... 7=Sun` |
| `11` | 1 | format_hint | display format preset |
| `12` | 1 | clock_mode | `0=24h`, `1=12h` |
| `13..31` | 19 | reserved | must be zero |

`weekday` は `unix_time_sec + tz_offset_min` を適用したローカル日付を基準にします。

### format_hint

| Value | Name | Intended display |
| ---: | --- | --- |
| `0` | `time_hm` | `HH:mm` |
| `1` | `time_hms` | `HH:mm:ss` |
| `2` | `date_ymd` | `YYYY-MM-DD` |
| `3` | `date_md` | `MM-DD` |
| `4` | `datetime_hm` | `YYYY-MM-DD HH:mm` |
| `5` | `weekday_hm` | `weekday HH:mm` |

### TIME_SYNC の送信タイミング

- 監視開始後、HELLO 成功済みデバイスへ送信
- デバイス再接続または検証済みデバイス集合の変更時に送信
- 分表示系は分境界で送信
- 日付表示系は日付境界で送信
- `periodic_sync_sec > 0` の場合、定期補正として送信
- `time_hms` でも毎秒送信しない

### ZMK 側推奨検証

- payload length が 32 bytes
- magic が `"HL"`
- version が `0x01`
- type が既知
- reserved bytes が zero
- `set_layer.layer` が `0..31`
- `time_sync.weekday` が `1..7`
- `time_sync.clock_mode` が `0` または `1`

---

## English

This document defines packets used between the host-side `rawhid-host` app and the ZMK Raw HID receiver.

### Transport

- HID Usage Page: `0xFF60`
- HID Usage: `0x0061`
- HID write size: 33 bytes
- Report ID: `0x00`
- Payload size: 32 bytes
- Multi-byte values: little-endian

The host writes one Report ID byte followed by the 32-byte payload.

### Common Header

| Offset | Size | Field | Value |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | ASCII `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | packet type |

### Packet Types

| Type | Name | Direction | ACK |
| ---: | --- | --- | --- |
| `0x01` | `set_layer` | Host -> ZMK | none |
| `0x02` | `clear` | Host -> ZMK | none |
| `0x10` | `hello` | Host -> ZMK | `hello_response` |
| `0x11` | `hello_response` | ZMK -> Host | none |
| `0x20` | `time_sync` | Host -> ZMK | none |

### Layer / Hello Layout

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x01`, `0x02`, `0x10`, `0x11` |
| `4` | 1 | layer | `0..31` for `set_layer` |
| `5` | 1 | flags | `0` in v1 |
| `6` | 1 | seq | host sequence |
| `7..31` | 25 | reserved | must be zero |

`hello_response` must return the same `seq` as `hello`. `set_layer` and `clear` do not have ACKs.

### TIME_SYNC Layout

| Offset | Size | Field | Type / Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x20` |
| `4..7` | 4 | unix_time_sec | `uint32`, little-endian |
| `8..9` | 2 | tz_offset_min | `int16`, little-endian |
| `10` | 1 | weekday | ISO weekday, `1=Mon ... 7=Sun` |
| `11` | 1 | format_hint | display format preset |
| `12` | 1 | clock_mode | `0=24h`, `1=12h` |
| `13..31` | 19 | reserved | must be zero |

TIME_SYNC is not sent every second. ZMK should store uptime when receiving TIME_SYNC and advance displayed time from the uptime delta.
