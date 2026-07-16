# Combo Editing Specification

この文書は、Keylink Studio と `zmk-rawhid-app` に追加する combo 編集機能の合意済み仕様を定義する。2026-07-13時点でFirmware Phase 1C〜2D、Host core／CLI／Tauri、Keymap Viewer UIを実装済みである。CLI mutationとUI read-only表示まで確認し、UIからの完全なmutation／永続化手順とExport／Restoreが未完了である。

## 1. 目的と基本方針

- ZMK の `.keymap` に定義された combo を初期値として読み込み、Keylink Studio から追加・編集・削除できるようにする。
- ZMK本体は変更しない。`/combos`を `status = "disabled"` とし、disabled childを `zmk-rawhid-app` がdefaultへ変換する。ZMK標準Combo listenerはELFへ入れず、module側の単一listenerだけがRAM上の共通combo tableを判定する。
- Combo 編集には ZMK Studio RPC と Host Link Config RPC の両方を使う。通常キー編集、Encoder 編集、Combo 編集の可用性と結果は分離する。
- Encoder の Config RPC、dirty、保存、破棄、保存レコード検証、部分失敗の設計を可能な範囲で踏襲する。
- 最大数分の静的配列を使用し、動的メモリ確保は行わない。
- build後のmap検査でlistener順が `Keylink Combo -> activity -> hold-tap -> keymap` であることと、`zmk_listener_combo` / 標準subscriptionが不在であることを強制する。この順序はZMK公開API保証ではなく、固定ZMK revision `484a0547` に対するbuild安全装置である。

### 1.1 Firmware Phase 1C result

- disabled `/combos`から読み取り専用default tableを生成し、32 static runtime slotsへ起動時コピーする方式が成立した。
- candidate、capture / re-raise、per-Combo timeout、包含 / overlap、active Combo、release、slow-release、layer、prior-idle、modifier除外、hold-tap相互作用をZMK revision `484a0547` の `combo.c` 相当としてmoduleへ移植した。
- Cornix defaultは重複して発火不能だった後方 `layer4` を削除し、6 Combo。central / left / right、clean / incremental buildが成功した。
- 32 Comboはbuild成功、33 Combo、1-key、9-key、同一key set + overlapping layerはbuild failure。layer完全分離、包含、partial overlapはbuild成功した。
- 標準ZMKとKeylink runtimeで2/3/4-key包含、fully / partial overlap、異なるtimeout、release、slow-release、layer、prior-idle、hold-tap、active上限 / 再利用、左右position横断のkeycode出力を比較し、意味論が一致した。
- Cornix実機で通常キー、既存Combo、timeout、release、layer、hold-tap、左右横断、非成立時re-raise、二重発火なし、再接続後default復元を確認済み。
- Phase 1C実測はCombo RAM約3,925 byte、Flash増加約3,240 byte。

## 2. 上限

| 項目 | 値 |
| --- | ---: |
| 最大 combo 数 | 32 |
| 1 combo の最小キー数 | 2 |
| 1 combo の最大キー数 | 8 |
| combo 名保存領域 | 16 byte（終端を含み最大15文字） |
| `timeout-ms` | 1..1000 ms、1 ms単位、既定50 ms |
| `require-prior-idle-ms` | 無効、または1..1000 ms、1 ms単位 |

- 上限は `zmk-rawhid-app` に持たせ、通常の `zmk-config-xxx` には新しい設定を追加しない。
- Devicetree に32個を超える combo がある場合は、キーボードfirmwareのビルドをエラーにする。一部だけを黙って無視しない。
- `COMBO GET_INFO` から最大 combo 数と最大キー数をHostへ通知し、Studioへ固定値を埋め込まない。
- 同時に保持できる発動済みcombo数は、ZMK既存の `CONFIG_ZMK_COMBO_MAX_PRESSED_COMBOS` に従う。Keylink独自設定は追加しない。

## 3. Combo model

各comboは次の情報を持つ。

- `name`
- `key-positions`
- `bindings`（1 binding、behavior + 最大2 parameter）
- `layers`
- `timeout-ms`
- `slow-release`
- `require-prior-idle-ms`

ユーザーが切り替える有効・無効フラグは設けない。スロットは未使用または使用中の2状態とする。

### 3.1 name

- 必須。1..15文字。
- 使用可能文字は `a-zA-Z0-9,._+-`。
- 大文字・小文字を区別せず一意とする。例: `Nav` と `nav` は同時登録不可。
- 表示時は入力された大文字・小文字を維持する。
- 内部識別には名前ではなくslot IDを使用する。
- Studioで新規追加するときは、未使用の `combo1`, `combo2`, ... を初期名として設定し、名前欄をすぐ編集できる状態にする。
- Devicetree node名から初期名を作るときは、禁止文字を削除してから15文字へ切り詰める。
- 変換後が空なら `combo1`, `combo2`, ... を割り当てる。
- 変換後に重複したら `_2`, `_3`, ... を付け、接尾辞を含め15文字以内へ収める。

### 3.2 key-positions

- Studio上のキーボードレイアウトをクリックして選択する。
- 選択済みキーの再クリックで解除する。
- 選択中キーはアクセントカラーで表示する。
- 選択数の `2 / 8` のような表示は設けない。
- 2キー未満では適用不可。8キー選択中は追加選択不可。
- 選択順序は意味を持たず、キー位置番号の昇順へ正規化して保存する。
- レイヤーを切り替えても、キー位置に対する選択状態を維持する。
- 左右分割をまたぐ選択を許可する。
- 物理キーボードから押下を記録する機能はMVP対象外。

### 3.3 重複とoverlap

- `A+B` と `A+B+C` のような包含combo、および一部が重なるcomboを許可する。
- timeoutと候補解決はZMKの現在のcombo判定方式へ合わせる。
- 各comboのtimeoutは、そのcomboに含まれる最初のキー押下時刻から計測する。
- 短いcomboは、包含する長い候補が残る間は保留する。
- 長いcomboが成立した場合、包含される短いcomboは発火しない。
- 長い候補がすべて不成立になった場合、成立済みの短いcomboを発火する。
- 保留時間に関する個別警告はStudioに表示しない。
- 同じキー集合でも、有効レイヤーが完全に分離していれば登録可能。
- 同じキー集合で有効レイヤーが1つでも重なれば登録不可。
- 「すべてのレイヤー」は、同じキー集合を持つすべてのcomboと重複する。
- Studioとfirmwareの両方で重複を検証する。

### 3.4 bindings

- 1 comboにつきbindingは1つ。
- Keymap編集と同じbehavior選択UIとfirmware behavior catalogを利用する。
- `&kp`に限定せず、firmwareが公開しcomboから実行可能なbehaviorを選択できる。
- binding未設定では適用不可。
- combo成立時にbehaviorを押下し、combo解除時にbehaviorを解放する。
- UI項目名は `bindings`。ツールチップは「comboが成立したときに実行する動作です。」とする。
- ユーザー向けエラー文ではbindingを「動作」と表現する。
- 保存後にbehaviorを解決できなくなったcomboは `stale` とし、実行しない。一覧には残して修正または削除できるようにする。
- stale表示は「このcomboに設定された動作は、現在のファームウェアでは使用できません。別の動作を選択してください。」とする。

### 3.5 layers

- 初期値は「すべてのレイヤー」。
- 「すべて」または1つ以上の個別レイヤーを選択する。
- 個別指定は複数選択可能。
- Studioにはレイヤー番号と表示名を表示する。
- firmwareで利用可能なレイヤーだけを選択可能とする。
- 「すべて」はspecial stateとして保持し、全bitを立てない。wire上では `layer_mask = 0` とする。
- 判定はZMKと同様、現在の最上位active layerが対象に含まれるかで行う。

### 3.6 timeout-ms

- comboごとに設定する。
- 既定50 ms、1..1000 ms、1 ms単位。
- ツールチップは「comboを成立させるために、最初のキーを押してから残りのキーを押し終えるまでの制限時間です。」とする。

### 3.7 slow-release

- comboごとにオン／オフを設定し、既定はオフ。
- オフでは構成キーのどれか1つを離した時点、オンではすべてを離した時点でcomboを解除する。
- ツールチップは「オフの場合、comboのキーを1つでも離すとcomboを解除します。オンの場合、すべてのキーを離したときに解除します。」とする。

### 3.8 require-prior-idle-ms

- UI名はZMK property名どおり `require-prior-idle-ms`。
- comboごとに無効／有効を設定し、既定は無効。
- 有効時は1..1000 ms、1 ms単位。wire / Settings上の無効値は `0xFFFF`、Phase 1C runtime内部では `0` を無効値として扱う。encode / decode境界で明示変換する。
- 修飾キーだけの直前入力はidle判定に影響させない。
- ツールチップは「comboの最初のキーを押す直前に通常キーが入力されていた場合、指定時間が経過するまでcomboを成立させません。高速入力中の意図しないcomboを防止できます。」とする。

## 4. 初期値、runtime、保存状態

状態を次の3層に分ける。

```text
Devicetree初期値（Flash・読み取り専用）
        ↓ 初回または .keymap に戻す
保存済み設定（Zephyr Settings）
        ↓ 起動時
実行中設定（RAM上の32 static slots）
```

- 実際のcombo判定に使うのはRAM上の共通tableだけ。
- 保存済み設定がなければDevicetree定義を先頭からslotへ読み込む。
- Studio追加は空きslotを使う。初期comboと追加comboを実行時に区別しない。
- Devicetree由来comboも編集・削除可能。
- 編集した初期comboについて、元のDevicetree comboと編集後comboを二重発火させない。
- 削除後に保存すれば再起動しても復活しない。
- firmware更新後も保存済みtableが存在すれば、Devicetreeの変更を自動mergeせず保存済み設定を優先する。
- Devicetree変更を反映するには既存の「`.keymapに戻す`」を使う。
- combo専用「初期設定に戻す」は設けない。

## 5. 編集、適用、保存、破棄

- Combo編集は既存のKeymap／Encoder編集sessionと保存バーへ統合する。
- KeymapタブとComboタブの両方に編集ボタンを置き、どちらから開始しても同じ編集sessionへ入る。編集状態はタブごとに分離しない。
- フォーム入力中はStudio内の下書きであり、まだfirmwareへ送らない。
- 新規追加は「適用」で初めて空きslotへ登録する。
- 既存編集も「適用」までfirmwareへ送らない。
- 「適用」は編集中のcombo 1件だけをRAMへ即時反映する。64 byte packetへ複数comboをまとめない。
- 複数comboは1件ずつ適用し、最後の共通「変更を保存」で `COMBO SAVE` を1回送って、すべてのdirty comboをFlashへ保存する。
- フォーム右下の「キャンセル」は下書きを破棄する。既存Comboでは最後に適用した内容へ戻し、新規追加直後の仮枠は一覧から消す。
- 下書きがある状態で別comboへ移動するときは「適用していない変更があります。変更を破棄して移動しますか？」と確認する。
- 下書き中は共通の保存、破棄、`.keymapに戻す`を実行不可とする。
- 入力エラーがあれば「適用」を無効化する。
- 個別削除に確認dialogは出さない。RAMから削除してdirtyにし、共通「変更を破棄」で戻せるようにする。
- 32個登録済みなら追加を無効化し、「登録できるcomboの最大数に達しています。」をtooltip表示する。

### 5.1 キー押下中の変更

- いずれかのキーが押されている間、Comboの追加、編集、削除、`DISCARD`、`RESET_TO_KEYMAP`を適用しない。
- firmwareは `BUSY` を返し、Studioは下書きまたはdirtyを維持する。自動再送せず、キーを離してユーザーが再実行する。
- 適用時の表示は「キーが押されています。すべてのキーを離してから、もう一度適用してください。」とする。
- `SAVE`はruntime定義を変えないためキー押下中でも実行可能。
- 通常キー、Encoder、Comboの結果は分離し、Comboだけ `BUSY` の場合も他featureの成功を巻き戻さない。

## 6. UI

- 既存Keymap編集画面内に `Keymap` / `Combo` の切り替えを設ける。編集ボタンはタブ列へ置かず、各タブの内容に属する操作として配置する。
- Combo modeでは通常キーbindingを編集せず、レイアウトをcomboキー選択に使用する。
- Combo一覧には設定済みslotだけを縦に表示し、名前、動作、対象レイヤーと各Comboの削除ボタンを置く。新規作成は一覧下部の`追加`から空きslotへ進み、追加直後は仮枠を表示する。
- フォーム項目名は `name`, `key-positions`, `bindings`, `layers`, `timeout-ms`, `slow-release`, `require-prior-idle-ms`。
- 一覧は `name` を主表示、`bindings` と `layers` を補助表示する。選択中はアクセント色の塗りつぶしではなく枠線で示す。
- `layers`はポップオーバーで複数選択し、mask 0は`すべて`、個別指定は`番号: レイヤー名`で表示する。
- 一覧で選択したcomboのキーは、通常キーの変更表示と同じ薄いアクセント背景と枠線で示す。
- 一覧順はslot順。重複comboの優先順位には使わず、並べ替え機能は設けない。
- firmware非対応時は「ファームウェアがcombo編集に対応していません。」と表示する。

## 7. 対応判定

Combo編集を利用可能とする条件:

1. ZMK Studio RPCが利用可能。
2. Host Link v2が利用可能。
3. Studio deviceのUIDとHost Link `device_uid_hash`が厳密一致。
4. `CONFIG_RPC` capabilityがある。
5. `COMBO GET_INFO`が成功する。

- Combo専用の `DEVICE_HELLO` capability bitは追加しない。
- Encoderと同様、共通 `CONFIG_RPC` capabilityの後にfeature固有 `GET_INFO`でprobeする。
- Host LinkとStudio RPCがOKでも、Combo feature実装前のfirmwareは `COMBO GET_INFO`非対応なのでCombo編集不可とする。
- Combo非対応でも通常キー／Encoder編集を無効化しない。

## 8. Config RPC

### 8.1 featureとoperations

- feature ID: `COMBO = 0x02`

| Op | Name | Purpose |
| ---: | --- | --- |
| `0x01` | `GET_INFO` | 上限、登録数、使用slot取得 |
| `0x02` | `GET_COMBO` | 1 slot取得 |
| `0x03` | `SET_COMBO` | 1 comboをRAMへ追加／更新 |
| `0x04` | `GET_DIRTY` | dirty取得 |
| `0x05` | `SAVE` | dirtyをSettingsへ保存 |
| `0x06` | `DISCARD` | 保存済み状態へ戻す |
| `0x07` | `DELETE_COMBO` | 1 comboをRAMから削除 |
| `0x08` | `RESET_TO_KEYMAP` | RAM tableをDevicetree初期値へ戻す |

- lifecycle op番号と意味はEncoderに合わせる。
- `RESET_TO_KEYMAP`の単独UIは設けず、共通「`.keymapに戻す`」から呼び、その後 `SAVE` する。
- statusは既存の `OK`, `BAD_PACKET`, `UNSUPPORTED_FEATURE`, `UNSUPPORTED_OP`, `INVALID_ARGUMENT`, `BUSY`, `NOT_FOUND`, `STORAGE_ERROR`, `INTERNAL_ERROR` を再利用する。

### 8.2 GET_INFO response

`payload_len = 16`。

| Offset | Field | Meaning |
| ---: | --- | --- |
| `0` | `max_combos` | 現在32 |
| `1` | `max_keys_per_combo` | 現在8 |
| `2` | `combo_count` | 実行可能runtime Combo数 |
| `3` | `flags` | saved table / fallback / storage診断 |
| `4..7` | `occupied_slots` | 実行可能runtime slot mask、u32 LE |
| `8..11` | `stale_slots` | 読み取り可能だが実行不能なsaved slot mask |
| `12..15` | `invalid_slots` | 構造 / CRC不正saved slot mask |

Studioの列挙対象は `occupied_slots | stale_slots | invalid_slots`。itemを取得できるのはruntime occupiedまたはstale slot。invalid slotはplaceholderとして表示し、修正／削除できるようにする。

### 8.3 Combo item wire layout

Host Link payload 52 byteに1 comboをちょうど格納する。

| Offset | Size | Field | Notes |
| ---: | ---: | --- | --- |
| `0` | 1 | `slot_id` | 0..31 |
| `1` | 1 | `key_count_flags` | bits 0..3 raw key_count、bit 4 slow-release、bits 5..7 zero |
| `2..17` | 16 | `name` | NUL終端、最大15 byte ASCII。NUL後zero |
| `18..33` | 16 | `key_positions[8]` | u16 LE、未使用は `0xFFFF` |
| `34..43` | 10 | `binding` | behavior_id u16 + param1 u32 + param2 u32 |
| `44..47` | 4 | `layer_mask` | u32 LE、0はall layers |
| `48..49` | 2 | `timeout_ms` | u16 LE |
| `50..51` | 2 | `require_prior_idle_ms` | u16 LE、`0xFFFF`は無効 |

- `GET_COMBO` responseと `SET_COMBO` requestで同じitem formatを使う。
- 分割送受信やmulti-packet transactionはMVPでは不要。
- wire structと内部structを同一視せず、明示的LE encode / decodeと52 byte compile-time assertionを使う。
- nameはUTF-8領域だがMVPの許可文字がASCII subsetのため、multibyte文字は受理しない。
- 受信key positionsは昇順へ正規化し、重複position、範囲外position、unused fieldが`0xFFFF`でないitemを拒否する。
- ComboBindingはEncoderBindingと同じ10 byte raw layoutを使う。`behavior_id = 0xFFFF`は無効。
- stale / invalid / saved状態はitemへ追加せず、GET_INFO masksを正とする。

### 8.4 request / responseとBUSY

- `GET_INFO`: request 0 byte、response 16 byte。
- `GET_COMBO`: request `slot_id` 1 byte、response Combo item 52 byte。
- `SET_COMBO`: request Combo item 52 byte、response 0 byte。slotはupsert先で、追加／更新flagは設けない。
- `GET_DIRTY`: request 0 byte、response `dirty` 1 byte。
- `SAVE`: request / response 0 byte。
- `DISCARD`: request / response 0 byte。
- `DELETE_COMBO`: request `slot_id` 1 byte、response 0 byte。空slotはidempotent `OK` no-op。
- `RESET_TO_KEYMAP`: request / response 0 byte。

`SET_COMBO` / `DELETE_COMBO` / `DISCARD` / `RESET_TO_KEYMAP` は `combo_runtime_idle() == false` なら `BUSY`。GET系とSAVEはキー押下中も許可する。全offset、status、retry、Settings異常時の詳細は `hostlink-config-rpc-packet-spec.md` のCombo節を正本とする。

## 9. Settingsと互換性

- Settings keyは `keylink/cmb/v1/table`。version 1は複数slot keyを使わず、16 byte header + 32 x 64 byte slot record = 2,064 byteの固定table imageを1 keyへ保存する。
- table magicはASCII `KCMB`、保存形式versionは1。headerと各slotへCRC-32/ISO-HDLCを持つ。
- slot recordはwire itemからslot IDを除いたmodel、occupied flag、SHA-256 identity digest先頭8 byte、CRCを持つ。
- EncoderとComboの保存形式versionは別管理とする。
- 保存形式が互換なfirmware更新では保存済みComboを維持する。
- 保存record構造を互換性なく変更した場合だけCombo record versionを上げる。app versionや通常のfirmware更新では上げない。
- 同一キー数のままキー位置番号だけを入れ替えるfirmware更新はMVPで自動検出しない。その場合はユーザーが「`.keymapに戻す`」を実行する。
- comboごとにrecordを検証し、1件の破損で他の正常comboを失わない。
- 壊れたcomboは実行せず、他comboは読み込む。不正slotへDevicetree初期comboを自動復活させない。
- table全体の管理情報を読めない場合だけDevicetree初期値へfallbackする。
- 保存失敗時はRAM上の編集内容とdirtyを維持し、再保存可能とする。
- `occupied_mask = 0`かつcanonical tombstone recordが削除済みslotを表す。saved tableが存在する限りDevicetree defaultを自動mergeしないため、削除済みdefaultは再起動後も復活しない。
- SAVEはpersisted imageをscratchへ読み、dirty slotだけを置換して `settings_save_one()`を1回呼ぶ。Zephyr NVSはdata後にATEを書くため、失敗／電源断後も新旧slotが混在しない。
- slot別key + metadata、A/B bank、generation commit markerはversion 1では不採用。Zephyr Settingsにmulti-key transactionがなく、A/B loadの追加RAMまたはboot時非同期loadが6 KB目標とlistener安全性を悪化させるため。
- stale / invalid recordは起動時に自動削除せず、該当slotをSET / DELETEしてSAVEが成功した場合だけ置換／tombstone化する。無関係slotのSAVEではraw recordを保持する。
- stale / invalid / fallback診断はdirtyとは別概念であり、診断だけでは保存バーを表示しない。

通知:

- 一部破損: 「一部のcombo設定を読み込めませんでした。読み込めなかったcomboは無効になっています。」
- 保存形式非互換: 「保存されていたcombo設定に互換性がないため、ファームウェアの初期設定を読み込みました。」

### 9.1 saved state

| State | Runtime | Host通知 | GET_COMBO | Dirty |
| --- | --- | --- | --- | --- |
| valid | 実行する | occupied mask | runtime item | clean |
| stale | 実行しない | stale mask | raw item | 診断だけではclean |
| invalid | 実行しない | invalid mask | `NOT_FOUND` | 診断だけではclean |
| metadata invalid / unsupported | Devicetree default | GET_INFO fallback flag | default runtime item | 診断だけではclean |
| storage read error | Devicetree default | read-error flag | default runtime item | 診断だけではclean |

invalidはlength / magic / version / reserved / canonical encoding / CRC等の構造不正。staleはCRCまで正しく読めるが、behavior identity / resolve / parameter、current position / layer / numeric validation、name / key set競合等により現在のfirmwareで実行不能なrecord。

## 10. 共通保存、破棄、.keymapに戻す

- 共通「変更を保存」はStudio RPCの通常キー、Config RPC Encoder、Config RPC Comboを個別実行する。
- 結果は3feature別に返し、一部成功を巻き戻さない。
- 成功featureだけdirty解除し、失敗featureはdirtyを維持する。
- 共通「変更を破棄」と「`.keymapに戻す`」も3feature別結果を表示する。
- 「`.keymapに戻す`」では通常キー／レイヤー、Encoder override、Combo tableをそれぞれfirmware初期値へ戻す。

例: 「通常キーとEncoderを保存しました。Comboの保存に失敗しました。」

## 11. Export / Restore

- Export対象へComboを追加し、現在有効な全Combo（Devicetree由来、Studio編集、Studio追加）を含める。
- 各entryにmodelの全fieldを保存する。
- 削除済みComboは含めない。
- Comboを読み取れない場合は黙って省略せず警告する。
- RestoreはEncoderと同様の安全な追加／更新方式とする。全table置換ではない。
- `combos` sectionがなければComboを変更しない。
- `combos` sectionが空でも何も変更しない。
- 同じ `name` があれば更新し、なければ追加する。
- backupにない既存Comboは削除しない。
- キー集合／layer競合、名前、上限、key position、behavior、layer、各数値をpreviewで検証する。
- 対象firmwareがCombo非対応ならComboを復元せず警告し、通常キーとEncoderは継続可能とする。
- Restore preview／resultへComboの追加、変更、skip、blocked、warningをfeature別に表示する。
- この追加／更新方式では、backup作成時に削除済みだったDevicetree初期Comboを復元先から自動削除しない。MVPの既知制約とする。

## 12. RAM budget

- 32 static slotsを確保するため、実際の登録数が少なくても32個分のRAMを使用する。
- Phase 1Cのlistener / runtime table / active / capture / candidate / timeout状態はmap実測で約3,925 byte。
- Phase 2AのSettings table scratchは2,064 byte固定。dirty / stale / invalid masksとload stateは約32..80 byteを見込む。
- saved runtime tableの全コピーは持たず、serialized imageまたはSettings再読出しからDISCARDする。Combo専用の2つ目のtable imageや32件分のsaved runtime structは追加しない。
- 合計概算は約6,021..6,069 byte（5.88..5.93 KiB）。6 KiB = 6,144 byte以内だが余裕は75..123 byteと小さい。
- Combo機能全体の目標を6 KB以内とする。
- Phase 2B以降は共通Config RPC cacheを再利用し、実装後にmap fileで再実測する。6 KBを超える場合はpacket仕様を削らず、scratch lifetime、runtime slot padding、共通work stateの共有から見直す。

## 13. 実装状況と次の作業

2026-07-13終了時点の状態:

- Phase 1C完了: ZMK本体を変更せず、`/combos`をdisabledにして`zmk-rawhid-app`の単一runtime listenerへ移行した。32-slot default/runtime table、ZMK相当のoverlap／release／slow-release／prior-idle／hold-tap動作、listener map検査、Cornix実機確認まで完了した。
- Phase 2A完了: Config RPC packetと2,064 byte単一Settings table imageの仕様を確定し、本書と`docs/hostlink-config-rpc-packet-spec.md`へ反映した。
- Phase 2B完了: firmware `GET_INFO` / `GET_COMBO`を実装し、Cornix実機で全6 slot、エラー応答、retry、既存Encoder RPC、handshake、通常入力を確認した。
- Phase 2C完了: table decoder、header／slot CRC、valid／stale／invalid分類、Settings起動時load、GET_INFO／GET_COMBO診断反映を実装した。production loaderを直接通すnative fixtureで、keyなし、valid空／6／32件、tombstone、invalid、stale、重複、metadata fallback、unsupported version、read error、short image、連続loadを検証した。
- Phase 2C production確認完了: Cornix central pristine／incremental、left／right build、productionへのtest seam非混入、USB backend維持、listener順、標準Combo listener不在を確認した。Combo RAM実測は4,964 byteで、6,144 byte上限内（残り1,180 byte）。対象UF2でCombo動作、GET_INFO／GET_COMBO、USB／BLE再接続、central再起動後の復元を実機確認した。
- Phase 2D完了: `SET_COMBO` / `DELETE_COMBO` / `GET_DIRTY` / `SAVE` / `DISCARD` / `RESET_TO_KEYMAP`、単一table NVS保存、write failure／ambiguous commit、deferred SAVEを実装した。native全batch、Cornix production build、slot 31を使ったmutation／SAVE／再接続後永続化／baseline復元を確認した。最終Combo RAMは約6,016 / 6,144 byte（残り約128 byte）。
- Host H1〜H3完了: `rawhid-host-core`へ型付きCombo codec／API、`rawhid-host-cli`へ全8 operationを追加した。responseは`seq + feature + op`で照合し、GET_COMBOはslot echoも確認する。timeout時は同一requestを1回retryし、deferred SAVEは1,500 ms以上待つ。現行workspace testはcore 177件、Tauri 9件がPASSしている。
- Host read-only実機確認完了: Cornix UID `2e4aa7fde0cef93d`でGET_INFO、GET_DIRTY、slot 0〜5、空slot 31をproduction CLIから確認した。saved table、6件、occupied `0x0000003f`、stale／invalidなし、dirty=falseでbaselineと一致した。
- Host mutation実機確認完了: slot 31だけを使用し、SET、GET_DIRTY、DISCARD、SET、SAVE、USB再接続後の永続化、DELETE、SAVE、USB再接続後のbaseline復元を確認した。既存slot 0〜5は変更していない。
- Tauri／UI実装完了: Combo DTO／commandとHost Link workerを追加し、Keymap Viewer編集モードへ`Keymap`／`Combo`切替、slot一覧、物理キー位置の選択、既存Behavior Pickerによる`bindings`選択、個別DELETEを追加した。ComboのRAM dirtyは共通編集バーへ統合し、保存・破棄・`.keymapに戻す`はCombo専用ボタンを設けず共通操作から実行する。フォーム右下には`キャンセル`と`適用`を置き、キャンセルは未適用の既存draftを最後の適用状態へ戻し、新規の仮枠を消す。フォームの未適用draft中は共通操作を無効化し、slot移動時に破棄確認を行う。
- Tauri／UI自動検証完了: `cargo test --workspace --no-fail-fast`（core 177、Tauri 9）と`cargo build --workspace`、`npm run build`、`git diff --check`がPASSした。
- UI実機確認完了: Cornixの設定済み6件をComboタブへ表示し、一覧、キー構成、動作、レイヤー、各propertyを実画面で確認した。重複Comboの適用はHost側事前validationで拒否し、firmwareの`INVALID_ARGUMENT`を技術コードのまま見せないUIへ変更した。Key Pressなどの動作は`&kp`等の短いZMK表記、キー位置はbinding labelと数値positionを併記する。設定済みComboだけの縦一覧、`追加`、枠内削除、`Group`アイコン、property tooltip、Behavior Picker検索、追加直後の仮枠、キャンセルを反映した。Keymap／Comboの両タブに同じ編集sessionを操作する編集ボタンを置き、layersは全レイヤー／複数レイヤーの選択と名前表示に対応した。Export完了／Restore対象なしの完了ダイアログ、Restore確認の通常キー・エンコーダ・Combo別集計、Restore後に重複成功通知を出さないことも確認した。

次の未実装範囲:

1. release準備と必要最小限の最終回帰。

次はrelease準備から始める。既存slot 0〜5を変更せず、必要な実機回帰は空きslot 31だけを使う。Firmware基準点は`zmk-raw-hid`の`43d9e4b feat: support 64-byte Host Link packets`と、`zmk-rawhid-app`の`cc1b8ef fix: restore combo editing firmware support`。Firmwareは`origin/feat/keymap-advanced-behaviors`へpush済みである。

## 14. 完了条件

- Firmware／Host／UI／Export／Restore／`.keymapに戻す`まで実装される。
- ZMK標準comboとの二重発火がない。
- ZMK相当のoverlap解決が確認できる。
- 32 combo／8 keysの境界と重複validationがHost／firmware双方で機能する。
- 通常キー、Encoder、Comboの保存／破棄／reset結果が分離表示される。
- RAM実測が記録され、目標超過時は設計を再評価する。
