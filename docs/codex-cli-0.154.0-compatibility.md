# Codex CLI 0.154.0 互換性更新

確認日: 2026-09-11

現行の互換性基準を `codex-cli 0.154.0` に更新した。実行ファイルは
`C:\Users\Onigiri\AppData\Roaming\npm\codex.cmd`、`codex --version` の出力は
`codex-cli 0.154.0` だった。

同じ実行ファイルで `app-server generate-json-schema --experimental` を実行して生成した
`codex_app_server_protocol.schemas.json` の SHA-256 は次のとおり。

`24DF528ACEC2952E6B96C1C2B061F98E60177D059E12C90CF318621380C9DE9E`

この version/hash pair を Broker の互換性ゲートの先頭に登録した。検証済みの
`codex-cli 0.153.2` とそれ以前の pair は互換テーブルに保持している。アプリケーション
version と Host Link protocol version は変更していない。

0.153.2 の実測 schema（`C:\Users\Onigiri\AppData\Local\Temp\codex-approval-probe-20260905-014551-32724\schema\codex_app_server_protocol.schemas.json`、旧 hash は
`B06F77062369D481A59CC70720C12B89CB9DD49C385863923262102D3AD6C978`）と今回の schema を
比較した。definitions は双方 91 件で、ClientRequest の method は 155 件から 159 件へ増え、
追加は `userVerification/status`、`userVerification/enroll`、`userVerification/delete`、
`userVerification/verify`、削除はありませんでした。Broker が参照する API（initialize、
thread/start、turn/start、item、approval、input、serverRequest/resolved、turn/completed）の
method 定義は双方に存在し、対象 request の必須フィールド（id、method、params）も同じです。
この比較範囲では Keylink Studio の利用 API に破壊的変更は確認されませんでした。実 Broker lifecycle
参照 definitions も比較し、`InitializeParams`、`ThreadStartParams`、`TurnStartParams`、
`CommandExecutionRequestApprovalParams/Response`、`ToolRequestUserInputParams/Response`、
`ServerRequestResolvedNotification`、`TurnCompletedNotification` は旧版と新 version で
SHA-256 が一致しました。`ThreadResumeParams` だけは差分があり、
`ConfigurationReasoning` / `ReasoningEffort` 定義と `ResponseItem` の `configuration_update`
variant が追加されましたが、既存 field の削除・型変更はありませんでした。
実 Broker lifecycle
E2E は version/schema preflight、initialize、thread/start、入力要求、approval、turn 完了、
token cleanup、4560/4561 の再 bind を確認し合格しました。WSL runtime、Tauri GUI、ScreenKey、
実機 HID は今回の検証範囲に含めません。

検証済み: core 全体 364 tests passed、Tauri lib 110 tests passed、Tauri/UI build 成功、
`codex_broker.rs` 単体 rustfmt 成功、diff check 成功。workspace 全体の fmt check は
今回変更していない `claude_activity.rs` の既存整形差分で失敗した。
初回 sandbox 内 E2E は preflight/initialize/thread start 後に timeout した。sandbox 外で再試行し、
preflight、initialize、thread/start、入力、approval、turn 完了、token cleanup、4560/4561 再 bind に合格した。

## 2026-09-11 追記: pin した hash と承認系定義の独立検証

上記とは別に、同日 Claude Code のセッションで再現確認を行った。

`codex --version` は `codex-cli 0.154.0`。同じ実行ファイルで
`codex app-server generate-json-schema --experimental --out <dir>` を実行し、生成された
`codex_app_server_protocol.schemas.json` の SHA-256 が
`24DF528ACEC2952E6B96C1C2B061F98E60177D059E12C90CF318621380C9DE9E` になることを確認した。
`codex_broker.rs` に pin した値と一致する。

あわせて、承認・入力要求まわりの定義を 0.153.2 実測 schema と全数比較した。上の比較では
対象外だったファイル変更・権限・elicitation もここに含む。

| 定義 | 0.153.2 → 0.154.0 |
|---|---|
| `CommandExecutionRequestApprovalParams` / `Response` | 同一 |
| `FileChangeRequestApprovalParams` / `Response` | 同一 |
| `PermissionsRequestApprovalResponse` | 同一 |
| `ToolRequestUserInputParams` / `Response` | 同一 |
| `McpServerElicitationRequestResponse` | 同一 |
| `PermissionsRequestApprovalParams` | 差分あり（下記1） |
| `McpServerElicitationRequestParams` | 差分あり（下記2） |

**差分1.** `PermissionsRequestApprovalParams` の `cwd` は、型参照名が
`v2/AbsolutePathBuf` から `v2/LegacyAppPathString` へ変わっただけである。フィールドの
追加・削除も、必須指定の変更も無い。

**差分2.** `McpServerElicitationRequestParams` の `mode` が 4 種から 5 種へ増えた。
新設は `openai/userVerification` で、これは 0.154.0 で `ClientRequest` に増えた
`userVerification/status` / `enroll` / `delete` / `verify` と対になる。既存 mode の削除は無い。

| mode | 必須フィールド |
|---|---|
| `openai/userVerification`（新設） | `challenge`, `description`, `mode`, `title` |
| `form` | `message`, `mode`, `requestedSchema` |
| `openai/form` | `message`, `mode`, `requestedSchema` |
| `openaiForm` | `message`, `mode`, `requestedSchema` |
| `url` | `elicitationId`, `message`, `mode`, `url` |

**新設 mode だけ `message` を持たない。** elicitation の本文を HUD へ出す実装
（`docs/ai-approval-hud-design.md` の段階5）では、`message` だけを見る作りにすると
この mode で表示が空になる。`title` と `description` を使う経路が要る。

いずれも Broker が現に使っている API の破壊的変更ではないため、0.154.0 を現行基準とする
判断は変わらない。

検証済み: `cargo test -p rawhid-host-core` 364 tests passed。
なお workspace 全体の fmt check が落ちる件は、0.154.0 対応とは無関係の
`claude_activity.rs` の既存整形差分であることを確認した。別 commit で解消する。
