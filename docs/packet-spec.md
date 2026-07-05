# Keylink Studio Packet Specification

この文書は、Keylink Studio host と firmware の Host Link packet wire format を定義します。現在の主仕様は Host Link v2 です。

## Transport

| Item | Value |
| --- | --- |
| HID Usage Page | `0xFF60` |
| HID Usage | `0x0061` |
| HID report ID | `0x00` |
| HID write size | 65 bytes |
| HID read payload size | 64 bytes |
| Host Link packet size | 64 bytes |
| Multi-byte values | little-endian |

host が `hidapi` で write する buffer は、先頭 1 byte の report ID `0x00` と 64 byte の Host Link packet です。USB HID と BLE HOG の Host Link packet format は同じです。BLE 側で 64 byte report を一度に扱えない場合の chunking は transport 層の責務であり、Host Link parser は復元済みの 64 byte packet だけを扱います。

Host Link v1 との後方互換はありません。v1 firmware は HELLO 失敗または Host Link unsupported として扱います。

## Common Header

すべての Host Link v2 packet は同じ 12 byte header を持ちます。

| Offset | Size | Field | Value / Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | `magic` | ASCII `"HL"` |
| `2` | 1 | `protocol_version` | `0x02` |
| `3` | 1 | `packet_type` | Packet Types |
| `4` | 1 | `seq` | request / response correlation |
| `5` | 1 | `feature` | Config RPC 以外は `0` |
| `6` | 1 | `op` | Config RPC 以外は `0` |
| `7` | 1 | `status_or_flags` | request/uplink では flags、response では status |
| `8` | 1 | `payload_len` | `0..=52` |
| `9..11` | 3 | `reserved` | must be zero |
| `12..63` | 52 | `payload` | unused bytes must be zero |

Validation:

- `magic` は `"HL"`。
- `protocol_version` は `0x02`。
- `packet_type` は既知の値。
- `payload_len <= 52`。
- header reserved と、`payload_len` より後ろの payload byte はすべて `0`。

## Packet Types

| Type | Name | Direction | Notes |
| ---: | --- | --- | --- |
| `0x01` | `HOST_HELLO` | Host -> Firmware | handshake |
| `0x02` | `DEVICE_HELLO` | Firmware -> Host | handshake response |
| `0x03` | `ERROR` | Firmware -> Host | reserved / diagnostic |
| `0x04` | `PING` | Host -> Firmware | reserved |
| `0x05` | `PONG` | Firmware -> Host | reserved |
| `0x10` | `AI_USAGE` | Host -> Firmware | AI usage snapshot |
| `0x20` | `TIME_SYNC` | Host -> Firmware | time display sync |
| `0x30` | `APP_LAYER` | Host -> Firmware | app layer set / clear |
| `0x40` | `BATTERY_STATUS` | Firmware -> Host | battery uplink |
| `0x50` | `HOST_ACTION` | Firmware -> Host | host action trigger |
| `0x60` | `KEY_STATS` | Firmware -> Host | key stats diff |
| `0x70` | `LAYER_STATE` | Firmware -> Host | display-only layer state |
| `0x80` | `KEY_PRESS` | Firmware -> Host | real-time key press state |
| `0x90` | `CONFIG_REQUEST` | Host -> Firmware | reserved for Config RPC |
| `0x91` | `CONFIG_RESPONSE` | Firmware -> Host | reserved for Config RPC |

`CONFIG_REQUEST` / `CONFIG_RESPONSE` は v2 header 上の既知 type として予約されていますが、現在の host 実装では typed decode しません。通常 uplink としても扱いません。

## Payload Layouts

既存 packet の意味は v1 と同じですが、seq は Common Header に統一し、各 packet 固有データは payload 領域に配置します。Config RPC 以外は `feature = 0`, `op = 0`, `status_or_flags = 0` です。

### HOST_HELLO

| Header Field | Value |
| --- | --- |
| `packet_type` | `HOST_HELLO` (`0x01`) |
| `payload_len` | `0` |

host は header `seq` に handshake sequence を入れます。

### DEVICE_HELLO

`payload_len = 12`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0..3` | 4 | `capabilities` | u32 LE | Capability Bits |
| `4..11` | 8 | `device_uid_hash` | u64 LE | `0` は identity unavailable |

firmware は `HOST_HELLO` と同じ header `seq` を返します。v2 では v1 の `protocol_min` / `protocol_max` は持ちません。

### APP_LAYER

`payload_len = 2`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0` | 1 | `action` | u8 | `1=set`, `2=clear` |
| `1` | 1 | `layer` | u8 | set は `0..31`, clear は `0` |

### TIME_SYNC

`payload_len = 9`

| Payload Offset | Size | Field | Type |
| --- | ---: | --- | --- |
| `0..3` | 4 | `unix_time_sec` | u32 LE |
| `4..5` | 2 | `tz_offset_min` | i16 LE |
| `6` | 1 | `weekday` | u8, ISO `1=Mon .. 7=Sun` |
| `7` | 1 | `format_hint` | u8 |
| `8` | 1 | `clock_mode` | u8, `0=24h`, `1=12h` |

### AI_USAGE

`payload_len = 19`

| Payload Offset | Size | Field | Type |
| --- | ---: | --- | --- |
| `0` | 1 | `provider` | u8, `1=codex`, `2=claude_code` |
| `1` | 1 | `flags` | u8 |
| `2..3` | 2 | `five_hour_used_bp` | u16 LE |
| `4..5` | 2 | `seven_day_used_bp` | u16 LE |
| `6..9` | 4 | `five_hour_reset_unix` | u32 LE |
| `10..13` | 4 | `seven_day_reset_unix` | u32 LE |
| `14..17` | 4 | `updated_unix` | u32 LE |
| `18` | 1 | `error_code` | u8 |

`used_bp` は basis points で、`10000` が `100.00%` です。

### BATTERY_STATUS

`payload_len = 1 + 2 * count`

| Payload Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0` | 1 | `count` | `1..=4` |
| `1 + 2i` | 1 | `source[i]` | `0=central/self`, `1..=3=peripheral` |
| `2 + 2i` | 1 | `level[i]` | `0..=100`, `0xFF=unknown` |

BATTERY_STATUS の header `seq` は意味付けしません。

### HOST_ACTION

`payload_len = 2`

| Payload Offset | Size | Field |
| --- | ---: | --- |
| `0` | 1 | `action_id` |
| `1` | 1 | `value` |

header `seq` は duplicate 抑制に使います。

### KEY_STATS

`payload_len = 4 + 3 * entry_count`

| Payload Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0` | 1 | `entry_count` | `1..=8` |
| `1` | 1 | `flags` | bit0 = `MORE_FOLLOWS`, others zero |
| `2..3` | 2 | `reserved` | must be zero |
| `4 + 3i` | 1 | `position[i]` | key position |
| `5 + 3i .. 6 + 3i` | 2 | `delta[i]` | u16 LE, non-zero |

### LAYER_STATE

`payload_len = 8`

| Payload Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0` | 1 | `active_layer` | `0..31` |
| `1..3` | 3 | `reserved` | must be zero |
| `4..7` | 4 | `layer_mask` | u32 LE |

`layer_mask != 0` の場合、`active_layer` の bit が立っていなければ reject します。host は表示用途にのみ使い、`APP_LAYER` として echo back しません。

### KEY_PRESS

`payload_len = 2`

| Payload Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0` | 1 | `position` | key position |
| `1` | 1 | `flags` | bit0 = `PRESSED`, others zero |

## Capability Bits

| Bit | Name | Meaning |
| ---: | --- | --- |
| 0 | `APP_LAYER` | app layer set / clear |
| 1 | `TIME_SYNC` | time sync |
| 2 | `AI_USAGE` | AI usage snapshot |
| 3 | `THEME` | reserved theme support |
| 4 | `BATTERY` | `BATTERY_STATUS` uplink |
| 5 | `HOST_ACTION` | `HOST_ACTION` uplink |
| 6 | `KEY_STATS` | `KEY_STATS` uplink |
| 7 | `LAYER_STATE` | `LAYER_STATE` uplink |
| 8 | `KEY_PRESS` | `KEY_PRESS` uplink |
| 9 | `CONFIG_RPC` | reserved Config RPC support |

Keylink Studio は capability を機能 gate として扱います。`BATTERY_STATUS` は既存互換のため capability 未広告でも表示対象として受ける場合がありますが、その他の uplink は該当 capability がない device からは破棄します。

## Host Receive Policy

HELLO 待機中に `DEVICE_HELLO` 以外の packet が届いた場合、host は `UplinkPacket` として完全に decode できる既存 uplink packet だけを pending queue に退避します。v1 packet、reserved non-zero、unused payload non-zero、unsupported typed decode、`CONFIG_RESPONSE` は debug log に残して破棄します。
