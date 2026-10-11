# Codex CLI 0.162.1 互換性更新

確認日: 2026-10-11

OpenAI Docs の [Codex App Server ガイド](https://learn.chatgpt.com/docs/app-server) では、App Server schemaを生成するときexperimental APIを含める方法として `codex app-server generate-json-schema --experimental --out <dir>` を案内している。最新リリースは OpenAI の [Codex CLI 0.162.1](https://github.com/openai/codex/releases/tag/rust-v0.162.1)。実機にインストールされたCLIは `codex-cli 0.160.0` だったため、既存環境を更新せず `npx --package @openai/codex@0.162.1` で一時実行した。

同じexperimental schema生成コマンドで得た `codex_app_server_protocol.schemas.json` の SHA-256 は次のとおり。

`E4E7F0C7D3FD77C48CD8619CAD2BF2B315D860AB006AD5A7EBE44F6E32E666C1`

この version/hash ペアをBrokerの厳密な互換性テーブルへ追加し、従来の0.160.0以下の全version/hashペアを維持する。既定のversion check OFFでもschema hashを照合し、ONではversionと対応hashの一致を要求する。

## 0.160.0との差分

両バージョンで生成したディレクトリを相対ファイル名と内容で比較した。ファイル数は440から447に増え、削除はなく、39ファイルの内容が変化した。追加7ファイルは次のとおり。

- `v2/BedrockCheckGovCloudRequirementsParams.json` と `Response.json`
- `v2/ThreadAttachmentOwnerListParams.json` と `Response.json`
- `v2/ThreadPredictionRequestParams.json` と `Response.json`
- `v2/ThreadPredictionUpdatedNotification.json`

Brokerが転送・解析する`ClientRequest` にはThread予測要求、attachment owner一覧、turnの `parentTurnId` / `rootTurnId`、environment skills、goal mutation origin等が加わった。Brokerは未知の要求を本文解析せずそのまま中継するため、これらに専用の処理追加は不要。

`ServerNotification`ではthread prediction通知、assistant messageの `partial_answer` phase、turnの `rootTurnId`、追加のsub-agent metadata、error schemaの拡張が加わった。BrokerはJSON-RPC通知をそのまま転送し、状態には識別子、item type、turn status/error、retry flagだけを抽出する。今回の差分はこれらの抽出パスを変えない。初期化、thread/turn識別、既知のitem開始・完了、承認4系統とMCP elicitation request、`serverRequest/resolved`、ユーザー入力の関連schemaに、Broker adapter変更を要する差分はなかった。

## 検証範囲

- 公式npm配布のCodex CLI 0.162.1を一時実行し、experimental schemaを生成。グローバルのCLIは0.160.0のまま。
- 0.160.0生成schemaのhashは既存互換文書記録と一致。0.162.1生成schemaのhashは上記と一致。
- `cargo test -p rawhid-host-core`: 374 passed。
- `cargo test -p rawhid-host-tauri --lib`: 122 passed（既存dead-code warning 1件）。
- `cargo build -p rawhid-host-tauri`: passed（既存dead-code warning 2件）。
- `npm run build`（ui）: TypeScriptとVite build passed。
- `tools/codex-broker-gate/test-broker.ps1`: fake App Server/Broker self-test passed。
- `cargo fmt --all -- --check` と `git diff --check`: passed。fmtとdiff checkの標準出力は空。テスト/buildのログは `C:\\Users\\Onigiri` に保存。
- `codex login status`: Logged in using ChatGPT。
- 実Brokerの認証済みTurnは未実施。隔離CLI実行pathの照会がユーザーにより中止されたため、実Turnゲートへ進んでいない。WSL、Tauri GUI、実ScreenKey/HIDも未検証。
