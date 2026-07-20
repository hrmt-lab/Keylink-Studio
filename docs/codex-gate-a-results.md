# Codex App Server Gate A 結果

- 状態: 完了
- 実施日: 2026-07-20
- 最終判定: **Observer方式不成立／Broker必須**
- 対象Codex CLI: `0.144.6`
- 実行環境: 通常のWindows PowerShell
- Schema SHA-256: `85EA836927D6CFDD3C68A9BDA17DBA48D2573BBC282AB2D5775A5005E40BC9C3`
- 最終試行ハーネスSHA-256: `B925CB507B4E1A77AF5F8E293A4E01CF494F6035433061E5E26A26E361213EDC`

## Preflight

| 項目 | status | 確認結果 |
|---|---|---|
| `codex --version` | 成功 | `codex-cli 0.144.6` |
| `codex --help` | 成功 | CLI接続オプションを確認 |
| `codex resume --help` | 成功 | SESSION ID指定とremote接続オプションを確認 |
| `codex app-server --help` | 成功 | listen・WebSocket認証オプションを確認 |
| `generate-json-schema --help` | 成功 | `--out`・`--experimental`を確認 |
| 最小Turn | 成功 | 正常終了、tool callなし、認証・設定・`CODEX_HOME`変更なし |

## 使用した正式なCLIオプション

| 用途 | オプション | status |
|---|---|---|
| CLI接続 | `--remote` | 使用済み |
| CLI token環境変数 | `--remote-auth-token-env` | 使用済み |
| 既存Thread再開 | `codex resume <SESSION_ID>` | 使用済み |
| App Server listen | `--listen` | 使用済み |
| WebSocket認証方式 | `--ws-auth capability-token` | 使用済み |
| tokenファイル | `--ws-token-file` | 使用済み |
| Schema出力 | `--out` | 使用済み |
| experimental Schema | `--experimental` | 使用済み |

Observerの`initialize`では、生成Schemaに存在する次のfieldだけを使用した。

- `params.clientInfo.name`
- `params.clientInfo.title`
- `params.clientInfo.version`
- `params.capabilities.experimentalApi`

## 配送結果

| シナリオ | Observerへの配送 | status |
|---|---|---|
| Thread／Turn・resumeなし | `thread/started`、`thread/status/changed`を受信。`turn/started`、`turn/completed`は未受信 | Observer単独では不足 |
| 後から接続 | 接続後のTurnで`thread/status/changed`を受信。Turn通知は未受信 | Observer単独では不足 |
| 明示的`thread/resume` | resume response成功後、`turn/started`、`turn/completed`を受信 | 購読手順として再現成功 |
| Approval | `item/commandExecution/requestApproval`が応答必須JSON-RPC requestとしてObserverへ配送 | **Observer不成立** |
| User input | 停止条件到達のため未実施 | 未確認 |
| MCP elicitation | 停止条件到達のため未実施 | 未確認 |
| Pending request中のCLI切断 | 停止条件到達のため未実施 | 未確認 |
| CLI接続／切断通知 | 停止条件到達のため未確定 | 未確認 |
| Observer自己識別・除外 | `clientInfo`送信は確認。別接続からの識別・除外は未確定 | 未確認 |

Approvalシナリオでは、Observerが先に対象Threadへ`thread/resume`し、resume responseを受信した後でCLIも同じThreadを再開した。その状態でCLIのTurnが要求したapprovalがObserverにも直接配送された。Observerは要求へ応答していない。

## 最終判定

品質レビューにより、旧ハーネスには失敗runが`prepared`のまま残る問題、Approval requestを現在のThread／Turnへ相関しない問題、CLI resume起動markerを実イベントと区別しない問題、Observer安全停止経路とprocess-tree cleanupのself-test不足が確認された。これらの修正と外部通信不要self-test完了後に、新しい検証ペアで再実施する。修正前の全runは最終判定の根拠に使用しない。

検証ペア`gate-a-final4-20260720`ではResumeは成功したが、Approval時にCLIが`Working...`のまま停止し、`turn/started`が発生しなかったため失敗として正しく棄却した。この結果を受け、Resume／Approvalの固定promptを正式な`[PROMPT]`引数で自動投入し、Turn開始・シナリオ全体のtimeoutを追加した。ハーネス変更後の新しい検証ペアで再実施する。

検証ペア`gate-a-final5-20260720`のResumeでは、自動投入したTurn自体は開始・完了したが、Observer追記中のsanitized logをrunnerが同時に読む際にWindowsの一時ファイルロックへ衝突したため、結果を失敗として棄却した。共有読み取りと短時間retryへ変更し、排他書き込み中のログを読むself-testを追加した。ハーネス変更後の新しい検証ペアで再実施する。

検証ペア`gate-a-final6-20260720`ではResumeが成功し、Approvalも同一Thread／Turnへ相関した`item/commandExecution/requestApproval`の配送とObserver未応答を確認したが、runnerがOSプロセスexit codeを取得できず棄却した。タイムアウト付き終了待機後にリダイレクト処理を同期してexit codeを整数取得する共通処理と、Observerがトップレベル`exit 42`直前に書くexit intent監査を追加した。合格条件は引き続きOS実測exit code `42`とする。

最終試行の検証ペア`gate-a-final7-20260720`では、同一ハーネス・明示指定した同一Thread IDでResumeが成功した。Approvalでは、CLI resume後の`turn/started`と同じThread ID／Turn IDを持つ`item/commandExecution/requestApproval`が、応答必須requestとしてObserverへ配送された。Observerから同じrequest IDへのoutbound responseは0件だった。Observerは`exit 42`直前のintentを記録したが、OS実測exit codeは`0`であり、安全停止のexit code要件だけは未成立とする。cleanup後の残存PIDはなく、ポート4500も解放された。

Observerへ応答必須requestが配送されないことは、Observer方式の必須成立条件である。
実測では`item/commandExecution/requestApproval`がObserverへ配送されたため、他の未確認項目の結果に関係なくObserver方式は不成立とし、**Broker必須**と判定する。

OS実測exit code `42`が成立しなかった点はハーネスの安全停止上の未解決事項として残すが、Observerへ応答必須requestが配送されたというGate Aの否定条件を覆さない。追加のCLI／model呼び出しは行わず、本試験を終了する。

## 未確認項目

- `item/tool/requestUserInput`の配送と解消
- `mcpServer/elicitation/request`の配送と解消
- pending approval／input中のCLI切断挙動
- CLI接続／切断の観測方法
- Observer自身を他接続から識別・除外する方法

これらはBroker必須の判定後にObserver試験を継続しない方針に従い、未確認のまま停止した。

## 仕様書へ反映した変更

- Gate Aの実測環境、Schema hash、配送結果を追記
- Observer方式不成立、Plan BのBroker方式採用を確定
- §4、§5、§7～§10、§18のBroker構成への改訂を次工程として明記

## 作成・変更ファイル

- `tools/codex-gate-a/run-gate-a.ps1`
- `tools/codex-gate-a/gate-a-common.ps1`
- `tools/codex-gate-a/start-app-server.ps1`
- `tools/codex-gate-a/launch-cli.ps1`
- `tools/codex-gate-a/observer.ps1`
- `tools/codex-gate-a/mcp-elicitation-server.mjs`
- `tools/codex-gate-a/test-gate-a.ps1`
- `tools/codex-gate-a/observer-self-test-server.mjs`
- `tools/codex-gate-a/process-tree-self-test.ps1`
- `tools/codex-gate-a/log-lock-self-test.ps1`
- `tools/codex-gate-a/README.md`
- `docs/codex-gate-a-results.md`
- `docs/keylink-studio-codex-screenkey-prototype-spec-reviewed-v10.md`

> 生ログ、生成Schema、token、認証情報、Thread本文、プロンプト、応答本文はこの文書へ記録していない。
