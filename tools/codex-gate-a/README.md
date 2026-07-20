# Codex App Server Gate A 手動試験

このディレクトリは、Keylink Studio本体へ着手する前に、Codex App Serverのクロスクライアント可視性を確認するための手動試験ハーネスです。
試験は通常のWindows PowerShellから実行してください。Codex内蔵shellからは実行しません。

## 安全上の前提

- 対象はWindows版Codex CLI `0.144.6`です。
- App Serverは`127.0.0.1:4500`だけでlistenします。
- capability tokenは実行ごとに生成し、子プロセスへ環境変数で渡します。
- tokenファイルはユーザー専用ACLを設定したOS一時ディレクトリへ置き、終了時に削除します。
- ObserverはSchemaで確認した`clientInfo`と`capabilities.experimentalApi`だけを送ります。
- Observerはserver requestへ応答しません。応答必須requestを受信するとexit code `42`で終了し、ハーネスもシナリオを停止します。
- `config.toml`、認証情報、`CODEX_HOME`は変更しません。
- 生ログと生成Schemaはgit管理外の`target/gate-a/`だけに保存します。commitしないでください。
- preflightで`target/gate-a/`がGitのignore対象であり、配下に追跡済みファイルがないことを確認します。

## ハーネスが行う確認

各実行の最初に、次のhelpと正式なオプション名を確認します。

```powershell
codex --version
codex --help
codex resume --help
codex app-server --help
codex app-server generate-json-schema --help
```

次のオプションが見つからない場合は、代替を推測せず停止します。

- `--remote`
- `--remote-auth-token-env`
- `--listen`
- `--ws-auth`
- `--ws-token-file`
- `--out`
- `--experimental`

外部通信やApp Server起動を行わず、helpとSchema検証だけを再確認する場合は次を使用できます。これはGate A本試験には数えません。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 -Scenario ThreadTurn -PrepareOnly
```

続いてexperimental Schemaを生成し、次をSchemaから確認します。

- `InitializeParams.clientInfo`
- `InitializeParams.capabilities`
- `ClientInfo.name`、`title`、`version`
- `InitializeCapabilities.experimentalApi`
- `ThreadResumeParams.threadId`
- Gate Aで確認するThread／Turn通知とserver request method

Resume／ApprovalではThread IDの自動選択を禁止しています。両方へ同じ`-ThreadId`と`-ValidationPairId`を明示し、Observerが先に`thread/resume`を完了した後、CLIも`codex resume <threadId>`で同じThreadを開きます。Approvalは、同じ検証ペアのResumeが成功済みであり、Thread IDとハーネスSHA-256が一致する場合だけ開始します。

各runの`run.json`には、使用したThread ID、検証ペア、Observerのresume結果、CLIのresume開始、CLI終了コード、応答必須request method、Gate A関連スクリプトごとのSHA-256と統合fingerprintを記録します。

判定では、CLI起動前の古いイベントを除外し、同一Thread ID・同一Turn IDの`turn/started`と`turn/completed`を相関します。Approvalも、CLI起動後に観測した`turn/started`と同じThread／Turnに属するrequestだけを採用します。CLI resumeは起動markerだけでは成功扱いせず、対応する`turn/started`の受信を必須とします。例外時も`run.json`を`failed`で原子的に確定し、cleanup結果を記録します。

## 最終版ハーネスでの再検証

最初に外部通信不要のself-testを実行します。Observerのexit `42`、異常なmarker／exit codeの拒否、イベント相関、Observer response検出、原子的metadata更新、失敗run確定、テスト用プロセスツリーcleanupを検証します。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\test-gate-a.ps1
```

`Gate A self-test passed.`と全項目の`passed`が表示された場合だけ、以下の2シナリオを順番に実行します。途中でコードを変更しないでください。Thread IDと検証ペアIDは両コマンドで同一にします。

### 1. Resume

固定promptは`codex resume ... [PROMPT]`の正式な引数として自動投入されます。CLIへ手入力しません。応答完了後に`/exit`します。

```text
Respond with exactly GATE_A_RESUME_OK. Do not use tools.
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 -Scenario Resume -ThreadId 019f7d67-f922-71b2-9c2a-5e855d56be80 -ValidationPairId gate-a-final7-20260720
```

終了表示の`Run status`が`passed`であることを確認します。失敗した場合はApprovalへ進みません。

### 2. 同一ThreadのApproval

Resume成功後、次を実行します。固定promptは自動投入されるため、CLIへ手入力しません。approval requestがObserverへ届くと、Observerは応答せずexit code `42`で安全停止し、ハーネスがCLIを終了します。ユーザーがapprovalへ回答する必要はありません。

```text
Create target/gate-a/manual-approval-test.tmp containing GATE_A. Request approval before writing it. Do nothing else.
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 -Scenario Approval -ThreadId 019f7d67-f922-71b2-9c2a-5e855d56be80 -ValidationPairId gate-a-final7-20260720
```

終了表示の`Run status`が`passed`であることを確認します。

Resume／Approvalでは、固定prompt投入後60秒以内に`turn/started`が観測されない場合、または180秒以内にシナリオが完了しない場合、ハーネスが自動的に失敗として停止します。

## 実行順序

PowerShellをリポジトリルートで開き、以下を上から順に1回ずつ実行します。各コマンドはApp Server、Observer、対話用Codex CLIを起動します。CLI画面に表示されるシナリオ指示に従い、完了後は`/exit`でCLIを終了してください。

### 1. Thread／Turn配信

CLIへ次を入力します。

```text
Respond with exactly GATE_A_THREAD_TURN_OK. Do not use tools.
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 -Scenario ThreadTurn
```

### 2. 後から接続したObserver

CLIで最初のThreadとTurnを完了します。CLIは閉じず、起動元PowerShellへ戻ってEnterを押します。
`Observer initialized`が表示されたらCLIへ戻り、同じプロンプトでもう1回Turnを完了してから`/exit`します。
これにより、後から接続したObserverが接続後のTurnを観測できるか確認します。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 -Scenario LateObserver
```

### 3. `thread/resume`後の配信

この旧手順は最終再検証には使用しません。上記「最終版ハーネスでの再検証」の明示的なThread ID付きコマンドを使用してください。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 -Scenario Resume -ThreadId <THREAD_ID> -ValidationPairId <PAIR_ID>
```

### 4. Approval要求と解消

CLIには、安全な一時ファイルの作成を依頼してください。approvalを5秒間未解消にした後、拒否します。

```text
Create target/gate-a/manual-approval-test.tmp containing GATE_A. Request approval before writing it. Do nothing else.
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 -Scenario Approval -ThreadId <THREAD_ID> -ValidationPairId <PAIR_ID>
```

### 5. User input要求と解消

CLIをPlan modeへ切り替え、`request_user_input`で選択質問を1回表示させ、5秒後に回答します。

```text
/plan
Ask me one multiple-choice question using request_user_input. Do not use any other tools.
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 -Scenario UserInput
```

### 6. MCP elicitation要求と解消

このシナリオだけ、一時的なCLI `-c` overrideで`mcp-elicitation-server.mjs`を登録します。恒久設定は変更しません。CLIでは`gate_a_request_elicitation`ツールを1回呼び、5秒後に回答します。

```text
Call gate_a_request_elicitation exactly once. Do not call any other tool.
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 -Scenario McpElicitation
```

### 7. Pending approval中のCLI切断

approvalを表示したまま応答せず、CLIウィンドウを閉じます。
CLIへ入力する内容はシナリオ4と同じです。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 -Scenario PendingApprovalDisconnect
```

### 8. Pending input中のCLI切断

user inputを表示したまま応答せず、CLIウィンドウを閉じます。
CLIへ入力する内容はシナリオ5と同じです。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 -Scenario PendingInputDisconnect
```

## ログの確認

各シナリオの出力先は次の形式です。

```text
target/gate-a/runs/<UTC timestamp>-<Scenario>/
```

`observer.jsonl`には以下のmetadataだけが保存されます。

- timestamp
- direction
- kind
- method
- request/response ID
- threadId／turnId
- status
- 応答必須か
- errorの有無

Thread本文、プロンプト、応答本文、token、認証情報は保存しません。App Serverのstdout/stderrは生ログなので、必要なmethod、field、status、配送先だけを`docs/codex-gate-a-results.md`へ転記し、生ログ自体はcommitしないでください。

## 判定

次をすべて満たす場合だけObserver方式成立です。

- CLIが開始したThread／TurnをObserverが安定して観測できる
- 必要なresume／購読手順が再現可能
- approval、user input、MCP elicitationの発生または解消を観測できる
- Observerへ応答必須requestが配送されない
- CLIの接続／切断を観測できる
- Observer自身を識別して除外できる

一つでも満たせない、または再現できない場合はBroker必須と判定します。

Gate Aの判定後は結果を記録して停止し、Gate B、Keylink Studio本体、仕様書§8～§10へ進まないでください。
