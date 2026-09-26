# Codex CLI 0.157.1 互換性更新

確認日: 2026-09-27

現行環境の `codex --version` は `codex-cli 0.157.1`。同じ CLI から
`codex app-server generate-json-schema --experimental --out <dir>` で生成した
`codex_app_server_protocol.schemas.json` の SHA-256 は次のとおり。

`D6D70A4B2AF4C6BB03DEE46AF2CDA9C8B7B4D656CD5A55C54F748146985CDB43`

この version/hash ペアを Broker の厳密な互換性テーブルへ追加し、旧版の検証済みペアは維持した。
互換性テーブルの既定は strict version check OFF のままだが、schema hash は常に照合する。
ON の場合は version と対応 hash の正しいペアを要求する。

## 生成 schema の確認

0.157.1 の生成 schema に Broker が使う App Server request/notification と主要定義が存在することを確認した。
承認要求と入力要求については `CommandExecutionRequestApprovalParams`、
`ToolRequestUserInputParams` が存在する。次のfieldを確認した。

| 定義 | Field | Broker の現状 |
|---|---|---|
| `CommandExecutionRequestApprovalParams` | `networkApprovalContext` | `host` と `protocol` を正規化し、承認理由欄へ表示。専用UIは未実装 |
| `ToolRequestUserInputParams` | `autoResolutionMs` | 質問の表示・回答経路では参照しない |

0.157.1 の `NetworkApprovalContext` schema には必須の `host` と `protocol` があり、独立した `port` field はない。
`host` の文字列はそのまま表示するため、port がその文字列に含まれる場合も表示される。専用のネットワーク承認UIは未実装。
`autoResolutionMs` による期限表示も未実装で、質問の回答／解決は既存の App Server 応答通知に従う。

## 検証範囲

- 公式 OpenAI Docs changelog で 2026-09-26 掲載の最新安定版が 0.157.1 であることを確認（[changelog](https://learn.chatgpt.com/docs/changelog)）。
- ローカル CLI version と生成 schema hash を取得し、ゲートへ登録する値と照合。
- `cargo test -p rawhid-host-core`: 374 tests passed。Broker対象テスト 22件も通過。
- `npm run build`（`ui`）: TypeScript compile と Vite production build passed。
- `cargo test -p rawhid-host-tauri --lib`: 122 tests passed。既存dead-code warning 1件。
- `cargo build -p rawhid-host-tauri`: passed。既存dead-code warning 2件。
- `tools/codex-broker-gate/test-broker.ps1`: Broker self-test passed（双方向転送、要求ID維持、token分離、切断処理、metadata秘匿）。
- Windows Broker の実 Codex CLI／App Server E2E は未実施。実行前確認で `codex login status` が `Not logged in`、`OPENAI_API_KEY` が未設定と判明した。利用できるAPI key providerもなかったため、認証が必要な1 Turn実行へ進めなかった。self-test はfake endpointを使うため、0.154.0 の前回実績を 0.157.1 の実測結果として扱わない。
