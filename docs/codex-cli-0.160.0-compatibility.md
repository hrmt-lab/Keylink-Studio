# Codex CLI 0.160.0 互換性更新

確認日: 2026-10-03

OpenAI Docs の [changelog](https://learn.chatgpt.com/docs/changelog) では、2026-10-01 公開の最新 Codex CLI は `0.160.0`。ローカルの `codex --version` も `codex-cli 0.160.0` だった。

同じ CLI から `codex app-server generate-json-schema --experimental --out <dir>` で生成した `codex_app_server_protocol.schemas.json` の SHA-256 は次のとおり。

`7243BA241962AF92CA60581F1A81808EBDA4212A800F8B205F54703BCFD508C5`

Broker の厳密な互換性テーブルへこの version/hash ペアを追加し、0.157.1 以下の既存ペアを維持した。既定の version check OFF でも schema hash は照合する。ON では version と対応 hash の一致も要求する。

## 0.157.1 との差分

0.157.1 の生成 schema（SHA-256 `D6D70A4B2AF4C6BB03DEE46AF2CDA9C8B7B4D656CD5A55C54F748146985CDB43`）と、ファイル名および内容を比較した。生成ファイルの追加・削除はなく、34 ファイルの内容が変化した。

- `ClientRequest` には `ThreadItemsListParams.cursor` の item anchor、`EnvironmentAddParams.authBearerToken`、`ListMcpServerStatusParams.serverName` が追加された。Broker はこれらの要求を解析しない。
- `ServerNotification` にはエラー enum の `flexUnavailable` と `tooManyDenials`、プラン enum の `promax`、Turn のエラー説明変更があった。Broker は通知を転送し、既知の event method だけを状態へ反映する。
- `ClientNotification.json` と `ServerRequest.json` は同一だった。Broker が使う `CommandExecutionRequestApprovalParams.json` と `ToolRequestUserInputParams.json` も同一だった。既存の初期化、Thread／Turn、承認・入力の Broker 経路に変更が必要な schema 差分は見つからなかった。

## 検証範囲

- ローカル CLI の version と実生成 schema hash を取得し、互換性ゲートに登録した。
- `cargo test -p rawhid-host-core`: 374 tests passed。
- `cargo test -p rawhid-host-tauri --lib`: 122 tests passed。既存の dead-code warning 1 件。
- `cargo build -p rawhid-host-tauri`: passed。既存の dead-code warning 2 件。
- `npm run build`（`ui`）: TypeScript compile と Vite production build passed。
- `tools/codex-broker-gate/test-broker.ps1`: fake App Server を使う Broker self-test passed（転送、ID、認証分離、切断、metadata）。
- `cargo fmt --all -- --check` と `git diff --check`: passed。
- `codex login status` は `Not logged in`。認証を要する Windows Broker 実 Turn は未実施。WSL、Tauri GUI、実 ScreenKey／HID も未実施。
