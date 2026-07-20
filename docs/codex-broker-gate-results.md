# Codex Broker Gate 結果

## 現在の判定

- Gate A最終判定: Observer方式不成立、Broker方式必須
- Brokerローカルself-test: 合格
- 偽CodexによるPowerShellハーネス全体テスト: 合格
- 通常のWindows PowerShellからの実Codex E2E: 合格
- Broker方式の実環境成立判定: 成立
- Gate B、本体機能、仕様書§8～§10に記載した本体状態管理: 未実装

Codex CLI 0.144.6では、区間別認証を行うBroker経由でrequest／response／notificationと
応答必須server requestを透過転送できることが確認できた。

## 固定した検証資産

| 項目 | 値 |
|---|---|
| Broker harness SHA-256 | `1FDA0D8B971F6C22FC5426EA5498E7DC75F2906A90F93C6428A397D8E8C1C983` |
| Gate A生成Schema SHA-256 | `85EA836927D6CFDD3C68A9BDA17DBA48D2573BBC282AB2D5775A5005E40BC9C3` |
| 対象Codex CLI | `0.144.6` |
| App Server listen option | `--listen` |
| App Server認証option | `--ws-auth capability-token` |
| App Server token option | `--ws-token-file` |
| CLI remote option | `--remote` |
| CLI token環境変数option | `--remote-auth-token-env` |

## 実Codex E2E

| 項目 | 結果 |
|---|---|
| run ID | `20260720-151439-3e34282e` |
| status | `passed` |
| Codex CLI | `codex-cli 0.144.6` |
| App Server | `ws://127.0.0.1:4500` |
| Broker | `ws://127.0.0.1:4501` |
| CLI exit code | `0` |
| approval method | `item/commandExecution/requestApproval` |
| approval JSON-RPC ID | `0` |
| matching CLI response | App Serverへの転送成功 |

`initialize`／`initialized`、`thread/start`、`turn/start`、`turn/started`、approval request／response、
`serverRequest/resolved`、`turn/completed`、`thread/unsubscribe`／responseの配送を確認した。

CLI終了時は`thread/unsubscribe`のresponse受信後にCLI側`read ECONNRESET`を観測した。
CLI exit codeは0で、`forward_failed`は記録されていないため、CLI終了に伴う切断として扱う。

## 確認済みの配送

| 方向 | JSON-RPC種別 | 結果 |
|---|---|---|
| CLI → App Server | request | 本文とIDを保持して配送成功 |
| App Server → CLI | response | 本文とIDを保持して配送成功 |
| App Server → CLI | notification | 本文を保持して配送成功 |
| App Server → CLI | server request | methodとIDを保持して配送成功 |
| CLI → App Server | server response | 対応するserver requestと同じIDで配送成功 |
| CLI → Broker切断 | close | App Server側へ切断伝播成功 |
| App Server → Broker切断 | close | CLI側へ切断伝播成功 |

認証なし／不正tokenはHTTP 401、追跡中CLIがある状態の2本目はHTTP 409で拒否した。
CLI→Broker tokenとBroker→App Server tokenは別値であり、Authorization headerを区間越しに転送しない。

## 実Codex E2E初回preflight

初回実行はモデル呼び出し前のhelp検査で停止した。Codex CLI 0.144.6の実表示が
`--remote <ADDR>`であるのに対し、ハーネスが表示専用metavarを`--remote <URL>`へ固定していたことが原因である。
オプション名`--remote`を行単位で確認し、metavar名には依存しない検査へ修正した。
偽Codexのhelpも`--remote <ADDR>`へ変更して回帰テスト済みである。

この停止ではApp Server、Broker、Codex Turnを開始しておらず、モデル利用は発生していない。

## 記録方針

`run.json`とBroker metadataには、方向、種別、method、ID、Thread ID、Turn ID、status、version、
スクリプトSHA-256だけを記録する。token、認証情報、prompt、response本文、tool引数は記録しない。
raw stdout／stderr、生成Schema、実行runは`target/`配下に置き、Gitへコミットしない。

## 最終判定

Broker方式を採用可能と判定する。Broker透過転送の追加実行は不要である。
次工程はGate Bであり、Brokerハーネスの再修正・再試験やKeylink Studio本体機能の先行実装は行わない。
