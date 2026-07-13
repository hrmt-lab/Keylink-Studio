# Host Link v2 and Config RPC Packet Specification

この仕様は、Keylink Studio と ZMK firmware の間で使う Host Link v2 packetと、Encoder割り当ておよびCombo tableを表示・変更・保存するためのConfig RPCを定義します。

この仕様は Host Link protocol version `0x02` を前提にします。過去互換性は考慮しません。

## Scope

Host Link v2 は全 packet を 64 byte 固定へ移行する。`CONFIG_REQUEST` / `CONFIG_RESPONSE` だけを例外扱いにはしない。

MVP では Host Link v1 との後方互換は持たない。Host Link Config RPCによるEncoder / Combo編集はHost Link protocol v2対応firmwareを必須とする。Host は v2 handshake に成功しない device へ Config RPC を送信してはならない。v1 firmware は Host Link unsupported として扱う。ZMK Studio RPC による通常キー編集は Host Link transport と独立しているため、Studio 接続が有効な場合は Host Link v2 非対応でも継続して利用できる。

| Version | Packet size | Header | Status |
| --- | ---: | --- | --- |
| Host Link v1 | 32 bytes | packet type ごとの個別 layout | 旧仕様 |
| Host Link v2 | 64 bytes | 全 packet 共通 header | 新仕様 |

Host Link v2 では、`HOST_HELLO` / `DEVICE_HELLO` / `APP_LAYER` / `TIME_SYNC` / uplink packet / `CONFIG_REQUEST` / `CONFIG_RESPONSE` のすべてが、このドキュメントの Common Header を持つ。

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

Host が `hidapi` で write する buffer は、先頭 1 byte の report ID `0x00` と 64 byte の Host Link packet で構成する。

USB HID と BLE HOG は同じ Host Link packet format を使う。Transport 固有の制約は packet format へ持ち込まない。

## BLE Transport Policy

BLE HOG では、環境によって 64 byte report を 1 回で運べない場合がある。Host Link v2 の論理 packet size は 64 byte 固定のままにし、BLE で MTU が不足する場合、Report Map / characteristic が 64 byte report を許容しない場合、または 64 byte report が安定しない場合は BLE transport 層で chunking する。

```text
Host Link v2 packet:
  64 byte fixed packet

USB HID:
  1 HID report = 1 Host Link packet

BLE HOG:
  ATT MTU >= 67、Report Map / characteristic が 64 byte report を許容し、かつ実機で安定する場合は 1 BLE message = 1 Host Link packet
  ATT MTU < 67、Report Map / characteristic が 64 byte report を許容しない、または 64 byte transfer が不安定な場合は transport 層で分割送受信し、64 byte packet に復元してから Host Link parser へ渡す
```

64 byte の Host Link packet を BLE で 1 回送信するには、ATT header 3 byte を含めて `ATT MTU >= 67` が必要である。加えて、BLE HOG の Report Map と入出力 characteristic が 64 byte report を許容している必要がある。MTU が条件を満たしていても、OS / BLE stack / notify buffer の都合で不安定な場合は chunking へ fallback する。

chunking は Host Link packet parser の外側で行う。`packet_type`, `seq`, `feature`, `op` の意味は、復元後の 64 byte packet に対してのみ解釈する。chunk header / fragment index / reassembly timeout は BLE transport 層の仕様であり、Host Link packet 仕様には含めない。

## Common Header

全 packet は 64 byte 固定とする。

| Offset | Size | Field | Type | Value / Notes |
| --- | ---: | --- | --- | --- |
| `0..1` | 2 | `magic` | bytes | ASCII `"HL"` |
| `2` | 1 | `protocol_version` | u8 | `0x02` |
| `3` | 1 | `packet_type` | u8 | Packet Types |
| `4` | 1 | `seq` | u8 | request / response correlation |
| `5` | 1 | `feature` | u8 | `CONFIG_*` 以外では `0` |
| `6` | 1 | `op` | u8 | `CONFIG_*` 以外では `0` |
| `7` | 1 | `status_or_flags` | u8 | request/uplink では flags、response では status |
| `8` | 1 | `payload_len` | u8 | `0..=52` |
| `9..11` | 3 | `reserved` | bytes | must be zero |
| `12..63` | 52 | `payload` | bytes | unused bytes must be zero |

Validation:

- `magic` は必ず `"HL"`。
- `protocol_version` は必ず `0x02`。
- `payload_len` は `0..=52`。
- `reserved` は必ずすべて `0`。
- `payload_len` より後ろの payload byte は必ずすべて `0`。
- 受信側は reserved / unused payload に non-zero がある packet を `BAD_PACKET` として reject する。
- multi-byte integer はすべて little-endian。

## Packet Types

| Type | Name | Direction | Notes |
| ---: | --- | --- | --- |
| `0x01` | `HOST_HELLO` | Host -> Firmware | Host Link handshake |
| `0x02` | `DEVICE_HELLO` | Firmware -> Host | Host Link handshake response |
| `0x03` | `ERROR` | Firmware -> Host | Generic error |
| `0x04` | `PING` | Host -> Firmware | Reserved |
| `0x05` | `PONG` | Firmware -> Host | Reserved |
| `0x10` | `AI_USAGE` | Host -> Firmware | Existing feature |
| `0x20` | `TIME_SYNC` | Host -> Firmware | Existing feature |
| `0x30` | `APP_LAYER` | Host -> Firmware | Existing feature |
| `0x40` | `BATTERY_STATUS` | Firmware -> Host | Existing uplink |
| `0x50` | `HOST_ACTION` | Firmware -> Host | Existing uplink |
| `0x60` | `KEY_STATS` | Firmware -> Host | Existing uplink |
| `0x70` | `LAYER_STATE` | Firmware -> Host | Existing uplink |
| `0x80` | `KEY_PRESS` | Firmware -> Host | Existing uplink |
| `0x90` | `CONFIG_REQUEST` | Host -> Firmware | Config RPC request |
| `0x91` | `CONFIG_RESPONSE` | Firmware -> Host | Config RPC response |

Host Link v2 の既存機能 packet は、v1 の payload byte offset をそのまま継承しない。必ず Common Header の `payload` 領域へ再配置する。

例:

```text
v1 APP_LAYER:
  byte 4 action
  byte 5 layer
  byte 7 seq

v2 APP_LAYER:
  byte 4 seq
  byte 8 payload_len
  byte 12 payload[0] action
  byte 13 payload[1] layer
```

### Existing Packet Payload Layouts

既存 packet は v1 の意味を維持し、v2 では Common Header の `payload` 領域へ詰め直す。`CONFIG_REQUEST` / `CONFIG_RESPONSE` 以外の既存 packet は、特記がない限り `feature = 0`, `op = 0`, `status_or_flags = 0` とする。`seq` は Common Header の `seq` を使う。

#### HOST_HELLO

| Header Field | Value |
| --- | --- |
| `packet_type` | `HOST_HELLO` (`0x01`) |
| `payload_len` | `0` |

Host は `seq` に handshake seq を入れる。

#### DEVICE_HELLO

`payload_len = 12`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0..3` | 4 | `capabilities` | u32 LE | Capability Bits |
| `4..11` | 8 | `device_uid_hash` | u64 LE | `0` は identity unavailable |

Firmware は `HOST_HELLO` と同じ `seq` を返す。v2 では過去互換性を考慮しないため、v1 の `protocol_min` / `protocol_max` は持たない。protocol version は Common Header の `protocol_version = 0x02` で固定する。

#### PING / PONG

`PING` と `PONG` は `payload_len = 0` とする。`PONG` は `PING` と同じ `seq` を返す。

#### APP_LAYER

`payload_len = 2`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0` | 1 | `action` | u8 | `1=set`, `2=clear` |
| `1` | 1 | `layer` | u8 | set は `0..31`, clear は `0` |

Validation:

- `action = 1` のとき `layer` は `0..31`。
- `action = 2` のとき `layer` は `0`。
- unknown action は `INVALID_ARGUMENT` 相当として reject する。

#### TIME_SYNC

`payload_len = 9`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0..3` | 4 | `unix_time_sec` | u32 LE | Unix time |
| `4..5` | 2 | `tz_offset_min` | i16 LE | Time zone offset minutes |
| `6` | 1 | `weekday` | u8 | ISO weekday, `1=Mon .. 7=Sun` |
| `7` | 1 | `format_hint` | u8 | Display format preset |
| `8` | 1 | `clock_mode` | u8 | `0=24h`, `1=12h` |

#### AI_USAGE

`payload_len = 19`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0` | 1 | `provider` | u8 | `1=codex`, `2=claude_code` |
| `1` | 1 | `flags` | u8 | AI usage flags |
| `2..3` | 2 | `five_hour_used_bp` | u16 LE | 使用率 x 100 |
| `4..5` | 2 | `seven_day_used_bp` | u16 LE | 使用率 x 100 |
| `6..9` | 4 | `five_hour_reset_unix` | u32 LE | Unix time or `0` |
| `10..13` | 4 | `seven_day_reset_unix` | u32 LE | Unix time or `0` |
| `14..17` | 4 | `updated_unix` | u32 LE | Unix time |
| `18` | 1 | `error_code` | u8 | AI usage error code |

#### BATTERY_STATUS

`payload_len = 1 + 2 * count`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0` | 1 | `count` | u8 | `1..=4` |
| `1 + 2i` | 1 | `source[i]` | u8 | `0=central/self`, `1..=3=peripheral` |
| `2 + 2i` | 1 | `level[i]` | u8 | `0..=100`, `0xFF=unknown` |

v2 では `seq` が Common Header にあるため、`BATTERY_STATUS` も `seq` を持つ。ただし Host は gap warning などの厳密な欠落検出を必須とはせず、重複抑制やデバッグ用途として扱ってよい。

#### HOST_ACTION

`payload_len = 2`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0` | 1 | `action_id` | u8 | Host config が意味を定義 |
| `1` | 1 | `value` | u8 | action argument |

#### KEY_STATS

`payload_len = 4 + 3 * entry_count`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0` | 1 | `entry_count` | u8 | `1..=8` |
| `1` | 1 | `flags` | u8 | bit0=`MORE_FOLLOWS`, others zero |
| `2..3` | 2 | `reserved` | bytes | must be zero |
| `4 + 3i` | 1 | `position[i]` | u8 | key position |
| `5 + 3i .. 6 + 3i` | 2 | `delta[i]` | u16 LE | non-zero key press delta |

#### LAYER_STATE

`payload_len = 8`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0` | 1 | `active_layer` | u8 | top active layer, `0..31` |
| `1..3` | 3 | `reserved` | bytes | must be zero |
| `4..7` | 4 | `layer_mask` | u32 LE | bit i = layer i active |

#### KEY_PRESS

`payload_len = 2`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0` | 1 | `position` | u8 | key position |
| `1` | 1 | `flags` | u8 | bit0=`PRESSED`, others zero |

#### ERROR

`ERROR` は診断用の best-effort packet とする。`magic`, `protocol_version`, `payload_len` など、response を構成する前提が壊れている packet は silent drop してよい。

`payload_len = 2`

| Header / Payload | Field | Notes |
| --- | --- | --- |
| Header `status_or_flags` | `status` | Status Codes |
| Payload `0` | `offending_packet_type` | 不明なら `0` |
| Payload `1` | `offending_seq` | 不明なら `0` |

## Capability Bits

`DEVICE_HELLO` は capability bit に Config RPC 対応を含める。

| Bit | Name | Meaning |
| ---: | --- | --- |
| 9 | `CONFIG_RPC` | `CONFIG_REQUEST` / `CONFIG_RESPONSE` 対応 |

Host は `CONFIG_RPC` capability を持たない device に Config RPC を送ってはいけない。

Config RPC feature ごとの対応有無は、Host が対象 feature の `GET_INFO` を probing して判定する。Host は Host Link v2 handshake と `CONFIG_RPC` capability を確認した後、`CONFIG_REQUEST feature=ENCODER op=GET_INFO` を送る。`OK` なら `ENCODER` feature 対応、`UNSUPPORTED_FEATURE` または `UNSUPPORTED_OP` なら非対応として扱う。`GET_INFO.encoder_count == 0` の場合は、`ENCODER` feature には対応しているが表示対象 encoder がない状態として扱う。

## Request / Response Rules

- Host は同一 device に対して未完了の `CONFIG_REQUEST` を同時に 1 件だけ持つ。
- Firmware は `CONFIG_REQUEST` ごとに必ず `CONFIG_RESPONSE` を 1 件返す。
- `CONFIG_RESPONSE` は request と同じ `seq`, `feature`, `op` を返す。
- Host は `seq` が一致しない `CONFIG_RESPONSE` を現在 request の応答として扱わない。
- Host は timeout した場合、同じ `seq` で最大 1 回 retry してよい。
### Host Receive Dispatch Rules

Host は Host Link v2 の受信 packet を、`packet_type` に基づいて dispatcher へ配送する。

Host が Config RPC の pending request を持っている場合でも、Firmware から `BATTERY_STATUS`、`HOST_ACTION`、`KEY_STATS`、`LAYER_STATE`、`KEY_PRESS` などの uplink packet が届く可能性がある。これらの uplink packet は pending request の response ではないため、Host は破棄してはならない。Host は通常の uplink packet として処理し、pending Config RPC request は継続する。

`CONFIG_RESPONSE` を受信した場合のみ、Host は現在の pending request と `seq`、`feature`、`op` を照合する。

```text
受信 packet:

  packet_type == CONFIG_RESPONSE:
    pending request が存在し、
    seq / feature / op が一致する場合:
      pending request の response として完了する

    pending request が存在しない、
    または seq / feature / op が一致しない場合:
      stale / unknown CONFIG_RESPONSE として扱う
      pending request は完了しない
      debug log に記録して破棄する

  packet_type != CONFIG_RESPONSE:
    通常の Host Link packet として処理する
    pending Config RPC request は完了しない
```

Host は、pending request 中に uplink packet を受信したことを timeout / retry の失敗条件として扱ってはならない。timeout は、対応する `CONFIG_RESPONSE` が期限内に届かなかった場合にのみ発生する。

- Firmware の duplicate response cache / retry 判定は短寿命とし、同一 pending / retry window 内だけ有効とする。
- cache 有効中に、同じ `seq`, `feature`, `op`, `payload_len`, payload bytes の `CONFIG_REQUEST` を受けた場合、直前 request の retry とみなし、可能なら直前 response を再送する。
- 非同期処理中など、同じ request が pending だが response がまだ生成されていない場合、Firmware は同一 request の retry に追加 response を返さず、最初の処理完了時に同じ `seq`, `feature`, `op` の response を返してよい。
- cache 有効中に、同じ `seq` だが `feature`, `op`, `payload_len`, payload bytes のいずれかが異なる `CONFIG_REQUEST` を受けた場合は `BAD_PACKET` を返してよい。
- cache が無効または期限切れの場合、同じ `seq` であっても新規 request として扱う。seq wrap 後の正当な request を、過去 request との `seq` 一致だけで reject してはならない。
- Host は `CONFIG_RESPONSE` の `status_or_flags != OK` を失敗として扱う。
- `CONFIG_REQUEST` の `status_or_flags` は flags として扱う。MVP では必ず `0`。
- `CONFIG_RESPONSE` の `status_or_flags` は status として扱う。

## Status Codes

| Value | Name | Meaning |
| ---: | --- | --- |
| `0x00` | `OK` | 成功 |
| `0x01` | `BAD_PACKET` | header / length / reserved / payload validation failure |
| `0x02` | `UNSUPPORTED_FEATURE` | feature 未対応 |
| `0x03` | `UNSUPPORTED_OP` | op 未対応 |
| `0x04` | `INVALID_ARGUMENT` | layer / encoder / binding などが不正 |
| `0x05` | `BUSY` | Firmware が処理中 |
| `0x06` | `NOT_FOUND` | 対象が存在しない |
| `0x07` | `STORAGE_ERROR` | settings / NVS 保存、削除、または読み込み失敗 |
| `0x08` | `INTERNAL_ERROR` | その他の内部エラー |

Status が `OK` 以外の場合、payload は op ごとの通常 response payload ではない。MVP では `payload_len = 0` とする。

## Features

| Value | Name | Notes |
| ---: | --- | --- |
| `0x01` | `ENCODER` | エンコーダ binding の表示・変更・保存 |
| `0x02` | `COMBO` | Combo table の表示・変更・保存。packet仕様は本書で確定、production実装はPhase 2B以降 |
| `0x03` | `TAP_DANCE` | Future reserved |

未知の feature は `UNSUPPORTED_FEATURE` を返す。

## Config RPC Future Extension Rules

この節は、`COMBO` / `TAP_DANCE` などの Config RPC feature を追加するときに忘れてはいけない共通ルールを定義する。`COMBO` は本書でpacket仕様を確定済み、`TAP_DANCE` はfuture reservedである。

### Payload Size and Logical Data Split

Config RPC の payload は Common Header の制約により最大 52 byte とする。52 byte を超える論理データを扱う future feature は、BLE transport chunking に依存してはならない。

`COMBO` は52 byte固定itemを1 request / responseへ格納する。`TAP_DANCE` など52 byteを超えるfuture featureは、feature固有のpagination、item単位 `GET` / `SET`、またはmulti-packet transactionを別途定義する。

BLE chunking は 64 byte Host Link packet を transport 層で復元するための仕組みであり、Config RPC の論理データ分割、item paging、transaction 境界を表現しない。

### Common Lifecycle Operations

Config RPC の op namespace は feature ごとに独立する。ただし、編集可能な Config feature では lifecycle op 番号を共通予約する。

| Op | Name | Meaning |
| ---: | --- | --- |
| `0x01` | `GET_INFO` | 対象 feature の情報取得 |
| `0x04` | `GET_DIRTY` | 対象 feature の未保存変更確認 |
| `0x05` | `SAVE` | 対象 feature の保存 |
| `0x06` | `DISCARD` | 対象 feature の破棄 |

これらの lifecycle op の payload layout と feature 固有の詳細は各 feature 仕様で定義する。

`ENCODER` feature では MVP として `GET_INFO` / `GET_DIRTY` / `SAVE` / `DISCARD` を必須とする。将来の `COMBO` / `TAP_DANCE` feature でも、編集・保存・破棄を持つ場合は同じ lifecycle op 番号を使う。

## Encoder Operations

| Op | Name | Direction | Notes |
| ---: | --- | --- | --- |
| `0x01` | `GET_INFO` | Host -> Firmware | layer 数、encoder 数、capability を取得 |
| `0x02` | `GET_BINDINGS` | Host -> Firmware | layer / encoder の CW / CCW を取得 |
| `0x03` | `SET_BINDINGS` | Host -> Firmware | layer / encoder の CW / CCW を即時反映 |
| `0x04` | `GET_DIRTY` | Host -> Firmware | 未保存変更の有無を取得 |
| `0x05` | `SAVE` | Host -> Firmware | 未保存変更を settings / NVS へ保存 |
| `0x06` | `DISCARD` | Host -> Firmware | 未保存変更を破棄 |
| `0x07` | `CLEAR_OVERRIDE` | Host -> Firmware | 指定 encoder の runtime override をクリア |

未知の op は `UNSUPPORTED_OP` を返す。

## Encoder Binding Format

Host Link Config RPC の encoder binding は、ZMK の sensor-bindings に置く sensor behavior ではなく、encoder event の方向判定後に実行される CW / CCW 方向ごとの通常 behavior binding である。

Encoder binding は 10 byte 固定とする。

| Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0..1` | 2 | `behavior_id` | u16 LE | ZMK `zmk_behavior_local_id_t` |
| `2..5` | 4 | `param1` | u32 LE | ZMK behavior parameter 1 |
| `6..9` | 4 | `param2` | u32 LE | ZMK behavior parameter 2 |

通常キーの raw binding と同じ意味で扱う。

`behavior_id` は Keylink 独自の値ではなく、ZMK 本体の `zmk_behavior_local_id_t` に合わせる。`&kp`, `&none`, `&trans` などの `behavior_id` を Host Link 仕様上の固定値として定義してはならない。

Encoder binding は、ZMK の `sensor-bindings` に記述する sensor behavior そのものではない。Host Link Config RPC では、CW / CCW それぞれに対して、通常キーと同じ形式の ZMK behavior binding を 1 つずつ保持する。

例えば `.keymap` の通常 ZMK 設定では以下のように sensor behavior を使う。

```dts
sensor-bindings = <&inc_dec_kp C_VOL_UP C_VOL_DN>;
```

一方、Host Link Config RPC の override では、`&inc_dec_kp` 自体を送るのではなく、Firmware が encoder event の方向を判定した後に実行する direction-specific binding を送る。

```text
CW  direction -> cw_binding  を通常 behavior binding として実行
CCW direction -> ccw_binding を通常 behavior binding として実行
```

したがって、`cw_binding` / `ccw_binding` の `behavior_id`, `param1`, `param2` は、detent ごとに実行される通常 behavior binding を表す。これらは `.keymap` の `sensor-bindings` 配列要素を直接表現するものではない。

| Value | Meaning |
| ---: | --- |
| `0xFFFF` | invalid / not found |

Host は `behavior_id` を UI 側で固定値として組み立てない。Rust core が ZMK Studio RPC の `list_all_behaviors()` と `get_behavior_details(id)` から接続ごとに作成した resolver を使い、UI の `EditBehavior` を `behavior_id`, `param1`, `param2` へ変換する。

resolver は `display_name` だけを正として逆引きしてはならない。wire に送る `behavior_id` は `list_all_behaviors()` で取得した id を正とする。resolver はメモリ上にのみ保持し、永続キャッシュしない。切断、接続先変更、アプリ終了時に破棄し、再接続時は ZMK Studio RPC から再取得する。

`zmk-studio-api 0.3.1` では `StudioClient::list_all_behaviors()` と `StudioClient::get_behavior_details(id)` が公開されており、既存の `ensure_behavior_catalog()` もこれらを使って通常キー編集用の role/id map を構築している。Host は catalog fingerprint や behavior identity hash を生成・保存しない。Host 側 resolver は、接続中の ZMK Studio RPC behavior catalog から `EditBehavior` を `behavior_id` / `param1` / `param2` へ変換するためだけに使う一時状態とする。

resolver が作成できない場合、または選択された `EditBehavior` を解決できない場合、Host は `SET_BINDINGS` を送ってはならない。

`behavior_id` は同一 firmware build 内で有効な local id として扱う。永続 ID としては扱わない。Firmware 更新後は、保存済み override の各 binding について `behavior_id` が現在も同じ identity の behavior を指しているかを Firmware 側で再検証する。保存済み override から `0xFFFF` または解決不能な behavior id が見つかった場合、その override は stale saved override として扱い、runtime override へ反映しない。以後の `GET_BINDINGS` は `source = KEYMAP` と `flags.STALE_SAVED_EXISTS = 1` を返す。

### Behavior Identity Hash

Firmware は catalog 全体の hash ではなく、保存する binding ごとの `behavior_identity_hash` を扱う。`behavior_identity_hash` は `behavior_id` そのものではなく、現在の firmware 上でその `behavior_id` が指している behavior の identity を表す hash とする。

hash 入力は Firmware が安定して取得できる情報だけに限定する。ZMK Studio RPC の `display_name` や metadata と完全一致させることを要求しない。

record version 1 の MVP hash 入力:

- identity schema version
- behavior device name / label
- binding cell count

ZMK behavior runtime API から任意 behavior device の `compatible` 文字列を安定取得する標準経路は使わない。record version 1 では `compatible_or_kind` を必須入力にせず、`behavior_device_name + binding_cell_count` を MVP の identity とする。

record version 1 の hash 長は 16 byte とする。将来、firmware 側で安定した behavior kind、role、Keylink eligibility class を定義して hash 入力へ追加する場合は、settings record の `record_version` を上げ、互換性ルールを別途定義する。

record version 1 の `behavior_identity_hash` は、canonical identity input に対する SHA-256 digest の先頭 16 byte とする。

```text
behavior_identity_hash = SHA-256(canonical_identity_input)[0..16]
```

上記の範囲表記は半開区間であり、SHA-256 digest の byte offset `0..15` の 16 byte を保存することを表す。

`canonical_identity_input` は、firmware が安定して取得できる behavior identity 情報を、key order と field order を固定した byte sequence として構築する。文字列は UTF-8 とし、各 field は長さ付きで encode する。実装依存の pointer address、device instance address、build ごとに変わる一時値は hash 入力に含めてはならない。

record version 1 の推奨 field order は以下とする。

```text
identity_schema_version
behavior_device_name_or_label
binding_cell_count
behavior_role_or_empty
keylink_eligibility_class_or_empty
```

record version 1 では `behavior_role_or_empty` と `keylink_eligibility_class_or_empty` は空文字列として encode する。hash algorithm または canonical input 形式を変更する場合は、settings record の `record_version` を上げ、互換性ルールを別途定義する。

settings / NVS に保存する encoder override record は、以下の key / value 形式とする。

Settings key:

```text
keylink/enc/v1/l%04x/e%04x
```

`layer_id` と `encoder_id` は lowercase hex とし、`%04x` は最低 4 桁の zero padding を表す。値が 4 桁を超える場合は切り詰めない。

Settings value:

```text
offset  size  field
0       2     magic = "KE"
2       1     record_version = 1
3       1     flags = 0
4       1     hash_len = 16
5       3     reserved = 0
8       16    cw_behavior_identity_hash
24      16    ccw_behavior_identity_hash
40      10    cw_binding
50      10    ccw_binding
60      4     crc32 u32 LE
```

Total size は 64 byte 固定とする。`cw_binding` / `ccw_binding` は Config RPC の `EncoderBinding` と同じ 10 byte layout を使い、multi-byte field は little-endian とする。record 内の multi-byte integer も little-endian とする。

CRC32 は value の先頭から `crc32` field 直前までの 60 byte に対して計算する。

record version 1 の `crc32` は CRC-32/ISO-HDLC、別名 CRC-32/IEEE 802.3 を使う。

```text
width      = 32
poly       = 0x04C11DB7
init       = 0xFFFFFFFF
refin      = true
refout     = true
xorout     = 0xFFFFFFFF
check      = 0xCBF43926  // ASCII "123456789"
```

CRC32 は settings record の偶発的な破損検出を目的とする。security boundary としては扱わない。identity の一致判定には `behavior_identity_hash` を使う。

末尾の `crc32` 自身は CRC 計算に含めない。

```text
crc_target = value[0..60]
crc32      = value[60..64]
```

上記の範囲表記は半開区間であり、`crc32` は byte offset `60..63` の 4 byte を表す。

`flags` は record version 1 では must be zero とする。non-zero の場合、Firmware は unsupported record として読み込まず、runtime override へ反映しない。

CW と CCW で別 behavior を使えるため、`behavior_identity_hash` も方向ごとに別々に保存する。各 binding には診断用に `behavior_role` / `behavior_name` を保存しない。診断情報が必要な場合は、record version を上げて互換性ルールを定義してから追加する。

起動時または saved override 読み込み時:

- value length が 64 byte であることを確認する。
- `magic == "KE"`、`record_version == 1`、`flags == 0`、`hash_len == 16`、`reserved[0..3] == 0` を確認する。
- CRC32 が一致することを確認する。
- `cw_binding.behavior_id != 0xFFFF`、`ccw_binding.behavior_id != 0xFFFF` を確認する。
- saved override の CW / CCW それぞれについて、保存済み `behavior_id` が現在の firmware 上で解決できるか確認する。
- 現在の firmware がその `behavior_id` から算出した `behavior_identity_hash` と保存済み hash を比較する。
- `param1` / `param2` が現在の behavior に対して妥当か検証する。behavior 種別を判定できる場合は、transparent など明らかに encoder override 非対応の binding を stale 扱いにしてよい。
- CW / CCW の両方が検証に成功した場合、その override を runtime override として有効化してよい。
- record 形式不正、CRC 不一致、unsupported record は invalid saved override として扱い、runtime override へ反映しない。該当 layer / encoder の実行時動作は通常 ZMK の `sensor-bindings` へ fallback する。
- 解決不能、identity 不一致、param 不正、または現在は encoder override 非対応になった binding を含む override は stale saved override として実行対象外にし、runtime override へ反映しない。該当 layer / encoder の実行時動作は通常 ZMK の `sensor-bindings` へ fallback する。
- stale / invalid saved override は `GET_BINDINGS` で `OVERRIDE` として返さず、通常の binding 表示上は `source = KEYMAP` を返す。ただし Host / UI が診断できるよう、`GET_BINDINGS` response の `flags.stale_saved_exists` または `flags.invalid_saved_exists` を true にする。
- MVP では stale / invalid saved override に対応する `GET_BINDINGS` の `cw_binding` / `ccw_binding` は全 byte `0` とする。Host / UI はこれを `&none` として表示せず、「`.keymap` の設定を使用中」と表示する。
- `stale_saved_exists` / `invalid_saved_exists` は dirty とは別概念とし、この flag が true であることだけでは `GET_DIRTY` の `dirty` を true にしない。
- stale / invalid saved override は起動時に自動削除しない。同じ layer / encoder に対してユーザー操作により runtime state が dirty になり、`SAVE` が成功した場合にのみ整理する。runtime state が `OVERRIDE` の場合は stale / invalid saved override を新しい saved override で置換し、runtime state が `KEYMAP` の場合は stale / invalid saved override を削除する。

### Special Behavior Values

`0xFFFF` は invalid / not found 専用の sentinel とする。

- Host は `behavior_id = 0xFFFF` を `SET_BINDINGS` で送ってはならない。
- Firmware は `SET_BINDINGS` で `behavior_id = 0xFFFF` を受け取った場合、`INVALID_ARGUMENT` を返す。
- Firmware は `behavior_id = 0xFFFF` を settings / NVS へ保存してはならない。
- Firmware は `behavior_id = 0xFFFF` を実行してはならない。
- Firmware が保存済み override 読み込み時に `behavior_id = 0xFFFF` または解決不能な behavior id を見つけた場合、その override は stale saved override として扱い、runtime override へ反映しない。起動時には自動削除しない。

`&none` 相当は有効な binding として許可する。これは「何もしない」という明示的な Studio override であり、実行時は通常 ZMK の `sensor-bindings` へ `BUBBLE` しない。ただし `&none` の `behavior_id` は固定値ではなく、Host が behavior catalog から `None` role として解決した local id を送る。

`&trans` 相当は MVP ではエンコーダ override として非対応とする。`&trans` の `behavior_id` も固定値として扱わない。

- Host は `EditBehavior::Transparent` をエンコーダ override 用の `EncoderBinding` に解決して送ってはならない。
- Firmware は `SET_BINDINGS` で受け取った binding が transparent behavior と判定できる場合、`INVALID_ARGUMENT` を返す。
- `&trans` を「firmware default へ戻す」操作として扱ってはならない。

firmware default へ戻す操作は `CLEAR_OVERRIDE` で表現する。`CLEAR_OVERRIDE` は Keylink Studio の runtime override を削除し、override がない状態へ戻す。override がない encoder event は zmk-rawhid-app では処理せず、通常 ZMK の `sensor-bindings` へ `BUBBLE` する。削除の永続化は `SAVE` で行う。

### Encoder Behavior Eligibility

MVP の encoder override は、detent ごとの tap-like action のみ許可する。hold/release、tap-hold 判定、sticky state、layer state mutation、device control を伴う behavior は encoder binding として扱わない。

MVP では encoder behavior eligibility の詳細判定は Host 側を主責務とする。Host は非対応 behavior を Binding Picker 上で選択不可にし、`EncoderBinding` へ変換せず、`SET_BINDINGS` を送信しない。

Firmware は `SET_BINDINGS` 受信時に最低限の安全検証を行う。Firmware は `layer_id` / `encoder_id` / `update_mask` / `reserved` / `behavior_id` / binding parameter を検証し、壊れた値、解決不能な `behavior_id`、invalid sentinel、不正な parameter を拒否する。ZMK behavior metadata が無く `zmk_behavior_validate_binding()` が `-ENODEV` を返す場合でも、`zmk_behavior_get_binding()` で behavior device 自体が存在するなら parameter を検証不能として受理してよい。metadata があり `-EINVAL` になる parameter 不正は従来どおり拒否する。Firmware は behavior 種別を判定できる場合に限り、transparent など明らかに非対応の binding を `INVALID_ARGUMENT` として拒否してよい。ただし MVP では Bluetooth command 種別や layer 系 behavior などの詳細分類を Firmware 側の必須実装とはしない。

MVP で許可する behavior:

- `KeyPress` のうち modifier-only ではない tap-like key
- `None`
- `Bluetooth` の select 系 command
- `OutputSelection`
- `MouseKeyPress`
- `MouseMove`
- `MouseScroll`

MVP で非対応とする behavior:

- `KeyPress` のうち modifier-only key
- `Transparent`
- `MomentaryLayer`
- `ToggleLayer`
- `ToLayer`
- `ModTap`
- `LayerTap`
- `StickyKey`
- `StickyLayer`
- `Bluetooth` の clear 系 command
- `Bluetooth` の disconnect 系 command
- `CapsWord`
- `KeyRepeat`
- `Reset`
- `Bootloader`
- `StudioUnlock`
- `GraveEscape`

Host は Bluetooth behavior の command 種別まで検証する。select 系 command だけを許可し、clear 系または disconnect 系 command は非対応とする。Firmware 側で Bluetooth command 種別まで分類できる場合は同じ制限で拒否してよいが、MVP では Firmware 側の必須実装とはしない。

Modifier-only key は、Left/Right Ctrl、Shift、Alt、GUI など、単独では修飾状態だけを表す key を指す。MVP では encoder detent を tap-like action として扱うため、modifier-only key は実用性が低く、押下/解放状態が残ったように見えるリスクを避けるため非対応とする。通常キー、consumer key、media key などの tap-like usage は許可してよい。

`zmk,behavior-input-two-axis` (`&mmv` / `&msc`) は press 中の経過時間に基づき周期 tick で relative input を生成する behavior である。encoder detent で `press -> 即 release` の tap-like invoke を行うと経過時間がほぼ 0 になり、移動量やスクロール量が出ない。

Firmware は encoder override 実行時、binding が `zmk,behavior-input-two-axis` の behavior device を指す場合に限り、通常の `zmk_behavior_invoke_binding()` を使わず、detent ごとの discrete relative input event をその behavior device から `input_report_rel()` で送ってよい。この場合も `&mmv` / `&msc` 用の既存 `zmk,input-listener` と input processor pipeline を通るため、新しい devicetree node は不要である。

MVP の discrete 量は以下とする。

- Raw sensor steps はそのまま detent と見なさず、4 steps を 1 detent として扱う。override が存在する encoder では、1 detent に満たない raw event は Firmware が `HANDLED` として消費し、標準 `sensor-bindings` へ fallback させない。
- `INPUT_REL_WHEEL` / `INPUT_REL_HWHEEL`: detent を蓄積し、2 detent あたり `+1` または `-1` notch
- `INPUT_REL_X` / `INPUT_REL_Y`: `MOVE_*` の packed 値を `20` で割った値
- 1 つの sensor event に複数 detents 相当の raw steps が含まれる場合は、その detent 数だけ実行する

この量は実機チューニング対象であり、必要になった場合は Kconfig 化する。

## ENCODER GET_INFO

### Request

| Header Field | Value |
| --- | --- |
| `packet_type` | `CONFIG_REQUEST` (`0x90`) |
| `feature` | `ENCODER` (`0x01`) |
| `op` | `GET_INFO` (`0x01`) |
| `status_or_flags` | `0` |
| `payload_len` | `0` |

### Response Payload

`payload_len = 4`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0` | 1 | `layer_count` | u8 | Editable layer count. `layer_id` の有効範囲ではない |
| `1` | 1 | `encoder_count` | u8 | Physical encoder slot count. 全 layer 共通 |
| `2` | 1 | `capabilities` | u8 | Encoder capability bits |
| `3` | 1 | `reserved` | u8 | must be zero |

`encoder_count` は `zmk,keymap-sensors` に登録された sensor 要素数から算出し、全 layer 共通とする。有効な `encoder_id` は `0..encoder_count-1` である。`encoder_id` は `zmk,keymap-sensors` の sensor 配列順 index とする。layer ごとの `sensor-bindings` 数や有無から `encoder_count` を算出してはならない。layer ごとに通常 ZMK の `sensor-bindings` が存在しない場合でも、その `encoder_id` は Host Link Config RPC 上は有効とする。

MVP では、`zmk,keymap-sensors` に登録された sensor はすべて encoder 編集対象として扱う。対象 keyboard の `zmk,keymap-sensors` には rotary encoder のみを含める前提とする。rotary encoder 以外の sensor が含まれている場合でも、MVP の Host Link Config RPC 上は encoder として扱われる。Firmware は sensor 種別による filtering や `encoder_id -> sensor_index` の再マッピングを行わない。

保存済み override の lookup key は `layer_id` と `encoder_id` を使うため、Firmware 更新で `zmk,keymap-sensors` の順序を変えると override が別 sensor に対応する可能性がある。Firmware / keymap author は `zmk,keymap-sensors` の順序を安定させる必要がある。

将来 rotary encoder 以外の sensor を同じ `zmk,keymap-sensors` に含めて編集対象から除外したくなった場合は、MVP の仕様を拡張し、sensor 種別 filtering、`encoder_id -> sensor_index` mapping、保存済み override の migration / invalidation ルールを別途定義する。

Encoder capability bits は将来の optional 拡張用に残す。MVP では必須 op の有無判定には使わない。

| Bit | Name | Meaning |
| ---: | --- | --- |
| 0 | `DIRTY` | 将来用。MVP では `GET_DIRTY` は必須 |
| 1 | `SAVE` | 将来用。MVP では `SAVE` は必須 |
| 2 | `DISCARD` | 将来用。MVP では `DISCARD` は必須 |

MVP では、`ENCODER` feature を持つ firmware は `GET_INFO` / `GET_BINDINGS` / `SET_BINDINGS` / `GET_DIRTY` / `SAVE` / `DISCARD` / `CLEAR_OVERRIDE` をすべて必須 op として実装する。Host は `ENCODER` feature があるなら、これらの op が使える前提で扱う。MVP firmware は `DIRTY` / `SAVE` / `DISCARD` bit を `1` にしてよいが、Host はこれらの bit を必須 op 判定に使わない。

`CLEAR_OVERRIDE` は firmware default へ戻す主要操作であるため、個別 capability bit は持たせない。ただし `CLEAR_OVERRIDE` は保存済み override を即時削除しない。対象 layer / encoder に runtime override、saved override、stale saved override、invalid saved override のいずれかが存在する場合だけ dirty にする。永続削除は `SAVE`、復元は `DISCARD` で行う。削除対象が何も存在しない場合は no-op として `OK` を返し、dirty は false のままとする。

## Encoder Layer Identity

Encoder Config RPC の `layer_id` は ZMK Studio RPC の `Layer.id` と同じ意味を持つ。UI 上のレイヤー配列 index やタブ順ではない。

```text
layer_id:
  ZMK Studio RPC get_keymap の layers[].id
  レイヤー順に依存しない識別子
  Host 側で 0, 1, 2... と独自生成しない

layer index:
  UI 表示やレイヤー並び替えで使う現在の位置
  Host Link Config RPC の layer_id には使わない
```

Host は ZMK Studio RPC `get_keymap` で `layers[]` を取得し、対象レイヤーの `Layer.id` を `GET_BINDINGS` / `SET_BINDINGS` / `CLEAR_OVERRIDE` の `layer_id` として送る。`GET_INFO.layer_count` は編集対象レイヤー数を示すだけで、`0..layer_count-1` が有効な `layer_id` であることを意味しない。

Firmware は override lookup key と settings / NVS 保存キーに `layer_id` を使う。Firmware 側の validation は ZMK keymap の有効な layer id かどうかで判定する。保存済み override の `layer_id` が現在の ZMK keymap に存在しない場合、その override は実行対象外として無視し、可能なら削除対象にする。

## ENCODER GET_BINDINGS

### Request Payload

`payload_len = 5`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0..3` | 4 | `layer_id` | u32 LE | ZMK Studio RPC `Layer.id` |
| `4` | 1 | `encoder_id` | u8 | `0..encoder_count-1` |

### Response Payload

`payload_len = 28`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0..3` | 4 | `layer_id` | u32 LE | request と同じ値 |
| `4` | 1 | `encoder_id` | u8 | request と同じ値 |
| `5` | 1 | `source` | u8 | Encoder binding source |
| `6` | 1 | `flags` | u8 | Encoder binding flags |
| `7` | 1 | `reserved` | u8 | must be zero |
| `8..17` | 10 | `cw_binding` | EncoderBinding | CW |
| `18..27` | 10 | `ccw_binding` | EncoderBinding | CCW |

Encoder binding source:

| Value | Name | Meaning |
| ---: | --- | --- |
| `0x00` | `KEYMAP` | Studio override は存在しない。実動作は `.keymap` の通常 ZMK `sensor-bindings` に従う |
| `0x01` | `OVERRIDE` | Studio override が存在し、`cw_binding` / `ccw_binding` が実動作に使われる |

Encoder binding flags:

| Bit | Name | Meaning |
| ---: | --- | --- |
| 0 | `STALE_SAVED_EXISTS` | 保存済み override は存在するが stale と判定され、runtime override へ反映されていない |
| 1 | `SAVED_EXISTS` | 現在有効な saved override が存在する |
| 2 | `RUNTIME_DIRTY` | 対象 layer / encoder の runtime state が saved state と異なる |
| 3 | `INVALID_SAVED_EXISTS` | 保存済み record は存在するが、record 形式不正、CRC 不一致、unsupported record などにより runtime override へ反映されていない |

未定義 bit は必ず `0` とする。

GET_BINDINGS は runtime state を最優先して返す。

対象 layer / encoder に runtime override が存在する場合、Firmware は `source = OVERRIDE` を返し、`cw_binding` / `ccw_binding` には現在の runtime override を返す。

この状態では、同じ layer / encoder の settings / NVS 上に stale saved override または invalid saved override が残っていても、MVP では `flags.STALE_SAVED_EXISTS` / `flags.INVALID_SAVED_EXISTS` を返さない。これらの stale / invalid saved override は、次回 `SAVE` 成功時に新しい saved override で置換される cleanup 対象として内部的に保持する。

対象 layer / encoder に runtime override が存在しない場合、Firmware は saved state / diagnostic state に基づいて `source` と `flags` を返す。

- valid saved override が存在する場合:
  - `source = OVERRIDE`
  - `cw_binding` / `ccw_binding` に saved override 由来の runtime override を返す
  - `flags.SAVED_EXISTS = 1`

- stale saved override のみが存在する場合:
  - `source = KEYMAP`
  - `cw_binding` / `ccw_binding` は全 byte `0`
  - `flags.STALE_SAVED_EXISTS = 1`

- invalid saved override のみが存在する場合:
  - `source = KEYMAP`
  - `cw_binding` / `ccw_binding` は全 byte `0`
  - `flags.INVALID_SAVED_EXISTS = 1`

- override / saved diagnostic が何も存在しない場合:
  - `source = KEYMAP`
  - `cw_binding` / `ccw_binding` は全 byte `0`
  - diagnostic flags はすべて `0`

`flags.RUNTIME_DIRTY` は、対象 layer / encoder の runtime state が saved state と異なる場合に true とする。runtime override が存在し、未保存の変更がある場合は `flags.RUNTIME_DIRTY = 1` とする。

`source = KEYMAP` の場合、Firmware は `.keymap` の `sensor-bindings` を通常キー binding へ変換して返さない。`cw_binding` / `ccw_binding` は全 byte `0` とする。Host / UI はこれを `&none` として表示してはいけない。UI は「`.keymap` の設定を使用中」または「Studio 未設定」として表示する。

`source = KEYMAP` かつ `flags.STALE_SAVED_EXISTS = 1` の場合、保存済み override は stale として実行対象外であり、実動作は通常 ZMK の `.keymap` `sensor-bindings` へ fallback する。この flag は診断用であり、dirty とは別概念である。Host / UI はこの flag だけで保存バーを表示してはならない。

`source = KEYMAP` かつ `flags.INVALID_SAVED_EXISTS = 1` の場合、settings / NVS 上に保存済み record は存在するが、record 形式不正、CRC 不一致、unsupported record などにより読み込めていない。実動作は通常 ZMK の `.keymap` `sensor-bindings` へ fallback する。この flag も診断用であり、dirty とは別概念である。

`flags.SAVED_EXISTS` は、対象 layer / encoder に現在有効な saved override が存在することを表す。`source = OVERRIDE` かつ `flags.SAVED_EXISTS = 1`、`flags.RUNTIME_DIRTY = 0` の場合、その override は保存済み clean 状態である。

`flags.RUNTIME_DIRTY` は、対象 layer / encoder 単位で runtime state が saved state と異なることを表す。`GET_DIRTY` は feature 全体の未保存変更有無を返す lifecycle op であり、`GET_BINDINGS` の `RUNTIME_DIRTY` は表示・診断用の per-binding flag として扱う。Host / UI は保存バー表示の主判定を `encoderDirty` / `GET_DIRTY` に置き、`STALE_SAVED_EXISTS` / `INVALID_SAVED_EXISTS` 単独では保存バーを表示してはならない。

代表的な状態:

| 状態 | source | flags |
| --- | --- | --- |
| `KEYMAP`, clean | `KEYMAP` | `0` |
| `KEYMAP`, stale saved override あり | `KEYMAP` | `STALE_SAVED_EXISTS` |
| `KEYMAP`, invalid saved record あり | `KEYMAP` | `INVALID_SAVED_EXISTS` |
| `OVERRIDE`, saved clean | `OVERRIDE` | `SAVED_EXISTS` |
| `OVERRIDE`, runtime dirty, saved override なし | `OVERRIDE` | `RUNTIME_DIRTY` |
| `OVERRIDE`, saved override あり、runtime 変更済み | `OVERRIDE` | `SAVED_EXISTS | RUNTIME_DIRTY` |

layer ごとに通常 ZMK の `sensor-bindings` が存在しない場合でも、対象 `encoder_id` が `0..encoder_count-1` の範囲内であれば `GET_BINDINGS` は `NOT_FOUND` / `INVALID_ARGUMENT` にせず、Studio override が存在しない状態として `source = KEYMAP` を返す。

`source = KEYMAP` の encoder を初回編集する場合、Host は現在の CW / CCW 具体値を知らない。Host は単方向変更を即時送信してはならない。UI は Studio override 作成フローとして CW / CCW 両方の明示設定を要求し、両方向が設定されるまで `SET_BINDINGS` を送らない。未設定側を Host が暗黙に `&none` として補完してはならない。初回 override 作成中にユーザーがキャンセルした場合、Host は何も送信せず、対象 encoder は `source = KEYMAP` のままにする。

`source = OVERRIDE` の場合、`cw_binding` / `ccw_binding` は settings / NVS 保存済み、または未保存 runtime override の現在値を表す。

Validation:

- `layer_id` が ZMK keymap の有効な layer id でない場合は `INVALID_ARGUMENT`。
- `encoder_id >= encoder_count` は `INVALID_ARGUMENT`。
- layer ごとの `sensor-bindings` 欠落は `NOT_FOUND` / `INVALID_ARGUMENT` にしない。

## ENCODER SET_BINDINGS

### Request Payload

`payload_len = 28`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0..3` | 4 | `layer_id` | u32 LE | ZMK Studio RPC `Layer.id` |
| `4` | 1 | `encoder_id` | u8 | `0..encoder_count-1` |
| `5` | 1 | `update_mask` | u8 | MVP では `0x03` 固定 |
| `6..7` | 2 | `reserved` | bytes | must be zero |
| `8..17` | 10 | `cw_binding` | EncoderBinding | CW |
| `18..27` | 10 | `ccw_binding` | EncoderBinding | CCW |

MVP では Host は `update_mask = 0x03` のみ送る。`SET_BINDINGS` は常に CW / CCW 両方の `EncoderBinding` を含める。Firmware は CW / CCW の両方に最低限の安全検証を行い、両方が有効な場合だけ両方を runtime override に反映する。片方でも不正な場合はどちらも反映せず、`INVALID_ARGUMENT` を返す。

既存 `source = OVERRIDE` の編集では、Host は最新の楽観 snapshot から CW / CCW 両方の payload を組み立てる。`source = KEYMAP` からの初回 override 作成では、Host は `.keymap` 側の具体値を知らないため、CW / CCW 両方が明示設定されるまで `SET_BINDINGS` を送ってはならない。

部分更新は MVP では扱わない。将来必要になった場合に、masked-out 側の保持・検証・競合処理を別途仕様化して追加する。

Validation:

- `layer_id` が ZMK keymap の有効な layer id でない場合は `INVALID_ARGUMENT`。
- `encoder_id >= encoder_count` は `INVALID_ARGUMENT`。
- layer ごとの `sensor-bindings` 欠落は `NOT_FOUND` / `INVALID_ARGUMENT` にしない。
- `update_mask != 0x03` は `INVALID_ARGUMENT`。
- `reserved` が non-zero の場合は `BAD_PACKET`。
- 対象 `behavior_id` が解決不能、invalid sentinel、または parameter が firmware 側で不正な場合は `INVALID_ARGUMENT`。
- Firmware が behavior 種別を判定でき、transparent など明らかに encoder override 非対応と判断できる場合は `INVALID_ARGUMENT` を返してよい。Bluetooth command 種別や layer 系 behavior などの詳細分類は Host 側を主責務とし、MVP firmware の必須実装とはしない。

### Response Payload

`payload_len = 0`

成功時、Firmware は runtime table を即時更新し、encoder dirty state を true にする。

`SET_BINDINGS` 成功後、その layer / encoder の `GET_BINDINGS` は `source = OVERRIDE` と、現在の runtime override を返す。保存前の runtime state が saved state と異なる場合、`flags.RUNTIME_DIRTY` を true にする。

同じ layer / encoder に valid saved override が存在する場合は `flags.SAVED_EXISTS` を true にしてよい。

同じ layer / encoder に stale saved override または invalid saved override が存在する状態で `SET_BINDINGS` が成功した場合、Firmware はそれらを次回 `SAVE` 成功時に新しい saved override で置換する cleanup 対象として内部的に保持する。ただし、MVP では `source = OVERRIDE` を返している間、`GET_BINDINGS` の `flags.STALE_SAVED_EXISTS` / `flags.INVALID_SAVED_EXISTS` は true にしない。

`SAVE` 成功後、その layer / encoder の stale / invalid saved override は新しい saved override で置換され、`flags.RUNTIME_DIRTY` は false になる。`DISCARD` した場合は runtime override を破棄し、SET_BINDINGS 前の saved / stale / invalid 状態へ戻す。

## ENCODER GET_DIRTY

### Request

| Header Field | Value |
| --- | --- |
| `packet_type` | `CONFIG_REQUEST` (`0x90`) |
| `feature` | `ENCODER` (`0x01`) |
| `op` | `GET_DIRTY` (`0x04`) |
| `payload_len` | `0` |

### Response Payload

`payload_len = 1`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0` | 1 | `dirty` | u8 | `0=false`, `1=true` |

Validation:

- `dirty` は `0` または `1` のみ。

## ENCODER SAVE

### Request

| Header Field | Value |
| --- | --- |
| `packet_type` | `CONFIG_REQUEST` (`0x90`) |
| `feature` | `ENCODER` (`0x01`) |
| `op` | `SAVE` (`0x05`) |
| `payload_len` | `0` |

### Response

`payload_len = 0`

`SAVE` は、Firmware が管理している dirty layer / encoder の現在の runtime state を settings / NVS の saved state へ同期する。

- dirty layer / encoder の runtime state が `OVERRIDE`:
  - 現在の CW / CCW override の `behavior_id` / `param1` / `param2` と、CW / CCW それぞれの現在の `behavior_identity_hash` を settings / NVS へ保存する。
  - 保存 value は `keylink/enc/v1/l%04x/e%04x` key に 64 byte record として書き込む。
  - 同じ layer / encoder の既存 saved override がある場合は上書きする。
  - 同じ layer / encoder の stale / invalid saved override がある場合も、新しい saved override で置換する。
- dirty layer / encoder の runtime state が `KEYMAP`:
  - 同じ layer / encoder の saved override が settings / NVS に存在する場合は削除する。
  - 同じ layer / encoder の stale / invalid saved override がある場合も削除する。
  - これは `CLEAR_OVERRIDE` 後の永続削除、または stale override の削除を表す。
  - 削除後、その layer / encoder の `GET_BINDINGS` は `source = KEYMAP` を返す。

settings subsystem / NVS 全体の読み込みに失敗している、または settings handler の commit 完了を確認できておらず保存済み override の存在有無を安全に判断できない状態では、Firmware は destructive な削除を行ってはならず、`STORAGE_ERROR` を返す。

ただし、個別 record を読み込めており、その record が形式不正、CRC 不一致、unsupported record、identity 不一致、解決不能 behavior、param 不正などにより stale / invalid saved override と判定できる場合は、この限りではない。dirty layer / encoder の runtime state が `OVERRIDE` の場合は新しい saved override で置換してよく、runtime state が `KEYMAP` の場合は削除してよい。

dirty 対象の全 entry に対する保存、上書き、削除がすべて成功した場合のみ、Firmware は `OK` を返し、encoder dirty state を false にする。

1 つでも保存、上書き、削除に失敗した場合、Firmware は `STORAGE_ERROR` を返す。settings / NVS は transaction を持たないため、`STORAGE_ERROR` 時でも一部 entry が保存、上書き、削除済みになっている可能性がある。この場合、Firmware は encoder dirty state を false にしてはならない。dirty 対象と `delete_pending` を保持し、次回 `SAVE` で dirty 対象を再度保存または削除する。

`CLEAR_OVERRIDE` 後の `SAVE` では、対象 layer / encoder の saved override または stale / invalid saved override を削除する。削除に失敗した場合も `STORAGE_ERROR` を返し、encoder dirty state と `delete_pending` を保持する。

stale / invalid saved override は起動時または `GET_BINDINGS` 時に自動削除しない。将来、明示的な cleanup operation を追加して stale / invalid saved override を一括削除できるようにしてよい。

Failure:

- 保存領域不足、settings 書き込み失敗、settings 削除失敗は `STORAGE_ERROR`。
- settings subsystem / NVS 全体の読み込み失敗により、保存済み override の存在有無を安全に判断できない場合は `STORAGE_ERROR`。
- 個別 record が stale / invalid saved override と判定できているだけの場合は、それ自体を `STORAGE_ERROR` とはしない。ただし、その entry の保存、上書き、削除に失敗した場合は `STORAGE_ERROR`。
- 保存機能が無効な firmware は `UNSUPPORTED_OP`。

## ENCODER DISCARD

### Request

| Header Field | Value |
| --- | --- |
| `packet_type` | `CONFIG_REQUEST` (`0x90`) |
| `feature` | `ENCODER` (`0x01`) |
| `op` | `DISCARD` (`0x06`) |
| `payload_len` | `0` |

### Response

`payload_len = 0`

成功時、Firmware は runtime override を settings / NVS 保存値へ戻す。保存値が存在しない binding は override なしへ戻す。encoder dirty state は false にする。

保存済み override が存在しない encoder は override なしへ戻す。この場合、以後の `GET_BINDINGS` は `source = KEYMAP` を返し、実動作は通常 ZMK の `.keymap` `sensor-bindings` へ戻る。

## ENCODER CLEAR_OVERRIDE

### Request Payload

`payload_len = 5`

| Payload Offset | Size | Field | Type | Notes |
| --- | ---: | --- | --- | --- |
| `0..3` | 4 | `layer_id` | u32 LE | ZMK Studio RPC `Layer.id` |
| `4` | 1 | `encoder_id` | u8 | `0..encoder_count-1` |

MVP では `CLEAR_OVERRIDE` は指定 layer / encoder の CW / CCW 両方を Studio override なしの状態へ戻す操作とする。runtime override が存在する場合は削除する。saved override、stale saved override、または invalid saved override が存在する場合は `SAVE` 時の削除対象として dirty cleanup 対象にする。saved override / stale saved override / invalid saved override は `CLEAR_OVERRIDE` 時点では削除しない。CW だけ、または CCW だけを `.keymap` に戻す部分削除は MVP では扱わない。

### Response

`payload_len = 0`

成功時、対象 layer / encoder に runtime override、saved override、stale saved override、invalid saved override のいずれかが存在する場合、Firmware は指定 layer / encoder の runtime override を削除し、encoder dirty state を true にする。以後の `GET_BINDINGS` は `source = KEYMAP` を返し、実行時は通常 ZMK の `.keymap` `sensor-bindings` へ `BUBBLE` する。saved override、stale saved override、または invalid saved override が存在する場合、それらは次回 `SAVE` 成功時に削除される。

対象 layer / encoder に runtime override、saved override、stale saved override、invalid saved override のいずれも存在しない場合、`CLEAR_OVERRIDE` は `OK` を返すが状態を変更しない。この場合 encoder dirty state は false のままとする。

`CLEAR_OVERRIDE` は `SET_BINDINGS` と同じ保存モデルに従う。`SAVE` 前に `DISCARD` した場合は、saved override があれば runtime override に復元し、saved override がなければ override なしへ戻す。`SAVE` 時点で runtime が override なしの場合、Firmware は saved override / stale saved override / invalid saved override を settings / NVS から削除する。

Validation:

- `layer_id` が ZMK keymap の有効な layer id でない場合は `INVALID_ARGUMENT`。
- `encoder_id >= encoder_count` は `INVALID_ARGUMENT`。
- layer ごとの `sensor-bindings` 欠落は `NOT_FOUND` / `INVALID_ARGUMENT` にしない。

## Sequence Examples

### Initial Load

```text
Host -> Firmware:
  CONFIG_REQUEST seq=10 feature=ENCODER op=GET_INFO payload_len=0

Firmware -> Host:
  CONFIG_RESPONSE seq=10 feature=ENCODER op=GET_INFO status=OK
  payload: layer_count=4 encoder_count=2 capabilities=0x07

Host -> Firmware:
  CONFIG_REQUEST seq=11 feature=ENCODER op=GET_BINDINGS
  payload: layer_id=<ZMK Studio Layer.id> encoder_id=0

Firmware -> Host:
  CONFIG_RESPONSE seq=11 feature=ENCODER op=GET_BINDINGS status=OK
  payload: layer_id=<same Layer.id> encoder_id=0 source=KEYMAP flags=0 zeroed cw_binding zeroed ccw_binding

Host UI:
  Encoder 0 は ".keymap の設定を使用中" と表示する
```

### Edit Encoder

```text
Host -> Firmware:
  CONFIG_REQUEST seq=20 feature=ENCODER op=SET_BINDINGS
  payload:
    layer_id=<ZMK Studio Layer.id>
    encoder_id=0
    update_mask=0x03
    cw_binding={behavior_id,param1,param2}
    ccw_binding={behavior_id,param1,param2}

Firmware -> Host:
  CONFIG_RESPONSE seq=20 feature=ENCODER op=SET_BINDINGS status=OK payload_len=0
```

### Save

```text
Host -> Firmware:
  CONFIG_REQUEST seq=30 feature=ENCODER op=SAVE payload_len=0

Firmware -> Host:
  CONFIG_RESPONSE seq=30 feature=ENCODER op=SAVE status=OK payload_len=0
```

## Implementation Notes

- Host 側は Config RPC response を通常 uplink packet と同じ read loop から受け取り、`seq` で pending request へ配送する。
- Config RPC pending 中に他の uplink packet が届いた場合、Host は捨てずに通常 uplink として処理する。
- Firmware 側は Config RPC request 処理を短く保つ。settings / NVS 保存は `SAVE` の時だけ行う。
- `SET_BINDINGS` は RAM 上の runtime override だけを変更する。
- `CLEAR_OVERRIDE` は RAM 上の runtime override を削除し、saved override / stale saved override / invalid saved override がある場合は `SAVE` 時の削除対象として dirty cleanup 対象にする。settings / NVS の saved override / stale saved override / invalid saved override は `SAVE` まで削除しない。削除対象が何もない場合は no-op とし、dirty にしない。
- `source = KEYMAP` からの初回 override 作成は、CW / CCW 両方が揃った時点で初めて `SET_BINDINGS` を送る。キャンセル時は Host Link request を送らず、Firmware state は変更しない。
- Host 側 UI は `SET_BINDINGS` 成功後に表示を確定する。失敗時は楽観更新を戻すか再読み込みする。
- Host 側 Rust core は、通常キー用の `EditBehavior -> zmk-studio-api::Behavior` 変換とは別に、エンコーダ用の `EditBehavior -> EncoderBinding` 変換を持つ。
- UI / Binding Picker は `behavior_id` を直接扱わない。
- BLE HOG では USB HID より timeout を長めにする。

## Combo Config RPC

この節は `COMBO = 0x02` のpacket仕様と永続化契約を定義する。2026-07-13時点でfirmwareの全operation、2,064 byte table decoder、Settings load／保存、valid／stale／invalid診断、native fixture、Cornix mutation／永続化実機確認まで完了している。Host core／CLI／TauriとKeymap Viewer UIも全operationへ対応し、CLI mutationとUI read-only表示まで確認済みである。UIからの完全なmutation／永続化手順とCombo Export／Restoreが残っている。

### Operations

| Op | Name | Request payload_len | Success response payload_len | Runtime変更 | Dirty変更 |
| ---: | --- | ---: | ---: | --- | --- |
| `0x01` | `GET_INFO` | 0 | 16 | なし | なし |
| `0x02` | `GET_COMBO` | 1 | 52 | なし | なし |
| `0x03` | `SET_COMBO` | 52 | 0 | 1 slotをupsert | 必要に応じてslot dirtyを更新 |
| `0x04` | `GET_DIRTY` | 0 | 1 | なし | なし |
| `0x05` | `SAVE` | 0 | 0 | なし | 成功時のみdirty解除 |
| `0x06` | `DISCARD` | 0 | 0 | 保存済みtableまたはboot fallbackへ復元 | 成功時にdirty解除 |
| `0x07` | `DELETE_COMBO` | 1 | 0 | 指定slotを未使用化 | 必要に応じてslot dirtyを更新 |
| `0x08` | `RESET_TO_KEYMAP` | 0 | 0 | Devicetree defaultへ置換 | 保存状態との差分をdirty化 |

Phase 2Bで `GET_INFO` / `GET_COMBO`、Phase 2CでSettings loadとsaved診断、Phase 2Dで全mutation operationとdeferred SAVEを実装した。Hostは同じwire contractの型付きcodec／API／CLIを持ち、payload length、reserved、padding、canonical item、status、timeout／retry、response identityを自動テストする。

未知のopは `UNSUPPORTED_OP`。全requestでCommon Headerのrequest flagsは`0`でなければならず、non-zeroは `BAD_PACKET` とする。成功responseのpayload外byteはCommon Headerの規則どおり`0`で埋める。

`SET_COMBO` / `DELETE_COMBO` / `DISCARD` / `RESET_TO_KEYMAP` は `rawhid_app_combo_runtime_idle() == false` の場合 `BUSY`。`GET_INFO` / `GET_COMBO` / `GET_DIRTY` はキー押下中も実行できる。`SAVE` はruntime定義を変えないためキー押下中も実行できる。

同一pending / retry windowの重複requestは共通Config RPC cache規則に従う。各Combo op自体も次のとおり冪等にする。

- `SET_COMBO`: 同じslotへ同じitemを再適用しても同じruntimeとdirtyになる。
- `DELETE_COMBO`: 既に空のslotでも `OK` のno-opとする。response消失後のretryを `NOT_FOUND` にしない。
- `DISCARD` / `RESET_TO_KEYMAP`: 同じ操作の再実行結果は同じ。
- `SAVE`: dirtyがなければ `OK` のno-op。単一table imageは同じruntimeから同じbyte列になるため再試行可能。

`SAVE` が非同期実行中の場合、同一requestのretryには追加処理を開始しない。それ以外のCombo mutationと、同じscratch bufferを必要とするsaved diagnosticの読出しは `BUSY` としてよい。

### COMBO GET_INFO

Requestは `payload_len = 0`。それ以外は `BAD_PACKET`。

Success responseは `payload_len = 16`。

| Offset | Size | Field | Type | Meaning |
| ---: | ---: | --- | --- | --- |
| `0` | 1 | `max_combos` | u8 | runtime slot上限。現在32 |
| `1` | 1 | `max_keys_per_combo` | u8 | 1 Comboのキー上限。現在8 |
| `2` | 1 | `combo_count` | u8 | `occupied_slots` のpopcount |
| `3` | 1 | `flags` | u8 | table / Settings診断flags |
| `4..7` | 4 | `occupied_slots` | u32 LE | 現在実行可能なruntime Combo。bit i = slot i |
| `8..11` | 4 | `stale_slots` | u32 LE | 読み取り可能だが現在のfirmwareで実行不能なsaved slot |
| `12..15` | 4 | `invalid_slots` | u32 LE | slot recordの構造またはCRCが不正なsaved slot |

`flags`:

| Bit | Name | Meaning |
| ---: | --- | --- |
| 0 | `SAVED_TABLE_LOADED` | 有効なtable metadataを持つsaved tableをbaselineとして読み込んだ |
| 1 | `TABLE_METADATA_INVALID_FALLBACK` | length / magic / header field / header CRC不正によりDevicetree defaultへfallbackした |
| 2 | `TABLE_VERSION_UNSUPPORTED_FALLBACK` | 認識可能なheaderだが保存形式version非対応のためdefaultへfallbackした |
| 3 | `STORAGE_READ_ERROR_FALLBACK` | Settings read失敗によりdefaultへfallbackした |
| 4 | `STORAGE_UNAVAILABLE` | firmwareにCombo Settings保存機能がない |
| 5..7 | reserved | must be zero |

Settings keyが存在しない初回bootは正常なDevicetree default状態であり、fallback異常flagを立てない。metadata fallback中の `occupied_slots` / `combo_count` は、実際に実行中のDevicetree default runtimeを返す。

Hostが一覧問い合わせするslot集合は `occupied_slots | stale_slots | invalid_slots`。`occupied_slots`は実行可能性、`stale_slots` / `invalid_slots`はsaved診断であり、互いに同時に立つ場合がある。たとえばstale saved recordを `SET_COMBO` で修正したが未保存の場合、そのslotはruntime上occupiedかつsaved診断上staleである。

`stale_slots` と `invalid_slots` は同じsaved recordについて同時に立てず、structurally readableならstale、信頼できなければinvalidを優先する。

`GET_COMBO` はruntime occupiedまたはstale slotのitemを返せる。invalid slotは信頼できるitemを構成できないため `NOT_FOUND` とし、Hostは `invalid_slots` からplaceholderと修正／削除操作を提供する。これにより追加問い合わせなしで列挙対象と警告対象を判断できる。

将来fieldを追加する場合はresponse `payload_len`を増やす。現versionの16 byte responseにreserved tailは置かず、Hostは未知の追加tailを無視できるdecoderにする。

### Combo Item

`GET_COMBO` success responseと `SET_COMBO` requestは同じ52 byte itemを使う。packed C structへcastせず、offsetごとの明示的LE encode / decodeを行う。

| Offset | Size | Field | Type | Validation / Meaning |
| ---: | ---: | --- | --- | --- |
| `0` | 1 | `slot_id` | u8 | `0..max_combos-1` |
| `1` | 1 | `key_count_flags` | u8 | bits 0..3 key_count、bit 4 slow_release、bits 5..7 zero |
| `2..17` | 16 | `name` | bytes | 1..15 byte ASCII + NUL。NUL後はzero |
| `18..33` | 16 | `key_positions[8]` | 8 x u16 LE | 使用分はposition、未使用は`0xFFFF` |
| `34..43` | 10 | `binding` | ComboBinding | 1 behavior binding |
| `44..47` | 4 | `layer_mask` | u32 LE | `0` = all layers |
| `48..49` | 2 | `timeout_ms` | u16 LE | `1..1000` |
| `50..51` | 2 | `require_prior_idle_ms` | u16 LE | `0xFFFF` = disabled、または`1..1000` |

`key_count_flags`:

| Bit | Meaning |
| ---: | --- |
| `0..3` | raw `key_count`。有効値2..8 |
| `4` | `slow_release` (`0=false`, `1=true`) |
| `5..7` | reserved、must be zero |

未定義bitがnon-zeroなら `BAD_PACKET`。key_countが2..8外なら `INVALID_ARGUMENT`。

`name` はUTF-8 byte列を運ぶ領域だが、MVP内部modelの使用可能文字が `a-zA-Z0-9,._+-` に限定されるため、wireでも同じsingle-byte ASCIIだけを受理する。最初のNULまでが名前で、長さ1..15 byte。NULがない、禁止文字、NUL後のnon-zero、case-insensitive重複は `INVALID_ARGUMENT`。

`ComboBinding` はEncoderBindingと同じ10 byte layout:

| Binding Offset | Size | Field | Type |
| ---: | ---: | --- | --- |
| `0..1` | 2 | `behavior_id` | u16 LE `zmk_behavior_local_id_t` |
| `2..5` | 4 | `param1` | u32 LE、ZMK parameter raw bits |
| `6..9` | 4 | `param2` | u32 LE、ZMK parameter raw bits |

`param1` / `param2` はwire上unsigned 32-bitとし、behavior固有metadataがsigned値として解釈する場合もraw bit patternを保持する。`behavior_id = 0xFFFF`、解決不能behavior、metadataがある状態でparameter validationが失敗するbindingは `INVALID_ARGUMENT`。`zmk_behavior_validate_binding()` が `-ENODEV` でもbehavior device自体を解決できる場合は、metadataなしでparameter検証不能としてEncoderと同様に受理する。

使用中key positionはfirmwareの有効範囲内かつ相互に重複不可。受信順は任意だが、validation前に一時model上で昇順へ正規化し、GET / runtime / Settingsは常に昇順で返す。未使用entryはすべて `0xFFFF` でなければ `INVALID_ARGUMENT`。

non-zero `layer_mask` は現在のfirmwareに存在するlayer indexのbitだけを含める。範囲外bitは `INVALID_ARGUMENT`。`0`はall layersであり「layerなし」ではない。

item内にsaved / stale / invalid flagは置かない。診断sourceは `GET_INFO` maskを正とし、52 byte itemをmodelだけに固定する。実装時はoffset定数から `COMBO_ITEM_LEN == 52` および `COMBO_ITEM_LEN == CONFIG_RPC_MAX_PAYLOAD` をcompile-time assertionする。

### COMBO GET_COMBO

Request `payload_len = 1`:

| Offset | Size | Field | Type |
| ---: | ---: | --- | --- |
| `0` | 1 | `slot_id` | u8 |

- slot範囲外: `INVALID_ARGUMENT`
- runtime occupied: 現在のruntime itemを `OK`, payload_len 52で返す
- runtime未使用かつsaved stale: CRC検証済みraw saved itemを返す。current validationを通らない値を含み得るためHostは `stale_slots` と併せて扱う
- invalid savedのみ、または完全な空slot: `NOT_FOUND`
- stale itemの再読出しにSettings readが必要で失敗: `STORAGE_ERROR`
- runtime itemをencodeできない内部不変条件違反: `INTERNAL_ERROR`

### COMBO SET_COMBO

Request `payload_len = 52`。success response `payload_len = 0`。

`slot_id`はupsert先であり、追加／更新を表す別flagは設けない。

- runtime occupied slotへのSETは更新。
- runtime empty slotへのSETは追加。
- stale / invalid saved slotへのSETは、そのslotのruntime修復。saved診断はSAVE成功まで残る。
- 32 slotすべてoccupiedの場合も、occupied slotの更新は可能。空きslotが存在しないため「追加先」は存在せず、Hostは `occupied_slots` から追加UIを無効化する。`slot_id = 32` や`0xFF`をauto-allocationとして扱わず `INVALID_ARGUMENT`。

正規化後、nameのcase-insensitive一意性、同一key setとlayer集合の重複をtable全体で検証する。layer mask `0`はall layersとして他方と必ず重なる。完全分離layerの同一key set、包含key set、partial overlapは許可する。競合は `INVALID_ARGUMENT`。

validation完了前はruntimeを変更しない。成功時のみslotを原子的に置換し、そのslotのruntimeが有効saved baselineと一致しなければdirtyにする。saved baselineを正確に比較できないdiagnostic / read-error状態では、安全側としてdirtyを維持する。

### COMBO GET_DIRTY

Request `payload_len = 0`。success response `payload_len = 1`。

| Offset | Size | Field | Type |
| ---: | ---: | --- | --- |
| `0` | 1 | `dirty` | u8、`0`または`1` |

`dirty = (dirty_slots != 0 || metadata_repair_pending)`。stale / invalid maskやfallback flagだけではdirtyにしない。

### COMBO SAVE

Request / success responseとも `payload_len = 0`。

- dirtyなし: `OK` no-op。
- Combo Settings非対応: `UNSUPPORTED_OP`。
- Settings subsystem未commit、baseline再読出し失敗、NVS write失敗、容量不足: `STORAGE_ERROR`。
- serialization不変条件違反: `INTERNAL_ERROR`。

version 1は個別Settings keyを削除しないため `settings_delete()` pathを持たない。`DELETE_COMBO`はtable image内のtombstoneへ変換し、SAVEの単一table writeが失敗した場合に `STORAGE_ERROR` を返す。将来table key自体を削除するoperationを追加する場合、`settings_delete()`失敗も `STORAGE_ERROR` とする。

SAVEは現在のpersisted table imageをscratch bufferへ読み、dirty slotだけをruntime itemまたはtombstoneで置換する。cleanなstale / invalid slot recordはそのまま保持する。header / metadata fallback状態からの明示的なmutationまたは `RESET_TO_KEYMAP` 後のSAVEでは、Devicetree defaultから新しい完全table imageを構成する。

`settings_save_one()` が成功した場合だけdirtyと、書換対象slotのstale / invalid診断を解除する。失敗時はruntime編集内容、dirty slot mask、診断状態を維持して再試行可能にする。

### COMBO DISCARD

Request / success responseとも `payload_len = 0`。

- runtime non-idle: `BUSY`。
- 有効saved table: valid slotだけruntimeへ復元し、stale / invalid slotは実行しない。
- saved tableなし: Devicetree defaultへ復元。
- metadata invalid / unsupported / boot storage fallback: boot時に採用したDevicetree defaultへ復元し、診断flagsは維持。
- baseline tableの再読出しが必要だが失敗: `STORAGE_ERROR`。runtimeとdirtyを変更しない。

成功時はdirtyを解除する。診断状態は保存媒体の状態なのでDISCARDでは消さない。

### COMBO DELETE_COMBO

Request `payload_len = 1`:

| Offset | Size | Field | Type |
| ---: | ---: | --- | --- |
| `0` | 1 | `slot_id` | u8 |

success response `payload_len = 0`。範囲外は `INVALID_ARGUMENT`、runtime non-idleは `BUSY`。runtime / saved / stale / invalidのいずれかがあるslotを未使用化し、必要ならdirtyにする。完全な空slotは `OK` no-op。saved recordの物理削除はこの時点では行わず、SAVEでtombstoneを含むtable imageとして確定する。

### COMBO RESET_TO_KEYMAP

Request / success responseとも `payload_len = 0`。runtime non-idleは `BUSY`。

runtime全体を読み取り専用Devicetree default tableから再生成する。saved tableと同一ならcleanを維持できる。それ以外、またはmetadata repairが必要な場合は差分slot / metadataをdirtyにする。RESET自体はSettingsを書き換えず、続くSAVE成功でdefault tableを永続化する。

### Validation and Status Mapping

| Condition | Status |
| --- | --- |
| Common Header不正、payload_len不一致、request flags、reserved byte / undefined bit non-zero | `BAD_PACKET` |
| slot範囲外、name、key count、position、layer mask、timeout、prior idle不正 | `INVALID_ARGUMENT` |
| case-insensitive name重複、同一key set + overlapping layers | `INVALID_ARGUMENT` |
| behavior_id sentinel / 解決不能、behavior parameter不正 | `INVALID_ARGUMENT` |
| GET_COMBO対象が空またはinvalid recordだけ | `NOT_FOUND` |
| mutation時runtime non-idle、SAVE処理との競合 | `BUSY` |
| Settings read / write失敗、未commit、NVS容量不足 | `STORAGE_ERROR` |
| Combo Settingsを組み込まないfirmwareへのSAVE | `UNSUPPORTED_OP` |
| 未知のCombo op | `UNSUPPORTED_OP` |
| 内部encode不変条件、work submitなど予期しない内部失敗 | `INTERNAL_ERROR` |

table満杯専用statusは設けない。SETは明示slot upsertなので、全slot occupied時も更新可能であり、追加可能slotがないことは `GET_INFO.occupied_slots == 0xFFFFFFFF` で表現する。

## Combo Settings Format

### Adopted Persistence Model

version 1は、複数Settings keyのslot更新、A/B metadata、generation別slot keyを採用しない。table metadataと32個の固定slot recordを1つの2,064 byte table imageへ格納し、単一 `settings_save_one()` で保存する。

Settings namespace / key:

```text
namespace:          keylink/cmb/v1
table metadata key: keylink/cmb/v1/table
slot record key:    なし（table value内の固定offset recordを論理slot recordとする）
```

別々のslot keyを持たないのは意図的なversion 1の決定である。Zephyr Settingsには複数key transactionがなく、slot key群とmetadata keyを更新すると、途中失敗後のbootで新旧slotが混在し得る。A/B bank方式はこの問題を解決できるが、両bank候補のload / saved snapshot用RAMまたは二段階非同期loadが必要になり、6 KB目標とboot時のlistener安全性を悪化させる。

CornixのZephyr Settings backendはNVS、erase sectorは4,096 byte。NVSはdataを書いた後にATE metadataを書くため、2,064 byteの単一valueはsector内上限に収まり、電源断時は旧valueまたは新valueのどちらかだけが有効entryになる。現buildは `CONFIG_NVS_DATA_CRC=n` なので、application-level header / per-slot CRCを必須とする。

Phase 2のKconfig / build contractでは、Combo保存対応をNVS backendに限定し、table length 2,064 byteが `erase-block-size - 64 byte` 以下であることをbuild-time assertionする。64 byteはATE群、alignment、optional NVS data CRCのconservative marginである。条件を証明できないbackendではruntime Comboを利用できても `GET_INFO.STORAGE_UNAVAILABLE=1`、`SAVE=UNSUPPORTED_OP` とし、保存対応を黙って有効化しない。record version 1のままtableを分割してはならない。

### Table Header

table value lengthは2,064 byte固定。multi-byte integerはLE。

| Offset | Size | Field | Value / Meaning |
| ---: | ---: | --- | --- |
| `0..3` | 4 | `magic` | ASCII `KCMB` |
| `4` | 1 | `record_version` | `1` |
| `5` | 1 | `slot_record_len` | `64` |
| `6` | 1 | `slot_count` | `32` |
| `7` | 1 | `flags` | version 1は`0` |
| `8..11` | 4 | `occupied_mask` | u32 LE。saved tableで存在するslot |
| `12..15` | 4 | `header_crc32` | bytes `0..11` のCRC-32/ISO-HDLC |
| `16..2063` | 2048 | `slot_records[32]` | 64 byte x 32 |

header length、total length、最大キー数はversion 1の固定値から決まる。value lengthが2,064でない、magic / flags / slot length / slot count / header CRC不正はtable metadata invalid。認識可能なheaderでversionだけ非対応の場合はversion unsupported。

### Slot Record

slot `i` のrecord開始offset:

```text
table_offset = 16 + i * 64
```

record内offset:

| Offset | Size | Field | Meaning |
| ---: | ---: | --- | --- |
| `0` | 1 | `record_flags` | bit0 occupied、bits1..7 zero |
| `1` | 1 | `key_count_flags` | wire itemと同じ |
| `2..17` | 16 | `name` | wire itemと同じcanonical bytes |
| `18..33` | 16 | `key_positions[8]` | u16 LE、未使用`0xFFFF` |
| `34..43` | 10 | `binding` | ComboBinding |
| `44..47` | 4 | `layer_mask` | u32 LE |
| `48..49` | 2 | `timeout_ms` | u16 LE |
| `50..51` | 2 | `require_prior_idle_ms` | u16 LE、disabled=`0xFFFF` |
| `52..59` | 8 | `behavior_identity_hash` | SHA-256 identity digest先頭8 byte |
| `60..63` | 4 | `record_crc32` | record bytes `0..59` のCRC-32/ISO-HDLC |

slot IDはrecord位置から決まるため保存しない。magic / versionはtable headerで一括管理する。`occupied_mask`がslot存在の正本で、`record_flags.occupied`はslot単位の整合性検査に使う。不一致はそのslotだけinvalidとし、table全体をfallbackさせない。

unoccupied / deleted slotは `record_flags = 0`、bytes `1..59 = 0`、有効CRCとする。header `occupied_mask`も0。これがDevicetree default削除のtombstoneであり、boot時にそのslotへdefaultをmerge / 復活させない。

CRCはEncoder recordと同じCRC-32/ISO-HDLC（`crc32_ieee()`）を使う。header CRCはheader bytes `0..11`、slot CRCはslot bytes `0..59`だけを対象とし、CRC field自身を含めない。1 slotのCRC不一致は `invalid_slots` の該当bitだけを立て、他slotを維持する。

```text
width   = 32
poly    = 0x04C11DB7
init    = 0xFFFFFFFF
refin   = true
refout  = true
xorout  = 0xFFFFFFFF
check   = 0xCBF43926  // ASCII "123456789"
```

behavior identity canonical inputはEncoder record version 1と同じschemaを使う。

```text
identity_schema_version = 1
behavior_device_name_or_label (length-prefixed UTF-8)
binding_cell_count
behavior_role_or_empty = empty
keylink_eligibility_class_or_empty = empty
```

Combo version 1は32 recordを6 KB RAM目標内に収めるため、SHA-256 digest先頭8 byteを保存する。Encoderの16 byteからのCombo固有差分である。これはsecurity authenticationではなくfirmware更新後のidentity取り違え検出であり、64-bit collision riskを受容する。hash幅を変更する場合はCombo `record_version`を上げる。

### Boot Load

1. Phase 1CのDevicetree default tableをruntimeへ生成する。
2. Settings keyなしならdefaultをsaved baselineとしてcleanで確定する。
3. table valueを固定scratch bufferへ読む。read失敗ならdefaultを実行し `STORAGE_READ_ERROR_FALLBACK`。
4. header length / magic / version / fields / CRCを検証。metadata invalid / unsupportedならdefaultを実行し対応fallback flag。
5. metadata有効ならslot 0..31を順に検証する。
6. structural / CRC不正slotはinvalid、structurally readableだが現在のbehavior identity / resolve / parameter / key position / layer / duplicate validationを通らないslotはstaleとする。
7. valid slotだけruntimeへ登録する。stale / invalid slotへDevicetree defaultをmergeしない。
8. 同一key set + overlapping layerのsaved recordが複数ある場合はslot昇順で最初のvalid recordを採用し、後続競合slotをstaleとする。slot順をCombo発火優先順位には使わない。

### Save Atomicity

SAVEはscratch bufferへ現在のpersisted imageを読み、dirty slotだけを置換してheader / CRCを再計算し、`keylink/cmb/v1/table`へ1回だけ `settings_save_one()`する。

- write失敗がslot途中で起きても有効ATEが作られないため旧tableが残る。
- writeが実際にはcommit済みだがerror responseになった曖昧ケースでも、新旧slot混在は起きない。Firmwareはdirtyを保持し、retryまたはDISCARDでstorageを再読出しする。
- response消失後のretryで既にdirtyがclearなら `OK` no-op。同じbyte列を再保存してもNVSはduplicate writeを省略できる。
- clean stale / invalid recordはraw bytesを保持し、無関係なslot編集のSAVEで自動削除しない。
- dirty stale / invalid slotは新しいvalid recordまたはtombstoneで置換し、成功後にそのdiagnostic bitだけ解除する。

copy-on-write A/B metadata、commit marker、slot別keyはversion 1では却下する。単一key table imageが、Zephyr NVSで実装可能な最小RAMかつ最も強いtable整合性を与えるためである。

### Encoder Settingsとの共通点とCombo固有差分

共通点:

- Config RPC lifecycle op番号、10 byte binding、明示的LE encode / decodeを共有する。
- CRC-32/ISO-HDLC、SHA-256ベースbehavior identity schema、magic / record versionによる互換性判定を踏襲する。
- runtime dirtyとstale / invalid診断を分離し、SAVE失敗時はruntimeとdirtyを維持する。
- 解決不能behavior、identity不一致、parameter不正は実行しない。metadataなしbehaviorの扱いもEncoderと同じ。

Combo固有差分:

- Encoderはlayer / encoderごとの独立64 byte keyであり、entry間の原子性を必要としない。Comboは削除tombstoneと32 slot全体が1 tableの意味を持つため、2,064 byteの単一key imageにする。
- Comboはslot単位CRCをtable image内に持ち、1 slot破損を他slotから隔離する。
- ComboはRAM目標のためidentity hashを8 byteとし、Encoderの16 byteとは保存形式versionを分離する。
- Combo診断は32-bit stale / invalid masksで返し、52 byte itemへsaved stateを混在させない。

### Operation-specific Failure Status

| Operation | Possible non-OK status |
| --- | --- |
| `GET_INFO` | `BAD_PACKET`。runtime内部不変条件違反だけ `INTERNAL_ERROR` |
| `GET_COMBO` | `BAD_PACKET`, `INVALID_ARGUMENT`, `NOT_FOUND`, `STORAGE_ERROR`, `BUSY`（SAVE scratch競合時）, `INTERNAL_ERROR` |
| `SET_COMBO` | `BAD_PACKET`, `INVALID_ARGUMENT`, `BUSY`, `INTERNAL_ERROR` |
| `GET_DIRTY` | `BAD_PACKET` |
| `SAVE` | `BAD_PACKET`, `BUSY`, `STORAGE_ERROR`, `UNSUPPORTED_OP`, `INTERNAL_ERROR` |
| `DISCARD` | `BAD_PACKET`, `BUSY`, `STORAGE_ERROR`, `INTERNAL_ERROR` |
| `DELETE_COMBO` | `BAD_PACKET`, `INVALID_ARGUMENT`, `BUSY` |
| `RESET_TO_KEYMAP` | `BAD_PACKET`, `BUSY`, `INTERNAL_ERROR` |

## Combo Saved State Classification

| State | Runtime実行 | GET_INFO | GET_COMBO | Dirty | SET / DELETE + SAVE | DISCARD | RESET_TO_KEYMAP |
| --- | --- | --- | --- | --- | --- | --- | --- |
| valid saved | する | occupied | runtime item | clean | 置換／tombstone化 | savedへ復元 | defaultとの差分をdirty化 |
| stale saved | しない | stale mask | raw itemを返す | 診断だけではclean | valid recordで置換／削除可能 | staleへ戻りruntime空 | default runtimeへ、SAVEまで診断維持 |
| invalid saved | しない | invalid mask | `NOT_FOUND` | 診断だけではclean | valid recordで置換／削除可能 | invalidへ戻りruntime空 | default runtimeへ、SAVEまで診断維持 |
| metadata invalid | defaultを実行 | metadata fallback flag | default runtime item | 診断だけではclean | 明示mutation後SAVEで完全tableへ置換 | defaultへ戻す | metadata repairをdirty化 |
| unsupported version | defaultを実行 | version fallback flag | default runtime item | 診断だけではclean | RESETまたは編集後SAVEでv1へ置換 | defaultへ戻す | metadata repairをdirty化 |
| storage read error | defaultを実行 | read-error flag | default runtime item | 診断だけではclean | read回復までSAVEは `STORAGE_ERROR` | 既知boot fallbackへ戻す。再読必要時失敗なら `STORAGE_ERROR` | runtime default化。保存はread回復後 |

invalidはvalue / slotのlength、magic、version、reserved、canonical encoding、CRCなど、recordを信頼してitem化できない構造不正。staleはslot CRCまで正しく読み取れるが、現在のfirmwareで実行安全性を満たさない状態である。staleにはbehavior identity不一致、behavior解決不能、parameter不正、position / layer / numeric rangeのcurrent validation失敗、case-insensitive name競合、同一key set + overlapping layer競合を含む。

diagnostic stateとdirty stateは別概念。stale / invalid / fallbackがあるだけでは保存バーを表示しない。ユーザーが該当slotをSET / DELETEするかRESETした時に初めてdirtyになる。

## Combo State Transitions

| Operation | Runtime | Saved baseline / storage | Dirty / diagnostics |
| --- | --- | --- | --- |
| 起動、savedなし | Devicetree default | defaultを論理baselineにする | clean、診断なし |
| 起動、valid table | valid slotだけload | table image | clean、stale / invalid maskは診断 |
| 起動、metadata異常 | Devicetree default | 壊れたimageは自動削除しない | clean、fallback flag |
| SET_COMBO | validation後に1 slot upsert | 変更なし | savedとの差分slotをdirty |
| DELETE_COMBO | slot未使用化 | 変更なし | persisted stateがあればdirty |
| SAVE成功 | 変更なし | dirty slotを含む完全imageへ原子的置換 | dirty解除、書換slot診断解除 |
| SAVE途中失敗 | 編集runtimeを維持 | 旧または新の完全image。混在なし | dirty / diagnostics維持、retry可能 |
| DISCARD | persisted valid slot、またはboot defaultへ復元 | 書込みなし | dirty解除、saved診断維持 |
| RESET_TO_KEYMAP | Devicetree defaultへ置換 | 書込みなし | savedとの差分とmetadata repairをdirty |
| RESET後SAVE | default runtime維持 | defaultを完全table imageとして保存 | dirty / saved診断解除 |
| stale / invalid状態で別slot編集SAVE | 編集slotだけ反映 | clean diagnostic recordはraw保持 | 編集slotdirtyだけ解除、他診断維持 |
| stale / invalid slotをSET後SAVE | valid runtimeを維持 | 該当recordをvalidで置換 | 該当dirty / diagnostic解除 |
| stale / invalid slotをDELETE後SAVE | runtime空 | 該当recordをtombstone化 | 該当dirty / diagnostic解除 |

dirty判定単位は32-bit `dirty_slots`とmetadata repair bit。saved table全体をruntime structとして二重保持しない。2,064 byte serialized scratch / baseline imageを再読可能な形で1つだけ持ち、DISCARDはそのimageまたはSettings再読出しからruntimeを再構成する。

Phase 1C実測RAM 3,925 byteへ、table image 2,064 byte、dirty / stale / invalid / load state約32..80 byteを追加すると、Combo全体は概算6,021..6,069 byte（5.88..5.93 KiB）。6 KiB = 6,144 byte以内だが余裕は約75..123 byteしかない。Phase 2B以降は保存scratch以外の大きなshadow tableを追加せず、最終mapで再計測する。Config RPC共通request cacheを再利用し、Combo専用response cacheや32件分のsaved runtime structを追加しない。
