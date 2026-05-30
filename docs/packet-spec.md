# RawHID Host Packet Specification

この仕様は、host 側 `rawhid-host` と ZMK 側 Raw HID 受信処理の間で使う packet を定義します。

## Transport

| Item | Value |
| --- | --- |
| HID Usage Page | `0xFF60` |
| HID Usage | `0x0061` |
| HID write size | 33 bytes |
| Report ID | `0x00` |
| Payload size | 32 bytes |
| Multi-byte values | little-endian |

host が `hidapi` で write する buffer は、先頭 1 byte の Report ID `0x00` と 32 byte payload です。

## Common Header

| Offset | Size | Field | Value |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | ASCII `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | packet type |

## Packet Types

| Type | Name | Direction | Notes |
| ---: | --- | --- | --- |
| `0x01` | `HOST_HELLO` | Host -> ZMK | same seq must be returned |
| `0x02` | `DEVICE_HELLO` | ZMK -> Host | response to `HOST_HELLO` |
| `0x03` | `ERROR` | ZMK -> Host | reserved / decodable in v1 |
| `0x04` | `PING` | Host -> ZMK | reserved / decodable in v1 |
| `0x05` | `PONG` | ZMK -> Host | reserved / decodable in v1 |
| `0x10` | `AI_USAGE` | Host -> ZMK | AI usage snapshot |
| `0x20` | `TIME_SYNC` | Host -> ZMK | time display sync |
| `0x30` | `APP_LAYER` | Host -> ZMK | app layer set/clear |

## HOST_HELLO / DEVICE_HELLO

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x01` or `0x02` |
| `4..6` | 3 | reserved | must be zero |
| `7` | 1 | seq | u8 wrapping counter |
| `8..31` | 24 | reserved | must be zero |

`seq` は u8 wrapping counter です。`rawhid-host` は `HOST_HELLO` を送り、同じ `seq` の `DEVICE_HELLO` が返った device だけを verified として扱います。

`byte 4..6` と `byte 8..31` は v1 では reserved zero です。

## APP_LAYER

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x30` |
| `4` | 1 | action | `1=set`, `2=clear` |
| `5` | 1 | layer | `0..31` for set, `0` for clear |
| `6` | 1 | reserved | must be zero |
| `7` | 1 | seq | u8 wrapping counter |
| `8..31` | 24 | reserved | must be zero |

Validation rules:

- `action = 1` のとき `layer` は `0..31`
- `action = 2` のとき `layer` は `0`
- unknown action は reject
- reserved byte が nonzero の packet は reject

## AI_USAGE

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x10` |
| `4` | 1 | provider | `1=codex`, `2=claude_code` |
| `5` | 1 | flags | see below |
| `6..7` | 2 | five_hour_used_bp | used percent * 100 |
| `8..9` | 2 | seven_day_used_bp | used percent * 100 |
| `10..13` | 4 | five_hour_reset_unix | uint32, little-endian |
| `14..17` | 4 | seven_day_reset_unix | uint32, little-endian |
| `18..21` | 4 | updated_unix | uint32, little-endian |
| `22` | 1 | error_code | see below |
| `23..31` | 9 | reserved | must be zero |

`used_bp` は used basis points です。`10000` は `100.00%` を意味します。

### Flags

| Bit | Name | Meaning |
| ---: | --- | --- |
| 0 | `five_hour_valid` | five-hour usage field is valid |
| 1 | `seven_day_valid` | seven-day usage field is valid |
| 2 | `estimated` | value is an estimate |
| 3 | `local_history_source` | source is local history |
| 4 | `quota_source` | source is quota/rate-limit information |
| 5 | `stale` | value is stale |
| 6 | `fallback_limit` | activity baseline was used |
| 7 | `error_present` | error_code is meaningful |

`AI_USAGE` には reset valid flag はありません。reset を quota reset として扱えるかどうかは、値そのものと `quota_source` で判断します。

### Source Rules

| Source | estimated | local_history_source | quota_source | reset fields |
| --- | ---: | ---: | ---: | --- |
| Codex `rate_limits` | 0 | 0 | 1 | quota reset |
| Codex history fallback | 1 | 1 | 0 | `0` |
| Claude OAuth API success | 0 | 0 | 1 | quota reset |

Codex history fallback は activity estimate です。quota reset として扱わないため、`five_hour_reset_unix=0` と `seven_day_reset_unix=0` を送ります。ZMK / UI 側も `quota_source=0` の reset は表示しない想定です。

### Error Codes

| Value | Name |
| ---: | --- |
| 0 | `none` |
| 1 | `source_disabled` |
| 2 | `missing_credentials` |
| 3 | `expired_credentials` |
| 4 | `auth_failed` |
| 5 | `rate_limited` |
| 6 | `fetch_failed` |
| 7 | `parse_failed` |
| 8 | `no_usage_data` |
| 9 | `missing_limit` |

Error/status は固定 code として扱います。access token、credentials JSON、Authorization header、HTTP request/response body、raw parse error は packet、UI、log、status に出しません。

## TIME_SYNC

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

`TIME_SYNC` は毎秒送りません。ZMK 側は `TIME_SYNC` 受信時の uptime を保存し、uptime 差分で表示秒を進める想定です。

## ZMK 実装時の要点

- 33 byte write の先頭は Report ID `0x00` です。
- payload は常に 32 byte です。
- magic `"HL"` と version `0x01` を確認してください。
- reserved byte は zero を前提にしてください。
- `HOST_HELLO` に対して、同じ `seq` の `DEVICE_HELLO` を返してください。
- `APP_LAYER` は packet type ではなく `action` で set / clear を分岐してください。
- `AI_USAGE` の history fallback は quota ではありません。`quota_source=0` の reset は表示しないでください。
