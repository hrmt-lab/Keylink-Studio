# Codex複数セッション対応仕様

## 1. 文書の位置付け

- 状態: Host実装・自動テスト完了。実機受け入れは3秒以内resume以外を確認済み
- 対象: Keylink StudioのCodex Broker、Codex activity、共通AIセッション選択
- 後続: 複数ScreenKeyへ異なるセッションを割り当てる機能

本仕様は、Keylink Studioが複数のCodexセッションを同時に表示候補として保持し、既存の
`cycle_ai_session`でClaude Codeセッションと区別せず循環できるようにするための仕様である。
複数ScreenKey対応より先に本仕様を実装し、表示候補の管理を完成させる。

## 2. 目的

次の利用形態を成立させる。

1. 複数のCodex CLIをKeylink Studioへ同時接続する。
2. CLIごとに異なるCodex threadを実行する。
3. 各threadの状態を非選択中も独立して追跡する。
4. CodexとClaude Codeの全有効セッションを1つの候補列として扱う。
5. 1つのScreenKeyで候補を循環し、選択時点の最新状態を表示する。
6. 後続の複数ScreenKey対応が、同じ候補集合をそのまま利用できるようにする。

## 3. 現行実装の制約

現行実装は次の単一Codexセッション前提を持つ。

- Brokerは同時に1つのCodex CLI接続だけを許可し、追加接続を`409 Conflict`で拒否する。
- `CodexEventAdapter`は接続を1件だけ保持する。
- `AiClientStateReducer`は`tracked_thread_id`を1件だけ保持する。
- 新しい`thread/start`、`thread/resume`、fork先への切替は以前のthreadを置き換える。
- `CodexActivityRuntime`が公開するsnapshotは1件だけである。
- `AiDisplayTarget::Codex`にthread識別子がなく、複数のCodex候補を区別できない。

## 4. 用語と識別子

### 4.1 接続

- `connection_id`: BrokerがWebSocket接続確立ごとに生成する一時識別子。
- 再接続後の`connection_id`は以前と同じであることを保証しない。
- JSON-RPC request IDの照合とメッセージ配送は`connection_id`単位で分離する。

### 4.2 Codexセッション

- Codexセッションの正規識別子はApp Serverが返す`thread_id`とする。
- 表示候補キーは`Codex(thread_id)`とする。
- `connection_id`は表示候補キーに含めない。
- 同じ`thread_id`を別接続がresumeしても、表示候補は重複させず1件へ統合する。
- `thread_id`をFirmwareへ送らない。Firmwareは従来どおり状態とclient typeだけを受信する。

### 4.3 所有接続

各Codexセッションは、状態更新の配送元となる`owner_connection_id`を最大1件持つ。

- `thread/start`または`thread/resume`の成功responseを受けた接続を所有接続とする。
- `thread/fork`の成功responseで作られたthreadも、その接続を所有接続とする。
- 所有接続がない状態で未知threadの`turn/started`を受けた場合、その接続を所有接続として候補を作る。
  App Server通知は複数接続へbroadcastされ得るため、Brokerは同じ接続で先に成功した
  `turn/start` responseと同じ`thread_id`を確認できた場合だけ、この規則を適用する。
  相関のないbroadcast通知だけでは所有権を作らない。
- 同じthreadを別接続が明示的にresumeした場合、所有権を新しい接続へ移す。
- 所有権移動後、旧接続から届く同じthreadのactivity eventは表示状態へ反映しない。

この規則により、同じthreadを複数CLIから同時操作した場合のapproval、input、item eventの
二重計上を防ぐ。同一threadの同時操作自体は保証対象外とし、所有権移動をログへ記録する。

## 5. 事前互換性ゲート

実装開始前に、現在サポート対象のCodex CLI／App Serverで次を確認する。

1. 1つのApp Serverへ2つのWebSocket clientを同時接続できる。
2. 各clientが`initialize`から異なる`thread/start`を成功できる。
3. 2つのthreadでTurnを同時実行し、両方のeventを観測できる。
4. 同じJSON-RPC request IDを各接続が使用しても応答が混線しない。
5. 一方の接続終了が他方の接続とTurnを終了させない。
6. approvalと`requestUserInput`が要求元接続で解決でき、他方の状態を変えない。

このゲートを通過した場合は、1つのApp Serverを共有し、Codex CLI接続ごとに独立した
Broker上流WebSocketを作る。App Serverが複数clientを安全に扱えないことが判明した場合は、
本仕様の実装を止め、App Serverを接続ごとに分離する別設計へ戻る。

Codex CLI versionとexperimental schema hashの既存preflightは維持する。複数接続対応を理由に
互換性検査を緩和しない。

## 6. Broker仕様

### 6.1 同時接続

- 認証済みCodex CLIを最大8接続まで同時に受け付ける。
- 接続ごとに独立した上流WebSocket、転送task、`connection_id`を持つ。
- 9件目以降は`409 Conflict`で拒否し、上限到達をログへ記録する。
- 1接続の転送失敗はその接続だけを終了し、他の接続とApp Server processを停止しない。
- Broker停止またはApp Server終了時は全接続を終了する。

### 6.2 状態公開

`CodexBrokerStatus`は次を公開する。

- `connected_client_count`: 現在接続中のCLI数。
- `client_connected`: 後方互換用。`connected_client_count > 0`と同値。

集約phaseは次の規則とする。

- 1件以上接続中: `connected`
- 0件で、切断後3秒の再接続猶予中: `reconnecting`
- 0件で、再接続猶予なし: `waiting_for_client`
- 起動、停止、全体errorは既存規則を維持する。

一部接続だけが再接続猶予中でも、別接続が残っていれば集約phaseは`connected`のままとする。

### 6.3 ランチャー

- Broker／App Server起動中でも「Codexを開く」を追加実行できる。
- 起動ごとに新しいTerminal tabとCodex CLI processを作る。
- Broker token、port、対応runtimeは同じ連携インスタンスの値を利用する。
- UIは接続数を表示し、単一の`client_connected`だけで追加起動を禁止しない。
- 「Codex連携を停止」は全Codex CLI接続、Broker、App Serverをまとめて停止する。
- 個別CLIをKeylink Studioから終了する機能は本仕様の対象外とする。

## 7. Codex Session Registry

### 7.1 構造

`CodexActivityRuntime`は単一Reducerの代わりに、次のRegistryを保持する。

- 接続ごとの`CodexEventAdapter`
- `thread_id`ごとのCodex Session Reducer
- threadごとの`owner_connection_id`
- 接続ごとの観測済みthread集合
- threadの初回登録順
- threadごとの再接続期限

各Reducerは次をthreadごとに独立して保持する。

- 最新`AiClientStateSnapshot`
- 現在のturn ID
- approval／input request
- active itemとwork phase
- work phase debounce
- `COMPLETED`の30秒期限
- revision

### 7.2 登録

次をCodexセッション登録の確定境界とする。

- `thread/start`成功responseの`result.thread.id`
- `thread/resume`成功responseで、要求IDと結果IDが一致
- `thread/fork`成功responseの`result.thread.id`
- 所有者のない未知threadに対する`turn/started`

`thread/started` notificationだけでは候補を確定しない。request／responseの相関を優先する
既存の安全境界を維持する。

### 7.3 thread切替

同じ接続が別threadをstart、resume、forkしても、以前のthreadを直ちに終了しない。
以前のthreadは同じ接続が観測した候補として残り、最後のsnapshotを保持する。
そのため、1つのCLIで複数threadを行き来した場合もセッション切替候補にできる。

非表示threadでも次を継続する。

- eventの取り込み
- `COMPLETED`から30秒後の`AVAILABLE`遷移
- approval／input解決
- work phase debounce
- 再接続期限の判定

### 7.4 切断と再接続

- CLI起因の切断時、その接続が所有するthreadを3秒間保持する。
- 猶予中に同じ`thread_id`が別接続からresumeされた場合、snapshotとrevisionを維持して所有権を移す。
- 3秒以内に所有接続が戻らなければ、その接続だけが所有していたthreadを候補から削除する。
- 他の接続が所有するthreadは削除しない。
- Broker停止、App Server停止、連携全体errorでは全Codexセッションを終了する。

### 7.5 上限

- Registryが保持するCodexセッションは最大32件とする。
- 上限到達時は、選択中ではなく、working／approval待ち／input待ちでもない最古の候補を退役させる。
- 安全に退役できる候補がなければ、新しい候補を追加せず警告を記録する。
- 時間経過だけを理由に有効な候補を削除しない。

## 8. 共通表示候補

`AiDisplayTarget`を次の識別形へ変更する。

```text
Codex { thread_id }
Claude { launch_id, session_id }
```

Codex／Claude Codeを同じ候補列へ登録し、クライアント種別ではグループ化しない。
並び順は、その候補がKeylink Studioへ最初に有効登録された順とする。

- 現在の選択が有効なら、候補追加時も維持する。
- 選択中候補が終了した場合は、終了前の位置に続く候補を選ぶ。
- 候補が再接続猶予へ入っただけでは選択を変えない。
- 猶予満了で候補が削除された時点で次候補へ移る。
- `cycle_ai_session`はクライアント種別に関係なく次候補へ進む。
- 選択変更時は選択後snapshotをfull stateとして即時送信する。
- 非選択候補のeventをHost Linkへ送らない。

候補表示用ラベルは次とする。

- Codex: `Codex <thread_idの先頭8文字>`
- Claude Code: 既存のsession label

thread ID全体は診断ログで確認できるようにするが、ScreenKeyへは送らない。

## 9. Host Link／Firmware境界

本仕様の複数Codex対応ではHost Link packetとFirmwareを変更しない。

- `AI_CLIENT_STATE`のpayloadは現行のままとする。
- `client_type = CODEX`も変更しない。
- session ID、thread ID、connection ID、候補数、選択順をFirmwareへ送らない。
- 1つのScreenKeyには、共通セレクタで選択中の1候補だけを送る。

複数ScreenKeyへ異なる候補を同時表示するHost側slot管理は
[複数ScreenKey表示設計](ai-display-slot-multiscreen-host-design.md)で実装した。Keylink Studioの
この変更自体はFirmwareを変更していないが、別workspaceのbit 13対応Firmwareと組み合わせた2画面の
個別表示は2026-08-09に実機確認した。

## 10. ログ

通常ログへ次を記録する。token、会話本文、入力内容は記録しない。

- CLI接続／切断: connection IDの短縮表記、現在接続数、切断元
- thread登録／退役: thread IDの短縮表記、理由
- thread所有権移動: 旧／新connection IDの短縮表記
- 表示選択: client type、セッション識別子の短縮表記
- 接続上限／Registry上限到達
- 未所有接続からのevent破棄

## 11. 異常系

- 1接続の不正認証はその接続だけを拒否する。
- 1接続の不正JSON、binary、未知methodは既存規則どおり無視または診断し、他接続へ影響させない。
- JSON-RPC request／response相関は接続をまたいで共有しない。
- 異なるthreadの同じturn ID、item ID、request IDを衝突させない。
- 所有していない接続からのactivity eventで表示状態を上書きしない。
- 選択中Codexセッションが消えても、Claude Codeを含む次候補へ安全に移る。
- すべての候補が消えた場合はセッションなし状態を送る。

## 12. 自動テスト受け入れ条件

### 12.1 Broker

- 2接続を同時に認証して転送できる。
- 接続ごとに異なる`connection_id`が発行される。
- 同じJSON-RPC IDを2接続で使ってもmessage metadataが接続別に配送される。
- 同一接続でも、CLI起点とApp Server起点で同じJSON-RPC IDが再利用されても相関を混線させない。
- 一方の切断後も他方が接続中で、phaseが`connected`を維持する。
- 8接続までは受理し、9接続目を拒否する。
- Broker停止で全接続taskが終了する。

### 12.2 Session Registry

- 2接続の異なるthreadを同時に候補として保持する。
- 1接続でthread Aからthread Bへ移ってもAを候補へ残す。
- 同じthreadを別接続がresumeした場合、候補を重複させず所有権を移す。
- threadごとのturn、approval、input、item、work phase、revisionが混線しない。
- 非所有接続からの同一thread eventを破棄する。
- 1接続の切断で他接続のthreadを終了しない。
- 同じthreadの3秒以内の再接続でsnapshotとrevisionを維持する。
- 猶予満了で孤立threadだけを候補から削除する。
- 非選択中も`COMPLETED`が30秒後に`AVAILABLE`へ戻る。

### 12.3 共通セレクタ

- 複数Codexだけを循環できる。
- 複数Codexと複数Claude Codeを登録順に循環できる。
- 非選択セッションのeventが表示を奪わない。
- 選択中セッション終了時に次候補へ移る。
- 選択変更時にclient typeを含むfull stateを即時送信する。
- 候補0件／1件でも安全に動作する。

### 12.4 回帰

- Codex CLI 1接続／1threadの既存状態遷移を維持する。
- Claude Code複数セッションの既存Registryを変更しない。
- `cycle_ai_session`の既存HOST_ACTION設定を維持する。
- Host core、Tauri、UIの既存テストとbuildが成功する。
- Host Link packet byte列とFirmwareに差分がない。

## 13. 実機受け入れ条件

1. Keylink StudioからCodex CLIを2つ起動できる。
2. それぞれ異なるthreadでTurnを実行できる。
3. 一方がworking、もう一方がcompletedなど、異なる状態を独立保持できる。
4. ScreenKeyのキー押下でCodex A、Codex B、Claude Codeの順に切り替えられる。
5. 非表示セッションで状態が変わっても、現在表示中のセッションは変わらない。
6. 非表示セッションへ戻したとき、最新状態が表示される。
7. 一方のCodex CLIを閉じても、他方の表示と動作を維持する。
8. 同じthreadを3秒以内に再接続した場合、不要なセッション終了表示を挟まない。
9. approvalとinput待ちが別threadへ誤表示されない。
10. 既存のClaude Code表示と単一Codex利用に回帰がない。

## 14. 実装順序

1. 事前互換性ゲートを実施する。
2. Brokerを複数接続・接続数管理へ変更する。
3. `CodexEventAdapter`を接続ごとのmapへ変更する。
4. Codex Session RegistryとthreadごとのReducerを実装する。
5. `CodexActivityRuntime`のsnapshot／change APIを複数session向けに変更する。
6. `AiDisplayTarget::Codex`へ`thread_id`を追加し、共通候補を登録順へ統一する。
7. ランチャーとUIを追加起動・接続数表示へ対応させる。
8. 自動テストを完了する。
9. 1つのScreenKeyで複数Codex／Claude Codeの切替を実機確認する。
10. 確認完了後、複数ScreenKey個別割り当て仕様へ進む。

## 15. 非対象

- 複数ScreenKeyへの個別割り当て
- Firmwareでのsession／thread管理
- ScreenKeyからapproval／inputへ回答する機能
- 会話本文、prompt、tool引数の保存または表示
- Codex CLI processの個別終了UI
- 同じthreadを複数CLIから同時操作した場合の完全な同期
- 複数App Server process方式。事前互換性ゲート不合格時に別途再設計する

## 16. 2026-08-08 実装結果

事前互換性ゲートはCodex CLI `0.147.0`と既存の単一App Server構成で実施し、合格した。
2つのCLI相当WebSocket clientを同時接続し、別threadの同時Turn、接続ごとに同じ
JSON-RPC IDを使ったrequest／response分離、一方の切断後の他方Turn完了、
`item/commandExecution/requestApproval`、`item/tool/requestUserInput`を確認した。
確認結果は次のとおりである。

- thread A: `019fe1c1-b55f-7db1-92c3-b792c18737a6`
- thread B: `019fe1c1-b560-7222-a051-d535c65f7f34`
- 両接続で使用したJSON-RPC ID: `1`, `2`, `3`
- client Aのinput request: 1件
- client Bのapproval request: 1件
- client A切断後のclient B Turn完了: 成功

Host実装は§6～§12を反映した。Brokerは最大8接続、Codex Session Registryは最大32件とし、
`thread_id`を正規識別子、`connection_id`を所有接続識別子として扱う。同一threadのresumeでは
候補を増やさず所有権を移し、旧所有接続からのeventを破棄する。CLI起因切断は3秒の猶予を設け、
その間の同一thread再接続ではsnapshotとrevisionを維持する。

§12のBroker、Session Registry、共通セレクタ受け入れ条件はRust自動テストへ追加した。
この複数Codex変更自体はHost Link packetとFirmwareを変更していない。後続のKeylink Studio側
複数ScreenKey対応では`display_slot`を末尾に追加する8-byte payloadをbit 13でgateする。Firmwareは未変更である。

§13の実機受け入れでは、Keylink Studioから2つのCodex CLIを起動した状態での実ScreenKey切替、
非選択eventの非奪取、片側CLI終了、approval／inputのthread間分離を確認済みである。3秒以内resume時の
表示とrevision維持は、レアケースとして未実施のまま残す。

### 2026-08-09 レビュー指摘の是正

複数threadを同じ接続が扱う場合の相関を接続全体でclearしていた箇所を、thread／turn単位の
削除へ置き換えた。これにより、thread Aのapproval／input待ち中にthread BのTurnが始まっても、
thread Aの応答相関は維持される。CLIからApp Serverへのresponseはserver request相関だけを
解決し、App ServerからCLIへのresponseだけが`thread/start`、`thread/resume`、`thread/fork`、
`turn/start`の送信相関を解決する。方向ごとのID空間を分けるため、双方が同じJSON-RPC IDを
用いても片方のrequestを誤消費しない。

接続上限は上流App Server WebSocketを張る前に予約し、8件の接続済みCLIとhandshake中の予約を
合わせて制限する。`connected_client_count`はupgrade完了後の接続だけを数える。接続局所の
handshake／転送エラーは全Session Registryを終了せず、Broker lifecycle全体の停止／errorだけが
全Codex sessionを終了する。`stopping`と`error`のBroker phaseおよび`last_error`は接続終了で
上書きしない。

CodexとClaude Codeの候補には共通の単調増加登録順を付け、同じmonitor tickで初めて登録された
場合もクライアント種別ではなく登録順で循環する。選択中の状態changeが、非選択threadの大量change
によって待ち行列から先に捨てられないよう、満杯時は非選択threadのchangeを優先して退避する。

追加した自動テストは、同一接続の2thread approval相関、方向間のJSON-RPC ID衝突、相関済み未知
threadの登録、接続局所error、実WebSocketの8接続受理と9件目`409 Conflict`、connection ID別の
message metadata、3秒猶予中のtick、保護中32sessionでの追加拒否、同tickのCodex／Claude登録順を
対象にする。実機受け入れは3秒以内resumeを除き確認済みである。

同日の再レビューで判明した候補再有効化の順序問題も是正した。候補列は毎回、全クライアント型を
通した不変の`registration_order`だけで再構築する。したがって、一時的に候補外となったClaude Code
sessionが同じ`(launch_id, session_id)`で再有効化しても、後から登録された候補の末尾には移動せず、
初回有効登録位置へ戻る。App Serverのerror responseは対応する`thread/start`、`thread/resume`、
`thread/fork`の送信相関を必ず解放し、接続が継続しても失敗済みrequestを残さない。両ケースの回帰
テストを追加した。
