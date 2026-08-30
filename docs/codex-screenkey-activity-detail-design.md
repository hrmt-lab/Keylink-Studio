# ScreenKey Turn内状態細分化設計

- 設計日: 2026-08-02
- 対象: Keylink Studio Host内部モデル、Codex App Server event adapter、Host Link v2
- 状態: Host／Firmware／Renderer実装、Host自動test、実機状態遷移確認が完了
- Firmware側repositoryはWSL上を正本とし、Windows上の同名フォルダは参照専用として扱う
- Firmware共通層の正本: `/home/onigiri/zmk-workspace/config/zmk-rawhid-app`
- Windows側`C:\01.keyboards\OriginalKeyboards\02.SW\zmk-rawhid-app`は読み取りだけに使用し、変更しない
- 現行互換性基準: `codex-cli 0.151.0`、experimental App Server schema SHA-256
  `31AE67BEB2C94CC9509F6A71968600062DC8C6D7FE45437ED3A9129838F4D2D9`
- 検証済み旧組み合わせとして`codex-cli 0.150.1`と`E9BAD0A20736E7D3ABA18C0F04BEF59856FB212AE21049FE17D786682203CFAE`、同一schemaの`codex-cli 0.149.1`／`0.149.0`、`codex-cli 0.147.0`と
  `BABFD5C98CD978DD858B4762CDFBC9FBA941E1A0E4053DE0050E4082AE1F075A`、`codex-cli 0.146.0`と
  `D3992FEC1398AFDBEC658DA2C720C6993FBF3C1CE4900785694D2196679EDDFC`も受理する
- `version_check_enabled = false`を既定とし、未知versionでも上記の検証済みschema hashと一致すれば起動する。
  `true`ではversion／schemaの正しい組み合わせを要求し、どちらのモードでも未知schemaは拒否する

## 1. 目的

現在の`WORKING`を上位状態として維持しながら、Turn中の処理を次の3種類へ細分化する。

| Turn内状態 | ScreenKey表示 |
|---|---|
| 推論・応答生成中 | 青色の外周をゆっくり呼吸 |
| コマンド／ツール実行中 | 現行の青い外周移動線 |
| Web検索中 | 現行の青い外周移動線 |

`WAITING_INPUT`はTurn内状態とは別の既存`activity_state`として扱い、オレンジ色の
外周をゆっくり呼吸させる。`WAITING_APPROVAL`、`COMPLETED`、`ERROR`、`AVAILABLE`、
`NONE`の意味は変更しない。

## 2. 設計原則

1. `activity_state`はセッション／Turn／要求待ち／終端を表す既存の上位状態として維持する。
2. Turn内の処理種別は新しい`work_phase`で表し、既存enumへ新しいactivity値を追加しない。
3. App Serverの構造化された`item/started`と`item/completed`だけを使用する。
4. command、tool、query、messageなどの本文から状態を推測しない。
5. 旧Firmwareには従来の6 byte Payloadだけを送り、表示とrevision挙動を変えない。
6. 承認／入力要求とTurn終端は`work_phase`より常に優先し、debounceしない。
7. ScreenKey固有の色、周期、animationはFirmware Rendererの責務とし、Host Linkへ含めない。

## 3. 現行実装との差

現行の`AiClientStateSnapshot`は次だけを保持する。

```rust
pub struct AiClientStateSnapshot {
    pub client_type: AiClientType,
    pub client_variant: AiClientVariant,
    pub session_active: bool,
    pub activity_state: AiActivityState,
    pub revision: u16,
}
```

`turn/started`で`WORKING`へ入り、承認／入力要求がなければ`turn/completed`まで
`WORKING`を維持する。Broker metadataには`item_id`があるが、itemの`type`と
`item/started`／`item/completed` lifecycleはReducerへ渡していない。

Codex CLI `0.146.0`の生成schemaでは、両notificationが`threadId`、`turnId`、
`item.id`、`item.type`を持つ。この構造化fieldを細分化の入力にする。

## 4. Host内部モデル

### 4.1 公開Snapshot

```rust
pub struct AiClientStateSnapshot {
    pub client_type: AiClientType,
    pub client_variant: AiClientVariant,
    pub session_active: bool,
    pub activity_state: AiActivityState,
    pub work_phase: AiWorkPhase,
    pub revision: u16,
}

#[repr(u8)]
pub enum AiWorkPhase {
    Unspecified = 0x00,
    Thinking = 0x01,
    Executing = 0x02,
    Searching = 0x03,
}
```

意味は次のとおり。

| 値 | 意味 | 表示fallback |
|---:|---|---|
| `0x00 UNSPECIFIED` | Turn中だが構造化itemだけでは分類できない | 旧`WORKING`の青い移動線 |
| `0x01 THINKING` | model側の推論、plan、応答生成 | 青い呼吸枠 |
| `0x02 EXECUTING` | command、tool、file変更などの実行 | 青い移動線 |
| `0x03 SEARCHING` | schemaで明示されたWeb検索 | 青い移動線 |

`UNSPECIFIED`を残すことで、Turn開始直後、未知item、将来schemaとの不一致でも
事実以上の推測をせず、現在の表示へ安全にfallbackできる。

### 4.2 Thread境界

Codex CLIの`/side`（`/btw`）は、親Threadを維持したまま一時的な別Threadを開始する。
Hostは`thread/fork` responseで子Threadを表示対象へ昇格し、親sessionを終了させない。
この切替は`NONE`を送らずに`AVAILABLE`へ遷移するため、ScreenKeyを一時的にも消灯させない。
以後、子ThreadのTurn／item eventを表示する。`/btw`から親へ戻るときは`thread/resume`を
伴わず親Threadの`turn/started`だけが届く場合があるため、開始したTurnのThreadを表示対象へ
自動で戻す。この切替も`NONE`を送らない。`thread/fork`と対応しない別Threadの
`thread/started` notificationは無視する。通常のsession置換は、Hostが送った明示的な
`thread/start`または`thread/resume`のresponseでのみ確定する。

### 4.3 Reducer内部状態

Reducerは公開Snapshotとは別に次を持つ。

```text
tracked_thread_id
tracked_turn_id
requests: request_id -> Approval | Input
active_items: item_id -> AiWorkPhase
observed_work_phase
published_work_phase
pending_work_phase + deadline
```

- `active_items`は追跡中Thread／Turnに属する既知itemだけを保持する。
- Turn開始、Turn終了、session置換／終了で必ずclearする。
- 同じ`item_id`の重複started／completedはidempotentに扱う。
- 別Thread／Turn、item ID欠落、未知typeは表示状態を変更しない。
- 公開work phaseだけが変わった場合は新しい`WorkPhaseChanged` change reasonを発行し、
  senderがbit 11対応deviceだけへ送れるようにする。

### 4.4 上位状態の優先順位

既存規則を維持する。

```text
WAITING_APPROVAL
  > WAITING_INPUT
  > WORKING + work_phase
  > AVAILABLE
```

- 未解消の承認要求が1件以上あれば`WAITING_APPROVAL`。
- 承認要求がなく入力要求が1件以上あれば`WAITING_INPUT`。
- 要求がなく追跡中Turnがあれば`WORKING`。
- 要求待ち中も`active_items`は更新するが、公開`work_phase`は`UNSPECIFIED`とする。
- 最後の要求が解消された時点で、現在の`active_items`からTurn内状態を再計算する。

### 4.5 複数itemの優先順位

複数の既知itemが同時にactiveな場合は次の順で1つへ集約する。

```text
SEARCHING > EXECUTING > THINKING > UNSPECIFIED
```

tool実行中にreasoning itemが残っていても移動線を表示し、Web検索が明示されていれば
内部状態は`SEARCHING`とする。`EXECUTING`と`SEARCHING`は現在同じ表示だが、Host内部と
wireでは区別し、将来Rendererが安全に表示を分けられる余地を残す。

## 5. Codex App Server event分類

対象は`codex-cli 0.146.0`のexperimental App Server schemaとする。

### 5.1 lifecycle

| method | 処理 |
|---|---|
| `item/started` | 対象Thread／Turnの`item.id`を分類して`active_items`へ追加 |
| `item/completed` | 対象Thread／Turnの`item.id`を`active_items`から削除 |

metadata抽出へ`item_type`を追加する。item本文、command、query、tool名、出力内容は
Broker eventへ保持しない。

### 5.2 item type対応表

| `item.type` | `AiWorkPhase` | 理由 |
|---|---|---|
| `reasoning` | `THINKING` | schemaが推論itemとして明示 |
| `agentMessage` | `THINKING` | model側の応答生成 |
| `plan` | `THINKING` | model側のplan生成 |
| `commandExecution` | `EXECUTING` | command実行 |
| `fileChange` | `EXECUTING` | file変更適用 |
| `mcpToolCall` | `EXECUTING` | MCP tool実行 |
| `dynamicToolCall` | `EXECUTING` | dynamic tool実行 |
| `collabAgentToolCall` | `EXECUTING` | agent操作tool実行 |
| `subAgentActivity` | `EXECUTING` | sub-agent処理 |
| `imageView` | `EXECUTING` | image tool処理 |
| `imageGeneration` | `EXECUTING` | image生成処理 |
| `sleep` | `EXECUTING` | interruptible tool処理 |
| `webSearch` | `SEARCHING` | schemaがWeb検索として明示 |

`userMessage`、`hookPrompt`、review mode境界、context compaction、未知typeは分類しない。

Codex CLI `0.146.0`には独立した`fileSearch` item typeがない。`rg`等のfile検索が
`commandExecution`やtool callとして通知された場合は`EXECUTING`とし、command文字列や
tool名から`SEARCHING`へ読み替えない。将来schemaに明示的な`fileSearch`相当typeが
追加された場合だけAdapterの対応表へ追加する。

## 6. debounce規則

上位`activity_state`とTurn終端の正確さを優先し、debounceは`work_phase`だけへ適用する。

1. `WAITING_APPROVAL`、`WAITING_INPUT`、`COMPLETED`、`ERROR`、`AVAILABLE`、`NONE`への
   遷移は即時。保留中のwork phase遷移を破棄する。
2. `EXECUTING`または`SEARCHING`への遷移は即時。短いtoolでも実行表示を落とさない。
3. `EXECUTING`／`SEARCHING`から`THINKING`／`UNSPECIFIED`への戻りは250 ms保留する。
   その間に次の実行itemが始まれば戻りを取消す。
4. `THINKING`と`UNSPECIFIED`の相互遷移は150 ms安定した場合だけ公開する。
5. 保留中に別候補へ変わった場合は、最新候補とdeadlineへ置き換える。
6. debounce後の実効値が公開値と同じならeventもpacketも発行しない。

時間値はHost単体testで仮想時刻を使って固定し、Rendererの呼吸周期とは分離する。
実機で不自然な残像がある場合は、wire enumを変えず時間値だけ調整できる。

## 7. Host Link v2表現

### 7.1 capability

未使用のbit 11を次に割り当てる。

```text
CAP_AI_CLIENT_WORK_PHASE = 1 << 11
```

- bit 11は`STATE_UPDATE`の`work_phase`末尾fieldを受理できることを示す。
- bit 11を広告するFirmwareはbit 10 `CAP_AI_CLIENT_STATE`も必ず広告する。
- bit 10のみのFirmwareは既存6 byte形式だけを受ける。
- Hostはcapabilityごとに送信先を分け、同じdeviceへ6 byteと7 byteを二重送信しない。

Host Link protocol versionはv2のままとする。これはcapabilityで明示的にgateされたPayload拡張であり、
既存deviceへ新形式を送らないためである。

### 7.2 Payload

先頭6 byteを変更せず、末尾へ1 byte追加する。

| Offset | Size | Field | 旧形式 | 詳細対応形式 |
|---:|---:|---|---|---|
| 0 | 1 | `client_type` | 同じ | 同じ |
| 1 | 1 | `client_variant` | 同じ | 同じ |
| 2 | 1 | `session_active` | 同じ | 同じ |
| 3 | 1 | `activity_state` | 同じ | 同じ |
| 4 | 2 | `revision` (`u16 LE`) | 同じ | 同じ |
| 6 | 1 | `work_phase` | なし | 追加 |

```text
CAP_AI_CLIENT_STATEのみ:
  payload_len = 6

CAP_AI_CLIENT_WORK_PHASEあり:
  payload_len = 7
```

packet type `STATE_UPDATE = 0xA0`、feature `AI_CLIENT = 0x0A`、op／flags `0x00`は変更しない。

### 7.3 組み合わせ検証

| `session_active` | `activity_state` | `work_phase` |
|---|---|---|
| false | `NONE` | `UNSPECIFIED` |
| true | `WORKING` | `UNSPECIFIED`～`SEARCHING` |
| true | `AVAILABLE`／要求待ち／終端 | `UNSPECIFIED` |

- 6 byte形式は`work_phase = UNSPECIFIED`としてdecodeする。
- 7 byte形式の未知`work_phase`はbase stateを破棄せず`UNSPECIFIED`へnormalizeし、診断logを残す。
- それ以外の既存`session_active`／`activity_state`不正は従来どおりpacket全体をrejectする。

### 7.4 revisionと送信規則

`revision`は既存の上位状態revisionとして維持する。

- session、`activity_state`が変わるとrevisionをincrementする。
- `work_phase`だけが変わる場合はrevisionを変えない。
- 詳細対応Firmwareはfingerprint／state equalityへ`work_phase`を含め、同じrevisionで
  `work_phase`だけが変わるpacketも新しい状態として受理する。
- 上位状態変化はbit 10のみのdeviceとbit 11対応deviceの両方へ送る。
- `work_phase`だけの変化はbit 11対応deviceだけへ送る。
- 5秒heartbeatは各deviceが対応する形式で同じsnapshotを再送し、animation timerを再開始しない。
- `COMPLETED`はHost側で15秒後に`AVAILABLE`へ遷移し、base revisionをincrementする。
  これにより期限切れ後のUSB再接続では`AVAILABLE`を再送し、緑枠を再表示しない。
- Renderer側の30秒one-shotは、Host切断時にも表示を消せるfallbackとして維持する。

この規則により、詳細を理解しない旧Firmwareへwork phase変化由来の新revisionを送らず、
旧`WORKING` animationが途中で不必要に再開始することを防ぐ。

## 8. Rendererへの契約

Rendererは`activity_state`を先に評価し、`WORKING`の場合だけ`work_phase`を見る。

| `activity_state` | `work_phase` | 表示 |
|---|---|---|
| `WORKING` | `THINKING` | 青い呼吸枠 |
| `WORKING` | `EXECUTING` | 青い移動線 |
| `WORKING` | `SEARCHING` | 青い移動線 |
| `WORKING` | `UNSPECIFIED` | 青い移動線（legacy fallback） |
| `WAITING_INPUT` | 必ず`UNSPECIFIED` | オレンジの呼吸枠 |
| その他 | 必ず`UNSPECIFIED` | 既存表示を維持 |

オレンジは`#F97316`とする。呼吸animationは20 frame、100 ms/frameの2秒周期で、
opacityを64→255→64の三角波として開始値64にする。Hostは色や周期を送らない。

## 9. 実装境界

### Host

- `JsonRpcMetadata`へ`item_type`を追加する。
- `CodexEventAdapter`で`item/started`／`item/completed`をsemantic eventへ変換する。
- `AiClientStateReducer`へactive item集約とwork phase debounceを追加する。
- `AiClientStateChangeReason`へ`WorkPhaseChanged`を追加し、detail-only送信を識別する。
- Snapshot、Tauri DTO、packet codecへ`work_phase`を追加する。
- Host Link senderをcapability別の6 byte／7 byte送信へ分ける。

### zmk-rawhid-app

- Firmware変更はWSL上の正本だけで行う。Windows上に同名フォルダが存在しても参照専用とする。
- `zmk-rawhid-app`の正本は`/home/onigiri/zmk-workspace/config/zmk-rawhid-app`である。
- Windows側`C:\01.keyboards\OriginalKeyboards\02.SW\zmk-rawhid-app`は今後も変更しない。
- capability bit 11、7 byte decode、`work_phase` validation／normalizationを追加する。
- Core state、fingerprint、ZMK eventへ`work_phase`を追加する。
- 6 byte legacy packetを`UNSPECIFIED`として受理し続ける。

### ScreenKey Renderer

- `WORKING + THINKING`と`WAITING_INPUT`の呼吸animationを追加する。
- 既存の移動線、承認待ち、完了、エラー表示は維持する。

## 10. テスト要件

### Host単体test

- 全item typeの分類表。
- 別Thread／Turn、欠落ID、未知typeの無視。
- 複数active itemの優先順位。
- approval／input優先と、解消後のwork phase復帰。
- Turn／session終了時のactive item・timer clear。
- 150 ms／250 ms debounceと即時execution遷移。
- detail-only変化でbase revisionが変わらないこと。
- capability別6 byte／7 byte codec、tokenやitem本文をpacketへ含めないこと。
- 旧deviceへdetail-only packetを送らないこと。
- `COMPLETED`が30秒未満では維持され、30秒で一度だけ`AVAILABLE`へ遷移すること。
- 新しいTurn／session終了で`COMPLETED`期限timerが解除されること。

### Firmware test

- 6 byte legacyと7 byte詳細形式の両方を受理。
- bit 11広告条件とbit 10併記。
- 組み合わせ不正のreject、未知work phaseの`UNSPECIFIED` normalize。
- 同revision・異なるwork phaseを新eventとして受理。
- heartbeatでanimationを再開始しない。
- Coreのみ／Renderer 0件／Renderer 1件のcapability構成回帰。

### 実機確認

- 推論だけのTurn。
- command、file変更、MCP tool、Web検索を含むTurn。
- toolが短時間で連続するTurnと、推論へ戻るTurn。
- 入力要求、承認要求、解消後の元work phase復帰。
- 完了、失敗、キャンセル、CLI再接続、Raw HID再接続。
- 旧6 byte Firmwareと新7 byte Firmwareを各1台ずつ使った後方互換確認。

### 実施結果（2026-08-02）

- 通常Turn、推論／実行表示、入力待ち、承認待ち、承認／入力解消後の復帰を実機確認した。
- `COMPLETED`の30秒自動解除、期限後および実行中のUSB再接続を実機確認した。
- `/btw`（side chat）の子Threadで青い実行表示と緑枠を確認し、親Threadへ戻った後の
  次Turnでも同じ表示へ復帰することを確認した。繰り返し実行時の消灯固定、ロゴ固定、
  期限切れ緑枠の再表示は発生しなかった。
- Host単体testは207件、Tauri単体testは21件が成功した。formatおよび`git diff --check`も成功した。
- 旧6 byte Firmware実機との後方互換確認、複数Renderer、LED-only、BLE、非64-byte interfaceは
  引き続き対象外／`DEFERRED`とする。

## 11. 対象外

- command文字列、tool名、message本文からのfile検索推定。
- HostからRendererへ色、速度、frameを送ること。
- Windows／WSLや複数sessionの同時集約規則。
- 複数Renderer、LED-only、BLE、非64-byte interfaceの実機検証。
- 承認／入力の操作responseをHost Linkへ追加すること。

## 12. 完了と再開地点

この状態細分化の対象範囲に残作業はない。再開時は、§11の`DEFERRED`項目を別機能として
扱い、対応実機と検証条件を先に確定する。Firmware作業が必要な場合でも、正本は
`/home/onigiri/zmk-workspace/config/zmk-rawhid-app`であり、Windows側の同名フォルダは変更しない。
