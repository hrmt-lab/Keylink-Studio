# Claude Code状態表示・複数セッション前提設計

- 状態: Keylink Studio・Firmware・ScreenKey実装、自動検証、正式Claude Code identity実機E2E完了
- 作成日: 2026-08-04
- Gate C更新日: 2026-08-08
- 対象: Windows版Keylink Studioから起動したClaude Code
- 基準環境: Claude Code `2.1.224`（event意味論）、`2.1.226`（session lifecycle）
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

製品設定では`PermissionRequest`と`Elicitation`を観測専用Receiverへ直接登録する。
Gate Cで関連`Notification`が約6秒遅れて届くことを確認し、即時の待機表示には使えないためである。
Receiverは常に空bodyの204だけを返し、許可・拒否・入力内容を返すdecision経路を持たない。
`Notification`の次のmatcherは補助情報として併用する。

- `permission_prompt`
- `elicitation_dialog`
- `elicitation_complete`
- `elicitation_response`
- `idle_prompt`

`PermissionRequest`後にユーザーが許可した瞬間を示すhookはない。したがって、許可後を推測で
`EXECUTING`へ変えず、Post系eventまたは詳細状態stale化まで`WAITING_APPROVAL`を維持する。

## 6. timeoutと受信口

timeoutはClaude Code停止時間の上限を決めるものであり、受信口のhungを許容可能にするものではない。
timeout超過はnon-blocking errorとなるが、配送済みか未配送かをHostから断定できない。
Receiverがqueueへ投入した後にClaude Code側だけが待機を終了する場合もあるため、
重複・遅延配送を前提にする。

初期値は次とする。

| イベント | timeout |
|---|---:|
| `PreToolUse`／`PostToolUse`／`PostToolUseFailure` | 1秒 |
| `PermissionRequest`／`PermissionDenied`／`Elicitation`系 | 1秒 |
| `Notification` | 1秒 |
| `UserPromptSubmit` | 2秒 |
| `PreCompact`／`PostCompact` | 2秒 |
| `Stop`／`StopFailure` | 3秒 |
| `SessionEnd` | 1秒 |
| `SessionStart` Helper | 2秒 |

`SessionEnd`は共有実行予算があるため1秒とし、長い猶予が必要になった場合は
`CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS`を含めてGate Cで再評価する。
SessionStart Helperとwrapper終了通知の内部HTTP送信は1回500 ms、失敗時の再試行1回、
再試行前待機100 msとする。Helperは外側の2秒以内、wrapperは約1.1秒以内でbest-effort送信を終え、
配送失敗をClaude Codeの失敗へ変換しない。

Receiverは次の順序だけを同期的に実行する。

1. token検証
2. body size上限と必須項目の最小検証
3. bounded queueへのnon-blocking投入
4. 空bodyの204応答

queue満杯時に待機しない。lifecycle／終端イベント用の予約容量と、tool／Notification詳細用の
通常容量を分ける。通常容量のoverflowは満杯時に到着した詳細イベントを破棄する。予約容量も使い切った場合は
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
Normalizerは`session_id + tool_use_id`をキーに短寿命tombstoneを保持する。初期実装は
120秒TTL、最大256件とし、TTL経過または件数上限超過で古いものから削除する。

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
最後に受理した関連eventから120秒後に`WORKING + UNSPECIFIED`へ縮退する。
これはEsc押下をhookから直接検知できず、Esc後にPost／Stop系eventが欠落する場合の表示上の安全策である。
stale化してもsession、Turn、active item、approval/inputの内部管理情報は終了扱いにせず保持する。

### 9.2 Turn終了の確定

`AVAILABLE`／`COMPLETED`へ落とすにはpositive signalを要求する。

- `Stop`／`StopFailure`
- wrapper／SessionEndによるセッション終了
- 将来確認された別の明示的終了イベント

`idle_prompt`は正常な`Stop`から約60秒後に届く例があり、Esc直後には安定して届かなかったため、
即時解除の主経路にせず補助signalとしてだけ扱う。120秒のstale化もpositive signalではなく、
`AVAILABLE`／`COMPLETED`／`ERROR`への遷移やsession retireには使用しない。

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

`SessionEnd`は`launch_id + session_id`が一致する個別sessionだけをretireする。
wrapper終了通知は同じ`launch_id`に残る全sessionをretireし、`SessionEnd`欠落を補償する。
最初のretireだけが状態とrevisionを変更し、後続の重複通知はno-opとする。`/clear`の
`SessionEnd(reason = clear)`は旧sessionだけをretireし、launch自体は終了させない。
wrapper終了済み`launch_id`はKeylink Studio終了までtombstoneとして保持し、遅延eventで復活させない。

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

初期の最小対応Claude Codeを`2.1.224`とする。これはhook仕様から導かれる絶対下限ではなく、
Gate Cと初期実装をこの環境で検証し、未検証の旧版をサポート対象へ含めないという実務判断である。

- 最小対応版未満: 起動前に拒否する
- 検証済み範囲: `2.1.224`から`2.1.226`まで通常起動する
- 最終検証版より新しい版: 警告付きで許可する
- 必須契約が欠ける場合: Claude Code本体は継続し、そのsessionの観測だけを安全に無効化する

Codex App Serverはexperimental schemaとhashへ強く依存するため、対応version／schema不一致を拒否する。
Claude Code hooksは文書化されたJSONの限定的なfieldだけをforward-compatibleに読むため、
最小版＋新しい版への警告を採用する。この非対称性を意図した互換性方針として記録する。

`prompt_id`や`SessionStart.source = fork`は利用できる場合に活用するが、初期設計の成立条件にはしない。

## 13. Firmware境界

Host側のClaude Code実装だけではScreenKeyへClaudeロゴを表示できないため、Firmware側は
WSL上の正本で別タスクとして次を実装した。

1. `RAWHID_APP_AI_CLIENT_CLAUDE_CODE = 0x02`を追加する
2. packet decoderのclient type検証を拡張する
3. `ai_client_state_model.c`側のstate validationも拡張する
4. capability bit 12を`CAP_AI_CLIENT_CLAUDE_CODE`として追加する
5. RendererごとにClaude Codeを表現できる場合だけcapabilityを立てる
6. ScreenKey用96×96 RGB565 ClaudeロゴとRenderer切り替えを追加する

capabilityの意味は「decoderが値を通す」だけではなく、対象Renderer方針でClaude Codeを
表現できることである。LED-onlyなどでは専用ロゴがなくても定義された表現を持つ場合に対応とする。

Firmware対応前のHost状態遷移確認では暫定`client_type = Codex`を使った。正式対応後はHostが
`client_type = CLAUDE_CODE (0x02)`を送り、bit 10とbit 12を広告したdeviceだけを送信対象とする。
bit 12のない既存deviceへ未知のclient typeは送らない。

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

この時点のClaude Code認証状態は`loggedIn = false`、`authMethod = none`であり、debug logでも
OAuth refresh token無効を確認した。ログインはユーザーの認証状態を変更するため、この作業では
実施せず、次の項目を再認証後へ残した。

- toolなしTurn、tool成功／失敗／並列実行、重い処理、連続tool
- permission許可／拒否／自動承認とNotification比較
- input待ちとMCP elicitationのClaude Code end-to-end
- 推論中、tool中、approval／input待ちのEsc中断と`idle_prompt`
- `/clear`、手動／自動`/compact`、`/resume`、forkのsession identity
- plugin再生成後の反映と`/reload-plugins`要否

再認証後の最初の確認には次の合成データだけを使い、2026-08-08にPASSした。

```powershell
cargo run -q -p rawhid-host-core --example claude_hook_probe -- run `
  --claude C:\Users\Onigiri\.local\bin\claude.exe `
  --project C:\01.keyboards\OriginalKeyboards\02.SW\Keylink-Studio `
  --print "Gate C synthetic test. Reply with exactly GATE_C_OK and do not use tools."
```

MCP elicitation確認時は`--with-mcp`を追加し、対話sessionでは`--print`を外す。

### 14.7 2026-08-08再認証後のevent意味論実測

ユーザーによる再認証後、Claude Code `2.1.224`で対話probeを実行した。全runでReceiverの
`unauthorized`／`malformed`／`oversized`／通常・priority overflowは0だった。主な結果は次のとおり。

| ケース | 観測eventと結果 |
|---|---|
| toolなしTurn | `UserPromptSubmit -> Stop` |
| tool成功 | `PreToolUse -> PostToolUse -> PostToolBatch -> Stop` |
| tool失敗 | `PreToolUse -> PostToolUseFailure(error = Exit code 1) -> PostToolBatch -> Stop` |
| 並列tool | 2件の`PreToolUse`後、開始順A／Bに対して完了順B／A。対応は`tool_use_id`で取る |
| auto承認 | permission eventなしで`PreToolUse -> PostToolUse` |
| manual許可 | `PreToolUse -> PermissionRequest -> PostToolUse`。`PreToolUse`は許可確定前に届く |
| manual拒否／permission中Esc | `PreToolUse -> PermissionRequest`で停止し、Post／Stop系hookは届かない |
| `AskUserQuestion`回答 | `PreToolUse -> PermissionRequest -> Notification(permission_prompt) -> PostToolUse`。回答は`tool_response`に入る |
| `AskUserQuestion`中Esc | `PreToolUse -> PermissionRequest`で停止し、Post／Stop系hookは届かない |
| MCP elicitation accept | `ElicitationResult(action = accept, content)`と`Notification(elicitation_response)`が届く |
| MCP elicitation Esc | `ElicitationResult(action = cancel)`後にPost／Stopまで届く |

permission／elicitation用`Notification`は元eventから約6秒後に届いた。即時状態遷移は
`PermissionRequest`／`Elicitation`を使い、Notificationは補助とする。並列toolはevent順が逆転するため、
Adapterは到着順ではなく`tool_use_id`で対応付ける。

Esc中断の結果は次のとおり。

- toolなし推論中: 最初の`UserPromptSubmit`以降、終了hookなし。次の`UserPromptSubmit`で同一sessionの復帰を確認。
- auto modeのforeground tool実行中: `PreToolUse`以降、Post／Stop系hookなし。transcript上は
  `toolDenialKind = user-rejected`だが、hook payloadだけでは実行中断の終了を確定できない。
- permission待ち／`AskUserQuestion`待ち: `PermissionRequest`以降、Post／Stop系hookなし。
- MCP elicitation待ち: `ElicitationResult(action = cancel)`で明示的に確定できる。
- 中断runの観測時間内では`Notification: idle_prompt`は観測されなかった。

後続のClaude Code `2.1.226` lifecycle runでは、正常な`Stop`から約60秒後に
`Notification: idle_prompt`が届く例を確認した。したがって、`idle_prompt`はTurn終了主経路に採用せず、
遅延した補助signalとしてだけ扱う。MCP elicitation以外の
Esc中断は、次の`UserPromptSubmit`、`SessionEnd`、wrapper終了通知などのpositive signalで回復する。
positive signalが届かない場合は120秒後に詳細表示だけを`WORKING + UNSPECIFIED`へ縮退し、
Turn終了とはみなさない。

### 14.8 2026-08-08 session lifecycle実測

Claude Codeは検証途中で`2.1.224`から`2.1.226`へ更新された。`/clear`は`2.1.224`、
手動`/compact`、`/resume`、fork、plugin再読込は`2.1.226`で確認した。確定結果は次のとおり。

| 操作 | 観測eventとsession identity |
|---|---|
| `/clear` | 旧IDで`SessionEnd(reason = clear)`後、約0.39秒で新IDの`SessionStart(source = clear)`。同一launch内のsession置換 |
| 手動`/compact`実行条件不足 | `PreCompact(trigger = manual)`だけが届き、`PostCompact`と`SessionStart(source = compact)`は届かない |
| 手動`/compact`成功 | `PreCompact -> SessionStart(source = compact) -> PostCompact`。session IDは維持し、`SessionEnd`なし |
| `/resume` | `SessionStart(source = resume)`。終了前と同じsession IDを再利用 |
| fork | `SessionStart(source = fork)`。元sessionとは異なる新しいsession ID |
| `/exit` | `SessionEnd(reason = prompt_input_exit)` |

主な証跡runは`20260808-133936-19128`（clear）、`20260808-134753-21232`（手動compact）、
`20260808-135625-4428`（resume）、`20260808-135919-19408`（fork）、
`20260808-142233-27916`（plugin reload）である。各runはexit code 0で、Receiverの
`unauthorized`／`malformed`／`oversized`／通常・priority overflowは0だった。

`PreCompact`はcompact成功signalではない。成功確定には`SessionStart(source = compact)`または
`PostCompact`を使う。`PostCompact.compact_summary`には会話要約本文が含まれるため、製品側は
内容をHost Linkへ送らず、通常ログにも保存しない。

pluginの`hooks.json`変更は実行中sessionへ自動反映されず、`/reload-plugins`後に反映された。
一時pluginから`Stop`だけを削除した検証では、reload前はキャッシュ済み`Stop`が届き、reload後は
`UserPromptSubmit`と`SessionEnd`を維持したまま`Stop`だけが届かなくなった。plugin生成物は
UTF-8 BOMなしで書く。Windows PowerShell 5.1の`Set-Content -Encoding UTF8`が付けるBOMは
reload時のplugin load errorになる。

自動`/compact`は最低100k token規模の文脈生成が必要となるため、今回のGate Cでは`DEFERRED`とする。
OS再起動直後と実アンチウイルス負荷も従来どおり`DEFERRED`とする。主要eventとsession lifecycleの
実測は完了し、製品実装前レビューで詳細状態stale閾値120秒、wrapper補助signalの役割、
production timeout、manual permission区間の表示を確定した。利用者向けマニュアルには、
Escを直接検知できないこと、120秒後に詳細不明へ縮退すること、許可直後を推測で実行表示にしないことを記載した。

### 14.9 2026-08-08 製品transport境界の実装

製品用のobserver plugin生成、loopback HTTP Receiver、`keylink-claude-hook` Helper、
PowerShell wrapper終了通知をHost coreへ実装した。

- `claude_hooks.rs`: BOMなしplugin／hooks／observer／wrapper生成と1～3秒のhook timeout
- `claude_observer.rs`: token／1 MiB上限／必須event名の検査、通常128件＋priority 16件のbounded queue、204応答
- `claude_hook_helper.rs`と`keylink-claude-hook.rs`: stdoutへdecision bodyを出さない500 ms×最大2回の転送
- `claude_hook_event.rs`: raw bodyをログ表示しないtransport eventとwrapper終了event
- wrapperは起動時に`observer.json`をメモリへ読み、`finally`からbest-effort終了通知を送る

Host core 216件はPASSした。この段階ではClaudeEventAdapter、tombstone／冪等Reducer、Session Registry、
Tauri launcher、既存Codexとの表示選択、Host Link、Firmwareへ接続していない。

### 14.10 2026-08-08 Claude状態Reducerの実装

`claude_activity.rs`へClaude Code専用のAdapter／Normalizerとsession単位Reducerを実装した。
raw hook payloadはこの境界でevent種別、`tool_use_id`、待機request keyだけへ正規化し、回答本文や
compact要約などを状態へ保存しない。

- `PostToolUse`／`PostToolUseFailure`が先に届いた`tool_use_id`はtombstone化し、後着の`PreToolUse`を無視する
- `PermissionRequest`は`WAITING_APPROVAL`、`Elicitation`は`WAITING_INPUT`。許可・実行開始を推測しない
- 最終関連eventから120秒後は、終端へ遷移せず`WORKING + UNSPECIFIED`だけをemitする
- `SessionEnd`、wrapper終了、receiver側overflowなどの同期不能通知は冪等に処理する。同期不能でも進行中Turnを
  `WORKING + UNSPECIFIED`へ縮退するだけで終了とはみなさない
- 同じsession IDの`SessionStart`はwrapper終了前なら再開でき、wrapper終了後の遅延eventは無視する

Reducer単体test 10件を追加した。この段階ではSession Registry、Tauri launcher、既存Codexとの表示選択、
Host Link、Firmwareへは接続していない。

### 14.11 2026-08-08 Keylink Studio Host接続の実装

Session Registry、Tauri launcher、Settings画面、Host Link送信へ接続した。Registryは`session_id`単位で
upsertし、`SessionEnd`を個別retire、wrapper終了をlaunch全体retireとして処理する。Receiver overflowは
該当launchを`desynchronized`へ縮退する。Claude CodeはWindows Terminalから一時pluginで起動し、停止時に
Receiverとplugin directoryをcleanupする。

Firmwareを変更しないため、Host Linkのwire identityは暫定的に既存`Codex + CLI`を利用する。
Claudeのsession IDや本文はpacketへ載せない。この暫定表示はFirmware側の`CLAUDE_CODE` identity実装後に
置き換える。Host core 228件、Tauri 22件、UI production buildはPASSした。

### 14.12 2026-08-08 実機E2E確認とHelper同梱

Keylink Studioから起動したClaude CodeとScreenKeyについて、次を実機確認した。

- Helperを含むobserver pluginが読み込まれ、SettingsからClaude Codeを起動できる
- toolなし応答、tool実行、manual permission、`/clear`、`/exit`、連携停止後の再起動
- permission中のEsc後、120秒で`WORKING + UNSPECIFIED`へ縮退し、新しいpromptで通常表示へ回復する

manual permissionは許可直後を示すhookがない。そのため、許可待ちの黄色表示をPost系eventまたは
120秒stale化まで維持する。短いcommandではPostとStopが連続するため、許可後の青い実行表示が
見えず黄色から緑へ遷移しても正常である。

通常の`cargo tauri dev`では別crate binaryを自動でbuildしないため、`dev.ps1`は先に
`keylink-claude-hook`をbuildする。release buildは`build-release.ps1`がrelease HelperをTauri sidecarとして
同梱する。この段階の実機確認は暫定`Codex + CLI` identityで行い、後続の正式identity確認と区別した。

### 14.13 2026-08-08 正式Claude Code identity実装と実機E2E

WSL正本の`zmk-rawhid-app`へ`CLAUDE_CODE = 0x02`とcapability bit 12を追加し、ScreenKey側へ
96×96 RGB565ロゴとclient typeによるRenderer切り替えを実装した。HostはClaude Code状態を
`0x02`で送り、bit 12のない既存deviceを送信対象から除外する。

書き込み済みScreenKeyとKeylink Studio新ビルドで、Claude Codeロゴ、青い実行中、黄色の許可待ち、
黄色枠点滅の入力待ち、緑の完了、`/exit`、連携停止を確認した。通常の連続許可は2回とも黄色になった。
一方、失敗したcommandをClaude Codeが同一Turn内で自動修正・再試行した1例では、2回目の許可画面中に
青い実行表示となった。この失敗後再試行だけは既知の境界事例として残す。

自動検証はHost core 230件、Tauri 23件、UI production buildがPASSした。Firmware側は5本の
AI Client／Rendererテストとfresh buildがPASSし、UF2は510,976 B、SHA-256は
`402188bba5bd46a40377966e5ee115cd8c1043735f156b141bacfc378c1ba49a`である。
Firmwareは`zmk-rawhid-app`の`1a2ee78 feat: add Claude Code AI client type and capability bit`と、
`zmk-config-screenkeytest`の`7b24ec9 feat: show the Claude Code logo on ScreenKey`へ分けてcommitした。
どちらも`develop`で`origin/develop`より1コミット先行し、未pushである。

## 15. 実装順序

1. Gate C用probe、Claude Code `2.1.224`のevent意味論、`2.1.226`の主要session lifecycle実測、
   stale／終了／timeout／manual permissionの実装前レビューは完了。自動compactは`DEFERRED`。
2. observer plugin生成、Receiver、Helper、wrapper終了通知のHost core境界は実装済み。
3. ClaudeEventAdapter／Normalizer、tombstone、120秒の詳細状態stale化、session単位の冪等Reducerを実装済み。
4. Session Registry、Tauri launcher、Settings画面、Host Link送信を実装済み。
5. 暫定Codex identityでのClaude起動・状態遷移を実機確認済み。
6. WSL正本側へFirmwareのClaude Code識別、capability、ロゴを実装済み。
7. 正式Claude Code identityでend-to-end確認済み。

## 16. 実機確認結果

- Settings起動、observer／Session Registry、正式Claude Codeロゴ、manual permission、Esc stale化、
  stale後の次prompt回復、`/clear`、`/exit`、停止→再起動を確認済み。
- `AskUserQuestion`による入力待ちはClaude Codeロゴと黄色枠点滅になり、選択後に完了表示へ戻ることを確認済み。
- command失敗後の自動再試行で2回目の許可待ちだけ青くなった1例は既知の境界事例。通常の連続許可は正常。
- 2台以上の同時Claude session表示、receiver overflowの実機注入、OS再起動直後／実AV負荷は未確認であり、
  初期対応の完了条件には含めない。

## 17. 次の作業

3リポジトリのcommitをreviewし、明示指示後にpush／統合する。失敗後自動再試行時の2回目の
permission表示は、必要に応じてraw hookを再採取してClaude Code側のevent欠落かHost reducer側かを切り分ける。

## 18. 非対象

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
