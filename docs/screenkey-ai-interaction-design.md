> **この文書は置き換えられました。** 現行の正本は [ScreenKey と HUD による AI 承認・回答 設計](ai-approval-hud-design.md) です。
> 本書は当時の調査記録として残しています。引き継いだ判断と破棄した判断の対応は同文書 §16 にあります。

# ScreenKeyによるAIセッション前面化と順序ベース回答

## 1. 文書の位置付け

本書は、複数ScreenKeyへ表示しているCodex／Claude Codeセッションについて、
待機中のScreenKeyから対応Terminalを前面化し、続くScreenKey押下をAIへの回答として
扱う後続機能の設計案である。

2026-08-24時点では**設計のみ**であり、Keylink Studio、Host Link、Firmwareのいずれにも
本機能は実装されていない。既存の複数セッション表示、論理`display_slot`、
`cycle_ai_session`の実装済み範囲は変更しない。

関連文書:

- [Codex／Claude Code共通 ScreenKeyセッション切替仕様](ai-session-display-switching.md)
- [複数 ScreenKey への AI session 表示](ai-display-slot-multiscreen-host-design.md)
- [Codex複数セッション対応仕様](codex-multisession-design.md)
- [Claude Code複数セッション対応設計](claude-code-screenkey-multisession-design.md)
- [Host Link v2 packet spec](packet-spec.md)

本書は、上記文書で非対象としていた「ScreenKeyからapproval／inputへ回答する機能」を
具体化する。従来案として検討した「4つのScreenKeyへ選択肢本文を表示する方式」は
将来拡張として残すが、最小構成では採用しない。選択肢本文は前面化したAIクライアントの
Terminalで確認し、ScreenKeyは選択肢の位置だけを指定する。

## 2. 対象ハードウェアと前提

主対象は次の構成である。

- ScreenKey 4個
- 通常キー 7個
- エンコーダ 1個
- 同時に最大4件のAIセッションを論理`display_slot 0..=3`へ表示
- 左から右へ並ぶScreenKeyを物理選択番号`0..=3`へ対応付け

Firmware keymapでは、左端のScreenKeyから順に`HOST_ACTION.value = 0, 1, 2, 3`を
送るよう設定する。Hostは通常状態ではこの値を対象`display_slot`として扱い、
回答待機状態では選択肢indexとして扱う。

Host actionの既存制約を維持する。

- device単位の許可リスト制
- 既定disabled
- 監視中のみ実行
- 未定義action ID、未設定deviceは実行しない
- 同一`seq`の重複packetは1回として扱う
- `value`をpath、command、自由入力文字列として解釈しない

## 3. 目的と操作フロー

対象となる待機表示は次の2種類である。

- `WAITING_APPROVAL`: 黄色点滅
- `WAITING_INPUT`: オレンジ呼吸明滅

ユーザー操作は次のとおりとする。

1. 待機中のセッションを表示しているScreenKeyを押す。
2. Keylink Studioがその`display_slot`へ現在割り当てているAIセッションを特定する。
3. 対応する専用Windows Terminalウィンドウを前面化する。
4. Hostは、その時点で対象セッションが持つ未解決requestと選択肢順を固定する。
5. 4つのScreenKeyを一時的に、左から「選択肢1、2、3、4」として扱う。
6. ユーザーは前面化したTerminalで選択肢本文を確認する。
7. 選択肢に対応するScreenKeyを押す。
8. Hostは上から同じ位置にある回答データを要求元AIクライアントへ返す。
9. request解決後、回答待機状態を解除して通常のセッション操作へ戻る。

例えばTerminal上の選択肢が次の順なら、意味をHostやFirmwareで固定しない。

```text
1. 許可
2. セッション中は許可
3. 拒否
4. 拒否して中止
```

対応は常に位置だけで決める。

| ScreenKey | 物理index | 回答する選択肢 |
| --- | ---: | --- |
| 左端 | 0 | 上から1番目 |
| 左から2番目 | 1 | 上から2番目 |
| 左から3番目 | 2 | 上から3番目 |
| 右端 | 3 | 上から4番目 |

## 4. 既存機能と追加する責務

### 4.1 既存機能

現行Keylink Studioには次がある。

- Codexの正規表示識別子`thread_id`
- Claude Codeの正規表示識別子`(launch_id, session_id)`
- Codex sessionごとの`owner_connection_id`
- 最大8個の論理`display_slot`
- slotごとのAuto／Pinned割当
- ScreenKeyからの`HOST_ACTION`
- Codex Brokerでの双方向WebSocket中継
- approval／inputの未解決状態検出
- Claude Code hook bodyの受信

### 4.2 現在不足しているもの

本機能には次の追加責務が必要である。

- AIセッションと専用Terminalウィンドウの対応
- Codex起動とBroker接続を結び付ける起動単位ID
- 回答可能な未解決requestの順序付き保持
- ScreenKeyの1回目と2回目の押下を区別するdevice単位の状態
- Codex Brokerから要求元connectionへresponseを代理送信する経路
- Terminal側で先に回答された場合の競合解消
- Claude Code用のdecision可能な応答経路

現在のCodex activity処理は、JSON-RPCのmethod、ID、`thread_id`、`turn_id`、
`item_id`などのmetadataを利用して待機状態を作るが、選択肢や回答用JSONを保持しない。
Claude Code側もhook body全体は受信するが、Reducerへはapproval／inputの開始・解消という
意味だけを渡し、回答内容を保持しない。

## 5. AIセッションごとの専用Terminalウィンドウ

### 5.1 起動方式

現行ランチャーはCodex／Claude Codeを次の形で既存Windows Terminalへ追加する。

```text
wt.exe -w 0 new-tab ...
```

同一Terminalウィンドウ内の特定tabだけを前面化するには、tab IDの取得、tab移動・閉鎖の
追跡、Windows Terminal固有の制御が必要になる。単に`WindowsTerminal.exe`のwindowを
前面化すると、別セッションのtabが表示される可能性がある。

本設計では、Keylink Studioから起動するAIクライアントを起動ごとに専用のWindows Terminal
ウィンドウへ変更する。各windowには、表示用project名とは別にHost内部で一意な
`terminal_target_id`を割り当てる。

```text
Keylink-Codex-<launch_id>
Keylink-Claude-<launch_id>
```

実際のwindow titleまたは検索tokenには推測困難な内部IDを使用し、project名だけでwindowを
識別しない。同じprojectから複数セッションを起動しても衝突させない。

### 5.2 前面化

Hostは起動後に対象windowのHWNDまたは同等の安定したwindow targetを確定する。
ScreenKeyの1回目押下では次を行う。

1. 最小化されていれば復元する。
2. 現在の最大化／通常状態を維持する。
3. 対象windowを前面化する。
4. windowが存在しなければ新規起動せず、回答待機状態へ入らない。

既存`app_launch`のexe名単位前面化は、複数のWindows Terminalを区別できないため
そのまま流用しない。`terminal_target_id`に対応するwindowだけを対象とする。

### 5.3 Codexの対応付け

Codex CLI起動時点では`thread_id`がまだ確定していない。さらに、複数CLIが同じBroker tokenを
使うだけでは、起動したTerminalと後から接続した`connection_id`を安全に相関できない。

起動ごとに次を発行する。

- `launch_id`
- 起動専用Broker credentialまたは同等の起動識別情報
- `terminal_target_id`

Broker接続時に次を確定する。

```text
launch_id
  -> connection_id
  -> owner thread_id
  -> terminal_target_id
```

同じ`thread_id`が別CLIから明示的にresumeされた場合は、Session Registryの
`owner_connection_id`移動に合わせて前面化対象も新しいTerminalへ移す。1つのCLIが複数threadを
扱う場合、それらは同じ`connection_id`とTerminalを共有してよい。

起動順だけでTerminalと接続を対応付ける方式は、並列起動や接続retryで誤相関するため採用しない。

### 5.4 Claude Codeの対応付け

Claude Codeは起動前に`launch_id`を発行しているため、次の対応をそのまま利用できる。

```text
(launch_id, session_id)
  -> terminal_target_id
```

同一launch内でsessionが変わってもTerminalは同じである。wrapper終了、window消失、明示停止時は
対応を退役させる。

## 6. 選択肢の取得と順序保持

### 6.1 共通原則

Hostは、選択肢の意味ではなく要求元protocolが持つ**順序**を正本とする。

- 配列順を並べ替えない。
- labelの辞書順や安全度で並べ替えない。
- 選択肢を日本語へ翻訳しない。
- Firmwareへ選択肢本文を送らない。
- ScreenKey押下後に別requestの選択肢へ差し替えない。
- request IDと選択肢配列を1回目の押下時に固定する。

Host内部では、回答に必要な値を不透明な`AnswerChoice`として保持する。

```text
PendingInteraction {
    client,
    session_target,
    owner_connection,
    request_id,
    request_kind,
    choices: Vec<AnswerChoice>,
    state,
}
```

`AnswerChoice`は可能な限り、元requestに含まれるresponse用JSON valueをそのまま保持する。
command approvalのpolicy amendmentのようなobjectも、Hostが内容を再構築せず元の値を使う。

### 6.2 Codex command approval

2026-08-24にローカル`codex-cli 0.149.0`から生成したexperimental App Server schemaでは、
`item/commandExecution/requestApproval`に`availableDecisions`がある。このfieldは
クライアントが提示できる判断の**順序付きリスト**である。

Hostは配列要素をそのまま保持する。

```text
choice[0] = availableDecisions[0]
choice[1] = availableDecisions[1]
choice[2] = availableDecisions[2]
choice[3] = availableDecisions[3]
```

選択後は概念的に次を送る。

```json
{
  "decision": "<選択したavailableDecisions要素そのもの>"
}
```

実際の要素はstringに限らずobjectの場合があるため、文字列化して再解釈しない。
`availableDecisions = null`または空の場合は、Terminalに選択肢が見えていても順序の正本がないため、
ScreenKey回答を有効にしない。

### 6.3 Codex file change approval

同schemaの`item/fileChange/requestApproval`には`availableDecisions`がない。response schemaには
`accept`、`acceptForSession`、`decline`、`cancel`が定義されているが、schema上のenum順と
Codex TUIがそのrequestで実際に提示した順が同一とは保証できない。

初期実装ではfile change approvalを順序ベース回答の対象外とする。対応する場合は、対象Codex
versionごとに実TUIとresponse順を互換性Gateで確認し、App Serverが明示的な順序を提供する場合を
優先する。推測した固定順で回答しない。

### 6.4 Codex permissions approval

`item/permissions/requestApproval`は要求されたpermission profileを含むが、通常の
`availableDecisions`配列ではない。responseもpermission profile、scope、
`strictAutoReview`を含む専用形式である。

初期実装では対象外とする。将来対応では、Terminalが提示する各行と完全なresponse objectの対応を
version別互換性Gateで確定してから`AnswerChoice`へ格納する。

### 6.5 Codex requestUserInput

`item/tool/requestUserInput`には順序付き`questions`があり、各questionの`options`も配列である。
各optionは`label`と`description`を持つ。

単一question、1～4個のoptionであれば直接対応できる。

```text
左端       -> options[0].label
左から2番目 -> options[1].label
左から3番目 -> options[2].label
右端       -> options[3].label
```

Codex 0.149.0のresponseはquestion IDごとの回答mapである。

```json
{
  "answers": {
    "<question id>": {
      "answers": ["<選択したoption label>"]
    }
  }
}
```

次は初期対象外とする。

- optionなしの自由入力
- `Other`選択後に文字列入力が必要なもの
- secret入力
- 複数questionを1回のresponseで要求するもの
- 5個以上のoption

複数questionは、questionごとに回答を保持して全question分を集めてから送る段階選択として
将来拡張できる。5個以上のoptionはエンコーダで4件単位にページ切替する案を採用候補とする。

### 6.6 MCP elicitation

MCP elicitationはform schemaからenum／single-select／multi-selectを取得できる場合がある。
単一selectかつ1～4択なら同じ順序ベース操作へ正規化できる。

次は対象外とする。

- 自由文字列、数値、秘密情報
- 複数fieldを持つform
- URL mode
- multi-select

### 6.7 Claude Code

現在のClaude Observerは`PermissionRequest`や`Elicitation`のhook bodyを受信するが、正常時は
空bodyの`204 No Content`だけを返す観測専用経路である。許可、拒否、入力内容をClaude Codeへ
返す型や待機機構を持たない。

したがって、初期実装でClaude Codeに提供できるのは「対応Terminalを前面化する」までである。
2回目のScreenKey押下を回答にするには、観測専用Receiverとは分離したdecision可能な
PermissionRequest／Elicitation経路と、実payload内の提示順を確認する互換性Gateが必要である。

Claude Codeの回答機能を、現在の204応答へ暗黙に追加しない。timeout、hook failure時の継続、
Keylink Studio停止時の挙動を別仕様で確定する。

## 7. ScreenKey入力状態機械

状態はkeyboard device単位に持つ。複数keyboardが接続されても、一方の操作で他方の
ScreenKeyを回答モードにしない。

```text
Idle
  -> Armed
  -> Resolving
  -> Idle
```

### 7.1 Idle

通常の`HOST_ACTION.value`は対象`display_slot`である。

押されたslotについて次を確認する。

1. 有効な`AiDisplayTarget`が割り当てられている。
2. 状態が`WAITING_APPROVAL`または`WAITING_INPUT`である。
3. 対象sessionに回答可能な未解決requestがある。
4. 選択肢が1～4件ある。
5. 現在のowner connectionとTerminal targetが有効である。

成立した場合だけTerminalを前面化し、`Armed`へ遷移する。Terminal前面化に失敗した場合は
requestへ回答せず、`Idle`を維持する。

### 7.2 Armed

`Armed`は次を固定する。

```text
ArmedInteraction {
    device_uid,
    source_display_slot,
    target_session,
    owner_connection,
    request_id,
    choices,
    armed_at,
    expires_at,
}
```

この状態で同じdeviceから届く`HOST_ACTION.value = 0..=3`を選択肢indexとして扱う。
範囲外、存在しない選択肢、別deviceからの入力は回答にしない。

1回目のpress packetを2回目の選択として再利用しない。既存`seq`重複排除に加え、
`Armed`遷移後に受信した新しいpressだけを受け付ける。

### 7.3 Resolving

2回目のpressを受けたら、response送信前に次をatomicに再確認する。

- requestがまだ未解決
- request IDが一致
- owner connectionが一致
- 選択肢indexが有効
- 他の回答処理がrequestを確保していない

requestを`Resolving`へ1回だけ遷移させ、要求元connectionへresponseを送る。
成功・失敗・外部解決のいずれでもdeviceの`Armed`を解除する。

### 7.4 自動解除

次の場合はAIへ何も回答せず`Idle`へ戻す。

- Terminal側で先に回答され、requestが解決した
- `serverRequest/resolved`を受信した
- Turn終了、session終了、owner connection切断
- Terminal window消失
- device切断
- Keylink Studioの監視停止または終了
- 30秒の無操作timeout
- 通常キーまたはencoder pressへ割り当てた明示cancel

timeout値30秒は初期案であり、実機操作で確定する。

## 8. Host LinkとFirmwareへの影響

### 8.1 最小構成

選択肢本文は前面化したTerminalに表示されるため、最小構成では新しいHost Link packetを
追加しない。

- Firmware -> Host: 既存`HOST_ACTION`
- `action_id`: 新しい組み込みaction、仮称`interact_ai_request`
- `value`: 左からのScreenKey index `0..=3`
- Host -> Firmware: 既存`AI_CLIENT_STATE`を維持
- capability bit追加: 不要
- Host Link protocol version変更: 不要

同じ2 byte uplinkを再利用するが、意味の切替はHostが保持するdevice単位`Idle／Armed`状態に
限定する。FirmwareはAI request ID、session ID、選択肢内容を保持しない。

### 8.2 任意の表示拡張

回答モードを明確にするため、将来は4画面へ大きく`1`、`2`、`3`、`4`だけを表示してもよい。
これは選択肢本文を送る機能ではない。

この表示が必要になった場合だけ、次のいずれかを別途設計する。

- 既存renderer内の一時回答モード
- capabilityでgateした小さなinteraction state downlink

初期実装の成立条件にはしない。

## 9. Codex Brokerの代理回答

現在のBrokerはCLIとApp Server間のmessageを中継し、activity側へmetadataを通知する。
ScreenKey回答では、requestを受け取った正しい上流WebSocket connectionへJSON-RPC responseを
送る経路が必要になる。

requestの正規キーは少なくとも次を含む。

```text
(connection_id, request_id, direction)
```

同じJSON-RPC IDは別connection、または逆方向requestで再利用され得るため、request ID単独で
照合しない。thread所有権も再確認する。

Hostがresponseを代理送信した後、Codex TUIが表示中promptを閉じるかはschemaだけから確定しない。
App Serverの`serverRequest/resolved`通知、同じconnection上の状態遷移、遅れて届くCLI responseを
実Broker／実TUIで確認する。

Terminal側とScreenKey側が同時に回答した場合は、先にatomic確保した1件だけを送る。
ScreenKey側が先に解決した後の遅延CLI responseを、App Serverへ二重配送しない。ただし、
CLI promptを消すために必要なnotificationまで抑制しない。

## 10. 成立範囲

2026-08-24時点の設計判定は次のとおりである。

| 対象 | Terminal前面化 | 順序ベース回答 | 初期対象 |
| --- | --- | --- | --- |
| Codex command approval、`availableDecisions` 1～4件 | 可能 | 可能 | 対象 |
| Codex `requestUserInput`、単一question、option 1～4件 | 可能 | 可能 | 対象 |
| Codex file change approval | 可能 | 提示順を取得できず未確定 | 対象外 |
| Codex permissions approval | 可能 | 専用変換が必要 | 対象外 |
| MCP単一select、1～4件 | 可能 | schema実測後に可能 | 後続 |
| 自由入力／secret入力 | 可能 | ScreenKeyだけでは不可 | 対象外 |
| 5件以上の選択肢 | 可能 | encoderページ切替が必要 | 後続 |
| 複数question | 可能 | 段階回答が必要 | 後続 |
| Claude Code approval／input | 可能 | 現Observerでは不可 | 後続 |

## 11. 事前互換性Gate

### Gate 0: Terminal所有権

- Codex／Claude Codeを起動ごとに専用Terminal windowへ起動できる。
- 同じprojectを4件起動しても一意に区別できる。
- ScreenKeyのslot 0～3から正しいwindowだけを復元・前面化できる。
- windowを閉じた後に別windowを誤って前面化しない。
- Codexの`launch_id -> connection_id -> thread_id`相関が並列起動でも混線しない。
- 同一threadのresume後は新ownerのwindowを前面化する。

### Gate 1: Codex代理response

- 実Codex TUIがapproval／inputを表示している状態でBrokerからresponseを送る。
- App Serverが正しいconnectionのresponseとして受理する。
- Codex TUIの待機表示が正常に閉じる。
- `serverRequest/resolved`または同等の解消signalを観測する。
- CLIから遅れて届くresponseを二重適用しない。
- 別connectionで同じrequest IDを使っても混線しない。

Gate 1でTUI promptが閉じない場合は、単純な代理response方式を実装しない。Brokerがrequestを
所有してCLIとScreenKeyの回答を調停する方式、またはApp Serverが提供する別のclient同期経路を
再設計する。Terminalへ矢印キーやEnterを`SendInput`で注入する方式は、focus race、UI変更、
誤入力の危険があるため代替案にしない。

### Gate 2: 選択順

- command approvalの`availableDecisions`順と実TUI表示順が一致する。
- string decisionとobject decisionの両方を不透明値として返せる。
- `requestUserInput.options`順と実TUI表示順が一致する。
- option labelをresponseへ戻してrequestが解決する。
- `availableDecisions = null`、option 0件、5件以上では回答モードに入らない。

### Gate 3: 4-ScreenKey実機

- 黄色点滅slotを押すと正しいCodex windowが前面化する。
- オレンジ呼吸slotを押すと正しいCodex windowが前面化する。
- 2回目の左端～右端が上から1～4番目へ対応する。
- 4セッションで同時に待機しても、1回目に選んだsessionだけへ回答する。
- Terminal側で先に回答すると回答モードが解除される。
- timeout、USB切断、session終了後のpressを回答として扱わない。
- `cycle_ai_session`、Auto／Pinned、通常のAI state表示を回帰させない。

## 12. 実装段階案

### Phase A: Codex最小構成

1. AIクライアントを専用Terminal windowで起動する。
2. Codex起動単位IDとBroker connectionを相関する。
3. `display_slot -> AiDisplayTarget -> terminal_target`を解決する。
4. `interact_ai_request` HOST_ACTIONを追加する。
5. command approvalの`availableDecisions`を順序付きで保持する。
6. 単一questionの`requestUserInput.options`を順序付きで保持する。
7. device単位`Idle／Armed／Resolving`を追加する。
8. Broker代理response Gateを通す。
9. 4-ScreenKey実機で受け入れ条件を確認する。

このPhaseではHost Link wire formatとFirmware Coreを変更しない。

### Phase B: 入力拡張

- encoderによる5件以上の選択肢ページ切替
- 複数questionの段階回答
- MCP single-select
- 回答モードの数字表示
- file change／permissionsのversion別互換性Gate

### Phase C: Claude Code

- decision可能なhook経路の設計
- PermissionRequest／Elicitationの実payload順序確認
- timeoutとKeylink Studio停止時の安全なfallback
- Codexと同じ`PendingInteraction`への正規化
- 実Claude Codeでの許可／拒否／入力E2E

## 13. 採用方針と未確定事項

### 採用方針

- 待機中ScreenKeyの1回目押下で対応Terminalを前面化する。
- AIクライアントは起動ごとの専用Terminal windowへ変更してよい。
- 2回目のScreenKey押下は、左から上位選択肢へ順番だけで対応させる。
- 選択肢本文をScreenKeyへ送らない。
- AI protocolが明示した配列順だけを正本とする。
- command approvalの回答値は意味を再構築せず不透明JSONとして保持する。
- 初期対象はCodex command approvalと単一questionの1～4択`requestUserInput`。
- Terminalへのキー入力注入は採用しない。
- Claude Codeの前面化と回答は別の成立境界として扱う。

### 実装前に確定する事項

- Windows Terminalの専用window作成と安定したwindow target取得方法
- Codex起動専用credentialの具体形式
- 回答モードのcancel操作
- 無操作timeoutの最終値
- 5択以上を初期版で無効化するか、encoderページ切替まで同時実装するか
- Gate 1でCodex TUI promptが閉じない場合の再設計
- Claude Code decision経路の仕様と対応version

以上を確定してGate 0～1を通すまでは、Host Link／Firmware変更へ進まない。
