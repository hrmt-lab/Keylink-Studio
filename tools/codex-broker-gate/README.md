# Codex Broker Gate

Gate AでObserver方式が不成立となったため、Codex CLIとCodex App Serverの間に置く
最小Brokerの透過転送性を検証するハーネスです。Gate BやKeylink Studio本体機能は実装しません。

```text
Codex CLI
  └─ ws://127.0.0.1:4501（Broker専用token）
       Keylink Studio Broker spike
         └─ ws://127.0.0.1:4500（App Server専用token）
              Codex App Server
```

BrokerはWebSocketの認証を区間ごとに終端し、JSON-RPC text messageを再解釈・再生成せずに
反対側へ送ります。状態抽出ログには時刻、方向、request／response／notificationの種別、
method、id、threadId、turnIdだけを記録します。prompt、response本文、tool引数、tokenは記録しません。

## 1. 外部通信不要のself-test

通常のWindows PowerShellで次を実行します。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-broker-gate\test-broker.ps1
```

次を合成WebSocket endpointで確認します。

- CLI→App Serverのrequestとresponse
- App Server→CLIのresponse、notification、応答必須server request
- server requestとCLI responseのID保持
- Broker用tokenとApp Server用tokenの分離
- 未認証接続と2本目のCLI接続の拒否
- CLI側・App Server側それぞれからの切断伝播
- メタデータへtokenやJSON-RPC本文が記録されないこと
- 偽のCodex CLI／App ServerによるPowerShellハーネス全体の起動、判定、run.json確定、後始末

## 2. 手動E2E

self-test成功後、コードを変更せず1回だけ次を実行します。このコマンドはApp ServerとBrokerを
起動し、固定promptを付けたCodex CLIを同じPowerShell内で開始するため、1 Turn分のモデル利用が
発生します。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\codex-broker-gate\run-broker-gate.ps1
```

CLIにapprovalが表示されたら、承認または拒否のどちらかを選びます。Turnが終了したら`/exit`で
CLIを閉じてください。ハーネスはApp ServerとBrokerを停止し、結果を
`target/codex-broker-gate/runs/<run-id>/run.json`へ確定します。

成功条件は次のとおりです。

- CLIがBrokerへ接続し、BrokerがApp Serverへ接続する
- CLI requestとApp Server notificationが両方向へ配送される
- `item/commandExecution/requestApproval`または`item/fileChange/requestApproval`がCLIへ配送される
- 同じJSON-RPC IDのCLI responseがApp Serverへ戻る
- CLI exit codeが0である

`target/codex-broker-gate`はGit管理外です。raw stdout／stderr、token、JSON-RPC本文はコミットしません。
tokenは起動ごとに別々に生成したユーザー専用一時ファイルに置き、終了処理で削除します。

## 3. 実装上の制限

- `ws://127.0.0.1`以外のlisten／upstreamを拒否します。
- 同時接続はCLI 1本だけです。
- BrokerはJSON-RPC requestへ代理応答しません。
- WebSocket ping／pongは各区間で終端します。
- Broker停止・異常終了は接続中CLIを切断します。暗黙の再接続は行いません。
- rawログは診断専用であり、結果文書へ転記しません。
