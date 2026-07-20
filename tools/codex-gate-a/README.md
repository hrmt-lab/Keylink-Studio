# Codex App Server Gate A 再検証ハーネス

Codex CLI `0.144.6`で、Observer方式が成立しない決定的条件を再現するための最終版です。
通常のWindows PowerShellから実行してください。`codex login`、ユーザー設定、`CODEX_HOME`は変更しません。

## 判定対象

最終判定に必要な次の2シナリオだけを残しています。

1. `Resume`: Observerが明示的に`thread/resume`した後、同じThreadをCLIでresumeし、
   `turn/started`と`turn/completed`を観測できることを確認する。
2. `Approval`: 同じThreadと同一ハーネスで、応答必須
   `item/commandExecution/requestApproval`がObserverへ配送されることを確認する。

Approval requestがObserverへ配送された場合、Observerは応答しません。ハーネスはCLIを停止し、
結果を`broker_required`として確定します。Observerのexit intent `42`とWindowsが観測したexit codeは
安全停止診断として記録しますが、Broker必須判定の条件にはしません。

## 前提

- `codex-cli 0.144.6`
- Windows PowerShell
- `node`は不要
- ポート`4500`が空いていること
- 使用する既存Thread IDが分かっていること
- `target/gate-a/`がGit管理外であること

preflightは次を確認します。

- `codex --version`
- `codex --help`
- `codex resume --help`
- `codex app-server --help`
- `codex app-server generate-json-schema --help`
- 正式なremote／listen／認証option
- 生成Schema上の`clientInfo`、`capabilities.experimentalApi`、`threadId`
- 必要なmethod
- `target/gate-a/`のGit除外

## 実行手順

`<THREAD_ID>`と`<PAIR_ID>`を決め、3コマンドすべてで同じ値を使います。

### 1. preflightのみ

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 `
  -Scenario Resume `
  -ThreadId <THREAD_ID> `
  -ValidationPairId <PAIR_ID> `
  -PrepareOnly
```

### 2. Resume

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 `
  -Scenario Resume `
  -ThreadId <THREAD_ID> `
  -ValidationPairId <PAIR_ID>
```

固定promptが自動投入されます。Turn完了後に`/exit`してください。
`Run status: passed`を確認してからApprovalへ進みます。

### 3. Approval

Resume後にコードを変更せず実行します。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-gate-a\run-gate-a.ps1 `
  -Scenario Approval `
  -ThreadId <THREAD_ID> `
  -ValidationPairId <PAIR_ID>
```

approvalへ回答しないでください。Observerへの配送後、ハーネスが自動停止します。
再現成功時の出力は次のとおりです。

```text
Run status: broker_required
Decision: observer_ineligible_response_required_request_delivered
```

## 保存内容

実行結果は`target/gate-a/runs/<run-id>-<scenario>/`へ保存します。

- `run.json`: version、Schema hash、ハーネスhash、method、ID、Thread／Turn相関、status、cleanup
- `observer.jsonl`: method、ID、Thread ID、Turn ID、status、方向だけのsanitized log
- stdout／stderr: ローカル診断用の生ログ

token、認証情報、Thread本文、prompt本文、応答本文、tool引数は結果メタデータへ記録しません。
`target/`全体はリポジトリの`.gitignore`で除外されています。

## 削除したシナリオ

次は最終判定前に停止条件へ到達したため未確認であり、再検証ハーネスから削除しました。

- `ThreadTurn`
- `LateObserver`
- `UserInput`
- `McpElicitation`
- `PendingApprovalDisconnect`
- `PendingInputDisconnect`

これらを再開しても、approval server requestがObserverへ配送されたという否定条件は変わりません。
必要になった場合は新しい検証目的と停止条件を定義し、別ハーネスとして追加します。
