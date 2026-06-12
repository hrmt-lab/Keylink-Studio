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
| `0x40` | `BATTERY_STATUS` | ZMK -> Host | battery levels (self + peripherals) |
| `0x50` | `HOST_ACTION` | ZMK -> Host | trigger a host-side action |
| `0x60` | `KEY_STATS` | ZMK -> Host | per-position key press deltas |
| `0x70` | `LAYER_STATE` | ZMK -> Host | active layer report (display only) |

`0x40` 以降は device-initiated (uplink) packet です。host は監視中に非ブロッキング読みで受信します。各 type は対応する capability bit を `DEVICE_HELLO` で立てた device からのみ受け付けます。

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

## BATTERY_STATUS (ZMK -> Host)

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x40` |
| `4` | 1 | count | entry count, `1..=4` |
| `5+2i` | 1 | source[i] | `0=self/dongle`, `1=left`, `2=right`, `3=aux` |
| `6+2i` | 1 | level[i] | `0..=100`, or `0xFF` = unknown / disconnected |
| `5+2*count..31` | - | reserved | must be zero |

Validation: count `1..=4`、source `0..=3` かつ packet 内で重複禁止、level は `0..=100` か `0xFF`(`101..=254` は reject)。変化時+定期(~5分)送信を想定します。

## HOST_ACTION (ZMK -> Host)

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x50` |
| `4` | 1 | action_id | opaque id; meaning is defined by host config |
| `5` | 1 | value | action argument, `0` if unused |
| `6` | 1 | reserved | must be zero |
| `7` | 1 | seq | u8 wrapping counter |
| `8..31` | 24 | reserved | must be zero |

host は同一 device からの **同じ seq の連続受信を 1 回として扱います**(firmware の二重送信対策)。リトライは不要です。action の実行内容は host 側 config の許可リスト(`[actions]`)で device 単位 (`actions.devices."uid:..."`) に定義し、未定義 id・未設定 device はログのみです。`value` を path やコマンドとして解釈することはありません。

## KEY_STATS (ZMK -> Host)

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x60` |
| `4` | 1 | entry_count | `1..=8` |
| `5` | 1 | flags | bit0 = `MORE_FOLLOWS`; other bits reserved zero |
| `6` | 1 | reserved | must be zero |
| `7` | 1 | seq | u8 wrapping counter (gap = lost packets) |
| `8+3i` | 1 | position[i] | key position (ZMK Studio physical layout position) |
| `9+3i..10+3i` | 2 | delta[i] | u16 LE presses since last report; `0` is rejected |
| `8+3*entry_count..31` | - | reserved | must be zero |

firmware は位置別カウンタを保持し、定期的(30〜60秒)に **非ゼロの position だけ** を送って 0 クリアします。8 entry を超える場合は複数 packet に分割し、最後以外に `MORE_FOLLOWS` を立てます。host は seq gap を警告ログにしつつ受信分を加算します(欠落はアンダーカウントとして許容)。

## LAYER_STATE (ZMK -> Host)

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x70` |
| `4` | 1 | active_layer | highest active layer, `0..=31` |
| `5..6` | 2 | reserved | must be zero |
| `7` | 1 | seq | u8 wrapping counter |
| `8..11` | 4 | layer_mask | u32 LE, bit i = layer i active; `0` = top layer only |
| `12..31` | 20 | reserved | must be zero |

`layer_mask` が非ゼロの場合、`active_layer` の bit が立っていなければ reject します。layer-state-changed イベント時に送信(~50ms デバウンス推奨)します。**host はこの報告を表示にのみ使い、APP_LAYER としてエコーバックしません。**

## ZMK 実装時の要点

- 33 byte write の先頭は Report ID `0x00` です。
- payload は常に 32 byte です。
- magic `"HL"` と version `0x01` を確認してください。
- reserved byte は zero を前提にしてください。
- `HOST_HELLO` に対して、同じ `seq` の `DEVICE_HELLO` を返してください。
- `APP_LAYER` は packet type ではなく `action` で set / clear を分岐してください。
- `AI_USAGE` の history fallback は quota ではありません。`quota_source=0` の reset は表示しないでください。
- uplink packet (`0x40`〜`0x70`) を送る場合は、対応する capability bit を `DEVICE_HELLO` で立ててください。bit が立っていない type は host が破棄します。
- uplink は best-effort です。host が読んでいない間(監視停止中など)の packet は失われます。`KEY_STATS` の欠落はアンダーカウントとして扱われます。
- `HOST_ACTION` の応答は最大で host の polling 間隔(既定 500ms)遅れます。

## Device identity and capabilities

`DEVICE_HELLO` v1 may include stable identity and capability fields. Host uses these fields for per-device feature routing.

```text
0..1    magic "HL"
2       protocol_version
3       type = 0x02 DEVICE_HELLO
4       protocol_min
5       protocol_max
6       reserved zero
7       seq
8..11   capabilities u32 LE
12..19  device_uid_hash u64 LE
20..31  reserved zero
```

Validation:

- `seq` must match the preceding `HOST_HELLO`.
- byte `6` and bytes `20..31` must be zero.
- `device_uid_hash = 0` is normalized to `None` on the host side.
- Non-zero `device_uid_hash` is displayed and stored as `uid:<16 lowercase hex digits>`.

Capability bits:

```text
bit0 = APP_LAYER
bit1 = TIME_SYNC
bit2 = AI_USAGE
bit3 = THEME
bit4 = BATTERY      (device sends BATTERY_STATUS)
bit5 = HOST_ACTION  (device sends HOST_ACTION)
bit6 = KEY_STATS    (device sends KEY_STATS)
bit7 = LAYER_STATE  (device sends LAYER_STATE)
```

RawHID Host does not send `APP_LAYER` packets to a device unless `APP_LAYER` capability is present. The device is still shown as a Host Link device in the Devices page.

### Host behavior for capabilities

The host treats `capabilities` as feature gates. A verified Host Link device remains visible in the Devices page even when a capability is missing, but feature packets are sent only when the relevant capability is present. In v1 this gate is enforced for `APP_LAYER` routing.

`device_uid_hash` is an identity hint, not a transport path. The host serializes non-zero values as `uid:<16 lowercase hex digits>` for config and UI. The value `0` is invalid as an identity and is normalized to `None`.
