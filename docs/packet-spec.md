# Keylink Studio Packet Specification

この仕様は、host 側 `Keylink Studio` と ZMK 側 Raw HID 受信処理の間で使う packet を定義します。

## Transport

| Item | Value |
| --- | --- |
| HID Usage Page | `0xFF60` |
| HID Usage | `0x0061` |
| HID write size | 33 bytes |
| Report ID | `0x00` |
| Payload size | 32 bytes |
| Multi-byte values | little-endian |

host が `hidapi` で write する buffer は、先頭 1 byte の Report ID `0x00` と 32 byte payload です。USB HID と BLE HOG のどちらでも Host Link の wire format は同じです。BLE 用の packet type、capability、protocol version は追加しません。

## Common Header

| Offset | Size | Field | Value |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | ASCII `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | packet の種別 |

## Packet Types

| Type | Name | Direction | Notes |
| ---: | --- | --- | --- |
| `0x01` | `HOST_HELLO` | Host -> ZMK | 同じ seq を返すこと |
| `0x02` | `DEVICE_HELLO` | ZMK -> Host | `HOST_HELLO` への応答 |
| `0x03` | `ERROR` | ZMK -> Host | 予約 / v1 でデコード可能 |
| `0x04` | `PING` | Host -> ZMK | 予約 / v1 でデコード可能 |
| `0x05` | `PONG` | ZMK -> Host | 予約 / v1 でデコード可能 |
| `0x10` | `AI_USAGE` | Host -> ZMK | AI 使用量スナップショット |
| `0x20` | `TIME_SYNC` | Host -> ZMK | 時刻表示同期 |
| `0x30` | `APP_LAYER` | Host -> ZMK | アプリレイヤー set / clear |
| `0x40` | `BATTERY_STATUS` | ZMK -> Host | バッテリー残量 (自身 + ペリフェラル) |
| `0x50` | `HOST_ACTION` | ZMK -> Host | ホスト側アクションのトリガー |
| `0x60` | `KEY_STATS` | ZMK -> Host | 位置別キー押下差分 |
| `0x70` | `LAYER_STATE` | ZMK -> Host | アクティブレイヤー報告 (表示専用) |
| `0x80` | `KEY_PRESS` | ZMK -> Host | キー押下 / 離し リアルタイムイベント |

`0x40` 以降は device-initiated (uplink) packet です。host は監視中に非ブロッキング読みで受信します。各 type は対応する capability bit を `DEVICE_HELLO` で立てた device からのみ受け付けます。

## HOST_HELLO / DEVICE_HELLO

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x01` or `0x02` |
| `4..6` | 3 | reserved | ゼロ固定 |
| `7` | 1 | seq | u8 ラッピングカウンター |
| `8..31` | 24 | reserved | ゼロ固定 |

`seq` は u8 wrapping counter です。`Keylink Studio` は `HOST_HELLO` を送り、同じ `seq` の `DEVICE_HELLO` が返った device だけを verified として扱います。

`byte 4..6` と `byte 8..31` は v1 では reserved zero です。

## APP_LAYER

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x30` |
| `4` | 1 | action | `1=set`、`2=clear` |
| `5` | 1 | layer | set は `0..31`、clear は `0` |
| `6` | 1 | reserved | ゼロ固定 |
| `7` | 1 | seq | u8 ラッピングカウンター |
| `8..31` | 24 | reserved | ゼロ固定 |

バリデーションルール:

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
| `4` | 1 | provider | `1=codex`、`2=claude_code` |
| `5` | 1 | flags | 後述 |
| `6..7` | 2 | five_hour_used_bp | 使用率 × 100 |
| `8..9` | 2 | seven_day_used_bp | 使用率 × 100 |
| `10..13` | 4 | five_hour_reset_unix | uint32、リトルエンディアン |
| `14..17` | 4 | seven_day_reset_unix | uint32、リトルエンディアン |
| `18..21` | 4 | updated_unix | uint32、リトルエンディアン |
| `22` | 1 | error_code | 後述 |
| `23..31` | 9 | reserved | ゼロ固定 |

`used_bp` は used basis points です。`10000` は `100.00%` を意味します。

### Flags

| Bit | Name | Meaning |
| ---: | --- | --- |
| 0 | `five_hour_valid` | 5 時間使用量フィールドが有効 |
| 1 | `seven_day_valid` | 7 日間使用量フィールドが有効 |
| 2 | `estimated` | 値は推定値 |
| 3 | `local_history_source` | ソースはローカル履歴 |
| 4 | `quota_source` | ソースはクォータ / レートリミット情報 |
| 5 | `stale` | 値は古い |
| 6 | `fallback_limit` | アクティビティベースラインを使用 |
| 7 | `error_present` | error_code が有効 |

`AI_USAGE` には reset valid flag はありません。reset を quota reset として扱えるかどうかは、値そのものと `quota_source` で判断します。

### Source Rules

| Source | estimated | local_history_source | quota_source | reset fields |
| --- | ---: | ---: | ---: | --- |
| Codex `rate_limits` | 0 | 0 | 1 | クォータリセット |
| Codex history fallback | 1 | 1 | 0 | `0` |
| Claude OAuth API success | 0 | 0 | 1 | クォータリセット |

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
| `4..7` | 4 | unix_time_sec | `uint32`、リトルエンディアン |
| `8..9` | 2 | tz_offset_min | `int16`、リトルエンディアン |
| `10` | 1 | weekday | ISO weekday、`1=Mon ... 7=Sun` |
| `11` | 1 | format_hint | 表示フォーマットプリセット |
| `12` | 1 | clock_mode | `0=24h`、`1=12h` |
| `13..31` | 19 | reserved | ゼロ固定 |

`TIME_SYNC` は毎秒送りません。ZMK 側は `TIME_SYNC` 受信時の uptime を保存し、uptime 差分で表示秒を進める想定です。

## BATTERY_STATUS (ZMK -> Host)

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x40` |
| `4` | 1 | count | エントリ数、`1..=4` |
| `5+2i` | 1 | source[i] | `0=self/dongle`、`1=left`、`2=right`、`3=aux` |
| `6+2i` | 1 | level[i] | `0..=100`、`0xFF` = 不明 / 切断 |
| `5+2*count..31` | - | reserved | ゼロ固定 |

Validation: count `1..=4`、source `0..=3` かつ packet 内で重複禁止、level は `0..=100` か `0xFF`(`101..=254` は reject)。`0` は有効な 0% です。`0xFF` は不明または切断を表し、host UI では `--%`、tray tooltip では `?` として表示します。変化時+定期(~5分)送信を想定します。

## HOST_ACTION (ZMK -> Host)

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x50` |
| `4` | 1 | action_id | 不透明 ID。意味は host config で定義 |
| `5` | 1 | value | アクションの引数。未使用時は `0` |
| `6` | 1 | reserved | ゼロ固定 |
| `7` | 1 | seq | u8 ラッピングカウンター |
| `8..31` | 24 | reserved | ゼロ固定 |

host は同一 device からの **同じ seq の連続受信を 1 回として扱います**(firmware の二重送信対策)。リトライは不要です。action の実行内容は host 側 config の許可リスト(`[actions]`)で device 単位 (`actions.devices."uid:..."`) に定義し、未定義 id・未設定 device はログのみです。`value` を path やコマンドとして解釈することはありません。

## KEY_STATS (ZMK -> Host)

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x60` |
| `4` | 1 | entry_count | `1..=8` |
| `5` | 1 | flags | bit0 = `MORE_FOLLOWS`、他ビットはゼロ固定 |
| `6` | 1 | reserved | ゼロ固定 |
| `7` | 1 | seq | u8 ラッピングカウンター (ギャップ = パケットロスト) |
| `8+3i` | 1 | position[i] | キー位置 (ZMK Studio の physical layout 位置) |
| `9+3i..10+3i` | 2 | delta[i] | u16 LE 前回報告からの押下数。`0` は reject |
| `8+3*entry_count..31` | - | reserved | ゼロ固定 |

firmware は位置別カウンタを保持し、定期的(30〜60秒)に **非ゼロの position だけ** を送って 0 クリアします。8 entry を超える場合は複数 packet に分割し、最後以外に `MORE_FOLLOWS` を立てます。host は seq gap を警告ログにしつつ受信分を加算します(欠落はアンダーカウントとして許容)。

## LAYER_STATE (ZMK -> Host)

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x70` |
| `4` | 1 | active_layer | 最上位アクティブレイヤー、`0..=31` |
| `5..6` | 2 | reserved | ゼロ固定 |
| `7` | 1 | seq | u8 ラッピングカウンター |
| `8..11` | 4 | layer_mask | u32 LE。bit i = レイヤー i がアクティブ。`0` = 最上位レイヤーのみ |
| `12..31` | 20 | reserved | ゼロ固定 |

`layer_mask` が非ゼロの場合、`active_layer` の bit が立っていなければ reject します。layer-state-changed イベント時に送信(~50ms デバウンス推奨)します。**host はこの報告を表示にのみ使い、APP_LAYER としてエコーバックしません。**

## KEY_PRESS (ZMK -> Host)

| Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | magic | `"HL"` |
| `2` | 1 | version | `0x01` |
| `3` | 1 | type | `0x80` |
| `4` | 1 | position | キー位置 (ZMK Studio の physical layout 位置) |
| `5` | 1 | flags | bit0 = `PRESSED` (1=押下, 0=離し)、他ビットはゼロ固定 |
| `6` | 1 | reserved | ゼロ固定 |
| `7` | 1 | seq | u8 ラッピングカウンター |
| `8..31` | 24 | reserved | ゼロ固定 |

`zmk_position_state_changed` の pressed / released 両方でそれぞれ 1 packet 送ります。host は受信した position を UI 上でリアルタイムハイライトするキーテスター機能で使用します。累積カウントは保持しません (KEY_STATS とは独立)。capability bit = `BIT(8)`。

## ZMK 実装時の要点

- 33 byte write の先頭は Report ID `0x00` です。
- payload は常に 32 byte です。
- USB HID と BLE HOG で payload layout を分けないでください。
- magic `"HL"` と version `0x01` を確認してください。
- reserved byte は zero を前提にしてください。
- `HOST_HELLO` に対して、同じ `seq` の `DEVICE_HELLO` を返してください。
- `APP_LAYER` は packet type ではなく `action` で set / clear を分岐してください。
- `AI_USAGE` の history fallback は quota ではありません。`quota_source=0` の reset は表示しないでください。
- uplink packet (`0x40`〜`0x80`) を送る場合は、対応する capability bit を `DEVICE_HELLO` で立ててください。bit が立っていない type は host が破棄します。
- uplink は best-effort です。host が読んでいない間(監視停止中など)の packet は失われます。`KEY_STATS` の欠落はアンダーカウントとして扱われます。
- `HOST_ACTION` の応答は最大で host の polling 間隔(既定 500ms)遅れます。

## デバイス識別と capability

`DEVICE_HELLO` v1 には、安定した識別子と capability フィールドを含めることができます。host はこれらのフィールドをデバイス単位の機能振り分けに使います。

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

- `seq` は直前の `HOST_HELLO` の値と一致しなければなりません。
- byte `6` および bytes `20..31` は zero でなければなりません。
- `device_uid_hash = 0` は host 側で `None` に正規化されます。
- 非ゼロの `device_uid_hash` は `uid:<16桁小文字16進数>` として表示・保存されます。

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
bit8 = KEY_PRESS    (device sends KEY_PRESS)
```

Keylink Studio は `APP_LAYER` capability がないデバイスへ `APP_LAYER` packet を送信しません。その場合もデバイスは Devices 画面に Host Link device として表示されます。

### capability の host 側動作

host は `capabilities` を機能ゲートとして扱います。capability がない場合も verified Host Link device は Devices 画面に表示されますが、機能 packet は対応する capability がある場合にのみ送信または受理されます。`APP_LAYER` / `TIME_SYNC` / `AI_USAGE` は送信可否、`BATTERY_STATUS` / `HOST_ACTION` / `KEY_STATS` / `LAYER_STATE` / `KEY_PRESS` は受信可否に使います。

`device_uid_hash` は識別ヒントであり、通信経路ではありません。host は非ゼロ値を config および UI 向けに `uid:<16桁小文字16進数>` として保存します。値 `0` は識別子として無効であり、`None` に正規化されます。
