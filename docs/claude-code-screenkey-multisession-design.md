# Claude Code状態表示・複数セッション前提設計

- 状態: Gate C一部実測済み／Claude Code再認証待ち
- 作成日: 2026-08-04
- Gate C更新日: 2026-08-05
- 対象: Windows版Keylink Studioから起動したClaude Code
- 基準環境: Claude Code `2.1.214`
- 関連仕様: `docs/keylink-studio-codex-screenkey-prototype-spec-reviewed-v10.md`

## 1. 目的

既存のCodex状態表示を壊さず、Claude Codeのセッション状態をKeylink Studioへ取り込み、
Host Link v2経由で表示できるようにする。

将来の複数セッション対応を前提に、状態取得、正規化、セッション管理、表示対象選択を分離する。
ただし、最初から複数セッションを同時表示するデバイスは要求しない。

## 2. 決定事項

1. 最初にClaude Code対応を実装するが、内部構造は複数セッションを前提にする。
2. 初期対象はKeylink Studioから起動したWindows上のClaude Codeだけとする。
3. WSL、外部から直接起動したClaude Code、SDK経由のセッションは初期対象外とする。
4. Keylink Studioがobserver pluginを生成し、`claude --plugin-dir <plugin>`で読み込ませる。
5. `SessionStart`だけcommand hookとHelperを使い、それ以外はHTTP hookを使う。
6. hookは観測専用とし、Claude Codeの判断、許可、入力内容を変更しない。
7. 複数セッション表示の開発中だけ、ScreenKey押下で表示セッションを切り替える。
8. 検証用切り替え機構はfeature gate付きの独立moduleに隔離し、恒久的な
   `HostActionKind`や設定契約には追加しない。
9. FirmwareのClaude Code識別・ロゴ対応はHost実装とは別タスクにする。

## 3. 全体構成

```text
Keylink Studio
  ├─ Windows Host上の受信口を起動
  ├─ endpointとtokenを生成
  ├─ observer pluginを一時ディレクトリへ生成
  └─ Windows Terminalの既存ウィンドウへClaude Codeを起動
          │
          ├─ SessionStart
          │    └─ command hook → keylink-claude-hook.exe → HTTP POST
          │
          └─ その他のイベント
               └─ HTTP hook → Keylink Studio

Keylink Studio
  Hook Receiver
      ↓ bounded/non-blocking queue
  ClaudeEventAdapter / Normalizer
      ↓ canonical AI activity event
  Session Registry
      ↓ selected session
  AI Client State送信
      ↓ Host Link v2
  Firmware / Renderer
```

Claude Code固有の並行HTTP配送、重複、順序逆転はAdapter／Normalizerで吸収する。
Codexの単一Brokerストリームを処理する共通ReducerへClaude固有の配送事情を持ち込まない。

## 4. 起動とobserver plugin

### 4.1 起動順序

1. Keylink Studioが受信口を起動する。
2. loopback endpoint、起動単位の高エントロピーtoken、`launch_id`を確定する。
3. 一時pluginディレクトリへplugin定義、`hooks.json`、`observer.json`を書く。
4. `claude --version`を検査する。
5. PowerShell wrapper経由で`claude --plugin-dir <絶対パス>`を起動する。
6. wrapperは起動時にendpoint、token、`launch_id`をメモリへ読み込む。
7. Claude Code終了時、wrapperの`finally`から終了通知を送る。

既存Codexランチャーと同様、Windows Terminalは`wt.exe -w 0 new-tab`を使う。
新しいタブは既存Windows Terminalプロセスへ委譲されるため、Claude Codeプロセスを
Keylink Studioの子孫としてJob Objectで追跡する設計にはしない。

### 4.2 pluginの内容

一時pluginディレクトリに置くのは設定ファイルだけとする。

- `.claude-plugin/plugin.json`
- `hooks/hooks.json`
- `observer.json`

`keylink-claude-hook.exe`は毎回コピーせず、Keylink Studioのインストールディレクトリに置く。
command hookは絶対パスと`args: []`を指定するexec formとし、シェルの引用規則へ依存しない。
HelperはClaude Codeから継承する`CLAUDE_PLUGIN_ROOT`を使って`observer.json`を特定する。

### 4.3 endpointとtoken

初期Windows実装では`127.0.0.1`へbindする。
tokenはplugin生成時に`hooks.json`／`observer.json`へ埋め込む。
Windows Terminalの既存プロセスへ委譲されても、親プロセスの環境変数継承へ依存しないためである。

token、Authorization header、hook本文の機密値はログへ出さない。
一時ディレクトリはユーザー専用権限とし、終了後に削除する。

### 4.4 一時ディレクトリの削除

`SessionEnd`受信直後には削除しない。wrapperが終了通知を送る前に`observer.json`が消える競合を
避けるため、wrapperは起動時に必要情報をメモリへ保持し、削除は次のいずれかで行う。

- wrapper終了通知の受信後
- 終了通知が来ない場合の猶予付きcleanup
- Keylink Studio次回起動時の古い一時ディレクトリcleanup

SessionEndとwrapper終了通知は冪等に処理する。

## 5. hook登録方針

### 5.1 初期登録イベント

| イベント | 経路 | 用途 |
|---|---|---|
| `SessionStart` | command Helper | セッションの登録／再開／clear／compact観測 |
| `UserPromptSubmit` | HTTP | Turn開始 |
| `PreToolUse` | HTTP | tool開始とwork phase判定 |
| `PostToolUse` | HTTP | tool正常終了 |
| `PostToolUseFailure` | HTTP | tool失敗／中断候補 |
| `Notification` | HTTP | permission、elicitation、idle候補の観測 |
| `Stop` | HTTP | Turn正常終了 |
| `StopFailure` | HTTP | Turn異常終了 |
| `SessionEnd` | HTTP | セッション終了 |

`SessionStart`はHTTP hook非対応であるためHelperを使う。それ以外では、tool呼び出しごとの
Windowsプロセス起動を避けるためHTTP hookを使う。

### 5.2 decision能力の隔離

`PermissionRequest`、`Elicitation`だけでなく、`PreToolUse`、`Stop`など複数のhookには
処理をblockまたは変更できる応答能力がある。イベント単位の注意だけに依存せず、受信口を
構造的に観測専用にする。

- 正常受信時は常に空bodyの`204 No Content`
- JSON decision bodyを生成できる型をReceiverへ持たせない
- queue投入失敗や内部処理失敗もdecision bodyへ変換しない
- command Helperはstdinを転送し、stdoutへ何も書かず、block用exit codeを返さない

初期の恒久設定では`PermissionRequest`と`Elicitation`を直接登録せず、決定能力を持たない
`Notification`の次のmatcherを優先する。

- `permission_prompt`
- `elicitation_dialog`
- `elicitation_complete`
- `elicitation_response`
- `idle_prompt`

Gate Cでは診断目的に限り、`PermissionRequest`／`Elicitation`も同じ観測専用Receiverへ登録して
Notificationとの発火条件を比較する。Notificationだけで必要情報を満たせる場合は登録しない。

## 6. timeoutと受信口

timeoutはClaude Code停止時間の上限を決めるものであり、受信口のhungを許容可能にするものではない。
timeout超過はnon-blocking errorとなるが、配送済みか未配送かをHostから断定できない。
Receiverがqueueへ投入した後にClaude Code側だけが待機を終了する場合もあるため、
重複・遅延配送を前提にする。

初期値は次とする。

| イベント | timeout |
|---|---:|
| `PreToolUse`／`PostToolUse`／`PostToolUseFailure` | 1秒 |
| `Notification` | 1秒 |
| `UserPromptSubmit` | 2秒 |
| `Stop`／`StopFailure` | 3秒 |
| `SessionEnd` | 1秒 |
| `SessionStart` Helper | 2秒 |

`SessionEnd`は共有実行予算があるため1秒とし、長い猶予が必要になった場合は
`CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS`を含めてGate Cで再評価する。

Receiverは次の順序だけを同期的に実行する。

1. token検証
2. body size上限と必須項目の最小検証
3. bounded queueへのnon-blocking投入
4. 空bodyの204応答

queue満杯時に待機しない。lifecycle／終端イベント用の予約容量と、tool／Notification詳細用の
通常容量を分ける。通常容量のoverflowは古い詳細イベントを破棄する。予約容量も使い切った場合は
sessionを`desynchronized`として記録し、同一sessionの最新lifecycle snapshotを優先する。
overflow記録はアトミックカウンタとレート制限ログだけにし、ファイルログ書き込みを
受信処理の完了条件にしない。

## 7. Adapter、Normalizer、Reducer

### 7.1 canonical eventへの変換

Claude固有JSONをAdapterで次の共通的な意味へ変換する。

- session upsert／end
- turn start／complete／failure
- work item start／complete／failure
- approval waiting
- input waiting
- idle candidate

未知フィールドは無視し、必須フィールド欠落はそのイベントだけを破棄して診断へ記録する。
hookのraw JSONを共通Reducerへ渡さない。

### 7.2 toolイベントの順序逆転

HTTP hookは並行到着するため、同じ`tool_use_id`で完了が開始より先に処理される可能性がある。
Normalizerは`session_id + tool_use_id`をキーに短寿命tombstoneを保持する。

- 未知itemへのPost／Failureを完了済みtombstoneとして記録する
- 遅れて同じitemのPreが来たら無視する
- 重複イベントはitem単位で冪等にする
- tombstoneは時間／件数上限付きで削除する

`prompt_id`は利用できる場合の診断・相関情報とするが、tombstoneの必須キーにはしない。

### 7.3 overflowとdesynchronized

overflow、必須イベント欠落、矛盾する順序を検出したセッションは`desynchronized`として記録する。
tool詳細だけが信用できない場合は、Turnが継続中である事実を保ったまま
`WORKING + UNSPECIFIED`へ縮退する。

`UNSPECIFIED`縮退は詳細状態の不整合に限定する。SessionEnd欠落やTurn終了不明を
`UNSPECIFIED`だけで解決したことにはしない。

## 8. 状態遷移

| Claude Codeイベント | Host側の基本処理 |
|---|---|
| `SessionStart` | Registryへupsert。Turnがなければ`AVAILABLE` |
| `UserPromptSubmit` | Turn開始、`WORKING + THINKING` |
| `PreToolUse` | active item追加。tool種別から`EXECUTING`／`SEARCHING` |
| `PostToolUse` | 該当active itemを削除。Turn継続中なら`THINKING` |
| `PostToolUseFailure` | 該当active itemを削除し、失敗／中断候補を記録 |
| `Notification: permission_prompt` | `WAITING_APPROVAL` |
| `Notification: elicitation_dialog` | `WAITING_INPUT` |
| `Stop` | `COMPLETED` |
| `StopFailure` | `ERROR` |
| `SessionEnd` | Registryからretire |

tool分類はClaude Codeの構造化されたtool名・イベントだけを使い、回答本文から推測しない。

## 9. 中断、欠落、stale状態

Esc中断では`Stop`／`StopFailure`が必ず届くとは仮定しない。
また、Postイベント欠落時はactive itemや待機状態が残るため、単純な無イベント時間だけで
正常な長時間buildとstale状態を区別できない。

したがって、一定時間イベントがなければ無条件に`AVAILABLE`へ落とすフラットwatchdogは採用しない。
watchdogという名称を使う場合も、詳細状態のstale化と、positive signalによるTurn終了確定を
別の機構として実装する。

回復処理を次の2種類に分ける。

### 9.1 詳細状態のstale化

active itemやapproval/input待ちが長時間更新されない場合、確定していない終了を推測せず、
必要に応じて`WORKING + UNSPECIFIED`へ縮退する。閾値はGate Cの実測後に決める。

### 9.2 Turn終了の確定

`AVAILABLE`／`COMPLETED`へ落とすにはpositive signalを要求する。

- `Stop`／`StopFailure`
- 中断直後に安定して届くことを確認できた`idle_prompt`
- wrapper／SessionEndによるセッション終了
- 将来確認された別の明示的終了イベント

`idle_prompt`は即時解除の主経路と仮定しない。Gate Cで発火有無と遅延を測り、
1～2秒程度なら主経路、数十秒なら補助、発火しないなら不採用とする。

中断時の`PostToolUseFailure`はtool終了のpositive signal候補だが、単独ではTurn全体の終了を
証明しない。active itemを解除した後も、別のTurn終了シグナルを待つ。

## 10. Session Registry

Registryの主キーは観測した`session_id`とする。各entryは少なくとも次を持つ。

- `session_id`
- `launch_id`
- 安定した登録順序
- 現在のsnapshot
- active itemsとtombstone
- desynchronized状態
- 最終イベント時刻
- 終了／retire状態

`/clear`、`/compact`、`/resume`、forkで`session_id`が維持されるとは仮定しない。
同じ`launch_id`から新しい`session_id`が開始された場合は、実測したイベント順序に従って
旧entryをretireまたは置換し、孤児entryを増やさない。

Keylink Studio再起動時にRegistryを永続化しない。検証UIから手動削除できるようにする。
終了を確認できないAVAILABLE entryは、長時間無活動なら表示候補から外してよいが、
終了済みと断定して自動削除しない。

Windows Terminal経由で起動したClaude CodeをKeylink Studioの子孫プロセスとして追跡できるとは
仮定しない。SessionEnd欠落の補償はwrapper終了通知、手動削除、再起動時Registryクリア、
表示候補からの除外で行う。

各セッションは独立したReducerとrevisionライフサイクルを持つ。Reducer生成時はセッションごとに
`initial_revision()`を呼ぶが、乱数seedの一意性を切り替えの正当性には使わない。

## 11. 表示セッションの選択

開発中の一時機能として、ScreenKey押下ごとに表示対象を次のsessionへ切り替える。

- 並び順は登録順を基本とし、同順位では`session_id`で安定化する
- 現在のsessionが終了したら次の有効sessionを選ぶ
- sessionがなければセッションなしを送る
- 5秒heartbeatは選択中sessionのsnapshotを送る
- 切り替え判定はsnapshotだけでなく
  `(selection_epoch, selected_session_id, snapshot)`の変化で行う
- 選択変更時は状態変化がなくてもfull送信する

wire packetには`session_id`がない。同一内容の2セッション間では画面が変化しないが、
Host内部の選択と以後のheartbeat／状態変化の送信元は切り替わる。検証UI／ログで現在の選択を確認する。

### 11.1 revision

Firmwareは到着順latest-winsで、同一revisionでもpayloadが変われば更新を受理する。
したがって選択切り替えの正しさをrevisionの偶然の不一致へ依存させない。

- revisionは各sessionのReducerが独立所有する
- `selection_epoch`はHost内の送信トリガでありpacketへ載せない
- 同一revision・異なるpayloadもfull送信する
- 同一revision・同一payloadで表示が変わらないのは正常とする

### 11.2 coalescing

Reducer適用／Registry更新とHID送信を分離する。HID遅延で入力queueの消費を止めない。
Host Link出力側はlatest-winsを基本とするが、送信理由を次のようにマージする。

- マージ対象に上位状態変化が1件でもあれば上位状態変化として送る
- 全件が`WorkPhaseChanged`のときだけ詳細変更として送る

これにより、work-phase capabilityだけを持たない旧デバイスが上位状態変化を取りこぼさない。
`COMPLETED`は、直後に実際の新Turnが始まった場合は省略可能だが、単なるHID backlogだけを理由に
未送信の終端状態を消さない。

## 12. 対応バージョン

初期の最小対応Claude Codeを`2.1.214`とする。これはhook仕様から導かれる絶対下限ではなく、
Gate Cと初期実装をこの環境で検証し、未検証の旧版をサポート対象へ含めないという実務判断である。

- 最小対応版未満: 起動前に拒否する
- 検証済み範囲: 通常起動する
- 最終検証版より新しい版: 警告付きで許可する
- 必須契約が欠ける場合: Claude Code本体は継続し、そのsessionの観測だけを安全に無効化する

Codex App Serverはexperimental schemaとhashへ強く依存するため、対応version／schema不一致を拒否する。
Claude Code hooksは文書化されたJSONの限定的なfieldだけをforward-compatibleに読むため、
最小版＋新しい版への警告を採用する。この非対称性を意図した互換性方針として記録する。

`prompt_id`や`SessionStart.source = fork`は利用できる場合に活用するが、初期設計の成立条件にはしない。

## 13. Firmware境界

Host側のClaude Code実装だけでは現行ScreenKeyへClaudeロゴを表示できない。
Firmware側はWSL上の正本で別タスクとして、少なくとも次を行う。

1. `RAWHID_APP_AI_CLIENT_CLAUDE_CODE = 0x02`を追加する
2. packet decoderのclient type検証を拡張する
3. `ai_client_state_model.c`側のstate validationも拡張する
4. capability bit 12を`CAP_AI_CLIENT_CLAUDE_CODE`として追加する
5. RendererごとにClaude Codeを表現できる場合だけcapabilityを立てる
6. ScreenKey用96×96 RGB565 ClaudeロゴとRenderer切り替えを追加する

capabilityの意味は「decoderが値を通す」だけではなく、対象Renderer方針でClaude Codeを
表現できることである。LED-onlyなどでは専用ロゴがなくても定義された表現を持つ場合に対応とする。

Firmware対応前のHost状態遷移確認では、検証ビルドに限り`client_type = Codex`で送信できるようにし、
Claude Code識別そのもののPASSとは区別する。

## 14. Gate C

実装前または最小probeで次を実測し、イベント対応表と閾値を確定する。

### 14.1 hookと起動

- `--plugin-dir`のhookが期待どおり読み込まれ、`defaultEnabled`等の影響を受けないこと
- plugin更新時の反映と`/reload-plugins`要否
- Windows初回、OS再起動直後、アンチウイルス動作時を含むSessionStart Helper所要時間分布
- command hook exec formと空白を含む絶対パス
- Keylink Studio停止中のconnection refusedでClaude Codeが継続すること
- Receiverが応答しないhung／遅延注入時に明示timeoutが効くこと
- 重いbuildと連続tool実行時の送信数／受信数／timeout率

### 14.2 受信queue

- 極小bounded queueで投入が待たされないこと
- overflow記録がatomic counterのみで受信をblockしないこと
- Reducer適用とHID送信が分離され、HID遅延中も入力をdrainできること

### 14.3 イベント意味論

- `PermissionRequest`と`Notification: permission_prompt`の発火条件比較
- 自動承認時にpermission待ち表示を誤点灯しないこと
- `Elicitation`と関連Notificationの発火条件比較
- 対話CLIで`AskUserQuestion`相当がどの経路を使うか
- Esc中断直後に届く全イベントと各遅延
- 長時間tool実行中の中断で`PostToolUseFailure`が届くか
- toolなし推論中、approval待ち、input待ちの各中断
- `idle_prompt`の発火有無と遅延分布
- 並列toolでPre／Post／Failureの到着順逆転と重複
- 通常失敗とユーザー中断を区別できる構造化fieldの有無

### 14.4 session lifecycle

- startup、`/clear`、手動／自動`/compact`、`/resume`、forkのイベント順序
- 各操作前後の`session_id`、`source`、SessionEnd `reason`
- 同じ`launch_id`内でsession IDが変わった場合の置換動作
- SessionEndとwrapper終了通知の順序・重複・片方欠落
- pluginがSDK等の意図しないsessionにも適用されないこと

### 14.5 出力と切り替え

- 同じrevision・異なるpayloadの2sessionを切り替えられること
- 同じrevision・同じpayloadでもHost内部の選択、heartbeat送信元が変わること
- 選択変更だけでfull送信されること
- 上位状態変化とwork phase変更をcoalesceしても旧device向け送信を失わないこと
- 表示中session終了時に次sessionまたはセッションなしへ移ること

### 14.6 2026-08-05実測結果

Gate C用の独立probeを
`crates/rawhid-host-core/examples/claude_hook_probe.rs`へ追加した。製品API、設定契約、
`AiClientType`、`HostActionKind`、Host Linkは変更していない。probeは次を1 binaryで扱う。

- loopback HTTP Receiverと一時observer plugin生成
- `SessionStart` command Helperとその他イベントのHTTP hook
- response遅延、socket drop、connection refused、極小queue、合成event flood
- Gate C専用stdio MCP elicitation fixture
- `target/claude-gate-c/<run-id>/`への未追跡JSONL証跡

基準環境はClaude Code `2.1.214`。通常権限で`--init-only`を実行すると、Claude Codeが
ユーザー領域のplugin data／session環境を作成できず`EPERM`になった。これはmanaged sandboxの
制約であり、通常ユーザー権限で再実行した結果は次のとおり。

| 項目 | 結果 |
|---|---|
| `--plugin-dir`からのplugin／19 hooks登録 | PASS |
| `SessionStart` command Helper | PASS。`source = startup` |
| `SessionEnd` HTTP hook | PASS。`reason = other` |
| startupからendまでのsession identity | PASS。同じ`session_id` |
| Receiver正常応答 | PASS。2件受信、2件accepted、overflow 0、Claude exit 0 |
| 1.5秒response遅延 | PASS。SessionEndはhook cancel表示になるがClaude exit 0 |
| responseを返さないsocket drop | PASS。hook error表示になるがClaude exit 0 |
| connection refused | PASS。event受信0件でもClaude exit 0 |
| queue容量1＋100件同時投入＋writer 100 ms遅延 | PASS。102件受信、4件accepted、通常queue overflow 98件、priority overflow 0、Claude exit 0 |
| MCP initialize／tools/list／`elicitation/create`／accept応答 | fixture単体PASS |
| probe test | 5件PASS |
| Host core回帰 | 207件PASS |
| Tauri回帰 | 21件PASS |

Receiverは正常時に空bodyの204だけを返し、fault時もClaude Codeをblockまたは変更しない。
極小queueでは通常`PreToolUse`を破棄しつつ`SessionStart`／`SessionEnd`をpriority queueで保持できた。
したがって、受信処理をbounded／non-blockingにし、lifecycle用予約容量を分ける方針は成立する。

一方、Claude Codeの認証状態は`loggedIn = false`、`authMethod = none`であり、debug logでも
OAuth refresh token無効を確認した。ログインはユーザーの認証状態を変更するため、この作業では
実施していない。次は再認証後に同じprobeで以下を実測する。

- toolなしTurn、tool成功／失敗／並列実行、重い処理、連続tool
- permission許可／拒否／自動承認とNotification比較
- input待ちとMCP elicitationのClaude Code end-to-end
- 推論中、tool中、approval／input待ちのEsc中断と`idle_prompt`
- `/clear`、手動／自動`/compact`、`/resume`、forkのsession identity
- plugin再生成後の反映と`/reload-plugins`要否

再認証後の最初の確認は次から開始する。promptとtool入力には合成データだけを使う。

```powershell
cargo run -q -p rawhid-host-core --example claude_hook_probe -- run `
  --claude C:\Users\Onigiri\.local\bin\claude.exe `
  --project C:\01.keyboards\OriginalKeyboards\02.SW\Keylink-Studio `
  --print "Gate C synthetic test. Reply with exactly GATE_C_OK and do not use tools."
```

MCP elicitation確認時は`--with-mcp`を追加し、対話sessionでは`--print`を外す。

上記が未実施のため、イベント対応表、終了positive signal、stale閾値、production timeoutは
まだ確定しない。OS再起動直後と実アンチウイルス負荷はユーザー合意により今回のGate Cから
`DEFERRED`とする。

## 15. 実装順序

1. Gate C用probeは実装済み。Claude Code再認証後にモデル依存イベントとversion依存を確定する。
2. observer plugin生成、Receiver、Helper、wrapper終了通知を実装する。
3. ClaudeEventAdapter／Normalizerとtombstoneを実装する。
4. Session Registryを導入し、既存Codexを含む複数session構造へ寄せる。
5. feature gate付きの一時的なScreenKey切り替え機構を実装する。
6. Host出力のcoalescing、heartbeat、selection triggerを実装する。
7. 暫定Codex identityで状態遷移を実機確認する。
8. WSL正本側でFirmwareのClaude Code識別、capability、ロゴを実装する。
9. Claude Code identityでend-to-end確認する。

## 16. 未確定事項

次はGate C結果が出るまで確定扱いにしない。

- Esc後にTurn終了を確定できるpositive signal
- `idle_prompt`の主経路／補助／不採用
- 詳細状態stale化の閾値
- `/clear`／`/compact`前後のsession identity
- Notificationだけでpermission／elicitation待ちを十分に表現できるか
- 初期timeout値の負荷下での実効性
- 最終検証済みClaude Codeバージョン

## 17. 非対象

- 初期段階でのWSL Claude Code
- Keylink Studio外から起動したsessionの自動発見
- ScreenKeyからClaude Codeのpermission／inputへ回答する機能
- 複数sessionの同時表示
- 一時切り替え操作の正式な設定契約化
- Claude Codeの回答本文解析
- Firmware変更をHost実装と同一タスクで完了扱いにすること

## 18. 参照

- Claude Code公式Hooks reference: <https://code.claude.com/docs/en/hooks>
- Claude Code公式Plugins: <https://code.claude.com/docs/en/plugins>
- 既存Codex activity実装: `crates/rawhid-host-core/src/codex_activity.rs`
- 既存Host Link送信実装: `crates/rawhid-host-tauri/src/commands.rs`
- 既存Windows Terminal起動実装: `crates/rawhid-host-tauri/src/codex_launcher.rs`
