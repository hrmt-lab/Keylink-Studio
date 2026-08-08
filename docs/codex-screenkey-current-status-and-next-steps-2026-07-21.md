# Codex Broker／ScreenKeyプロトタイプの現在地と次の作業

- 最終更新日: 2026-08-08
- 用途: Codex Broker／ScreenKeyプロトタイプの実施結果、完了条件、次の開始地点を管理する進捗文書
- 主な確認元:
  - `docs/keylink-studio-codex-screenkey-prototype-spec-reviewed-v10.md`
  - `docs/claude-code-screenkey-multisession-design.md`
  - Obsidian `Keylink Studio/handoff.md`
  - Obsidian `Keylink Studio/decisions.md`
  - Obsidian `Keylink Studio/gotchas.md`
  - Obsidian `Keylink Studio/log/2026-08-08.md`
  - Obsidian `Keylink Studio/log/2026-07-20.md`
  - Obsidian `Keylink Studio/log/2026-07-21.md`
  - Gate A／Broker Gate／Gate B結果文書

## 結論

このプロトタイプは「Codexの状態をScreenKeyへ表示する」主要経路の実装・実機確認に加え、Keylink Studio、Broker、Codex App Server、Codex CLIの開始・停止・再開始ライフサイクルと停止確認UIまで完了している。

2026-07-25、完了対象を「Codex CLI `0.145.0`、Keylink Studio、Broker、
Codex App Server、Host Link v2、`zmk-rawhid-app`、ScreenKeyを接続した経路。
対応デバイス1台、ScreenKey Renderer 1件、USB接続」に確定した。
この対象範囲の実装、実機確認、構成別自動テスト、最終回帰は完了したため、
**ScreenKey単体を対象としたCodex状態表示プロトタイプは完了**とする。

次フェーズは、将来の複数セッション対応を前提にしたClaude Code状態表示とする。
初期対象はKeylink Studioから起動したWindows上のClaude Codeに限定し、hook受信、
Session Registry、一時的なScreenKey押下による表示セッション切り替え、Firmware境界を
`docs/claude-code-screenkey-multisession-design.md`に定義した。

2026-08-05、Keylink Studio内へ独立したGate C probeを追加し、Claude Code `2.1.214`で
plugin読込、`SessionStart`／`SessionEnd`、connection refused、1.5秒遅延、socket drop、
bounded queue overflow、MCP elicitation fixtureを確認した。Host core 207件、Tauri 21件、
probe 5件もPASSした。当時はClaude Codeが未ログインでOAuth refresh tokenも無効だったため、
tool、permission、input、中断、session lifecycleを再認証後へ残した。
2026-08-08、ユーザー再認証後のClaude Code `2.1.224`で、toolなし／成功／失敗／並列、
permission許可／拒否／auto、`AskUserQuestion`、MCP elicitation accept／cancel、各状態のEsc中断を
実測した。並列toolは開始順と完了順が逆転し、permission／elicitation用Notificationは約6秒遅延した。
通常のEsc中断ではPost／Stop系hookと`idle_prompt`が届かず、MCP elicitationだけが
`ElicitationResult(action = cancel)`で明示終了した。
続いてClaude Code `2.1.224`で`/clear`、`2.1.226`で手動`/compact`、`/resume`、fork、
plugin再読込を実測した。`/clear`は旧sessionを`reason = clear`で終了して新IDを
`source = clear`で開始し、手動compactは同じIDで
`PreCompact -> SessionStart(source = compact) -> PostCompact`となった。`/resume`は同じID、
forkは新IDを使用した。plugin設定変更は`/reload-plugins`後に反映された。
自動compactは最低100k token規模となるため、OS再起動直後と実AV負荷とともに今回のGate Cでは
`DEFERRED`とする。主要event／session lifecycle実測は完了した。製品実装前レビューでは、
詳細状態stale閾値を120秒、`SessionEnd`を個別session終了、wrapper通知をlaunch全体終了、
HTTP hookを1～3秒、Helper／wrapper内部送信を500 ms×最大2回と確定した。

なお、後続のCodex CLI起動ボタンで追加したWSL起動経路には、
WSL側Codex TUIのLinux形式cwdをWindows側App Serverが解釈できず、
`thread/start`が失敗する既知不具合がある。
ScreenKeyプロトタイプのWindows経路の完了判定は変更しないが、
ランチャー機能はWSL App Server起動へ修正済みで、WSL側Codex CLI `0.146.0`のschema比較と
Broker経由のThread／Turn、入力待ち、承認待ち、停止→再開始、port・token directory・WSL App Serverの後始末を確認した。
対応基準を`0.146.0`へ更新した。
原因、解消方針、将来の複数runtime境界は
`docs/codex-launcher-wsl-runtime-plan.md`を参照する。

2026-08-08、Windows側Codex CLI `0.147.0`のexperimental schemaを生成し、SHA-256
`BABFD5C98CD978DD858B4762CDFBC9FBA941E1A0E4053DE0050E4082AE1F075A`を確認した。
0.146.0から既存methodの削除はなく、Keylink Studioが使用するThread／Turn／item／approval／inputの
相関fieldは維持されているため、0.147.0を現行基準として追加した。WSL側0.146.0も検証済みの
version／schema hashペアとして継続対応する。

複数デバイス、複数Renderer、LED-only、ScreenKey以外のTarget、BLE経路、
非64-byte interfaceの実機除外確認、全実機Targetでの既存Feature回帰は
未確認のままPASSにはせず、対応実機完成後の拡張検証へ`DEFERRED`とする。

## 何を実現したいのか

目的は、Keylink StudioがWindows上でCodex App ServerとBrokerを管理し、Codex CLIのThread、Turn、承認待ちなどを構造化イベントとして取得して、Host Link v2経由でキーボードへ送ることである。

```text
Codex CLI
  ⇅
Keylink Studio Broker
  ⇅
Codex App Server
  ↓ 状態だけを抽出
AI Client State
  ↓ Host Link v2
zmk-rawhid-app
  ↓ ZMK event
ScreenKey Renderer
  ↓
128×128 LCD
```

ScreenKeyでは主に次を表現する。

- セッションなし: 消灯
- `AVAILABLE`: Codexロゴ
- `WORKING`: 青い外周アニメーション
- `WAITING_APPROVAL`／`WAITING_INPUT`: 黄色点滅
- `COMPLETED`: 緑枠を30秒表示
- `ERROR`: 赤点滅

重要なのは、Host側や共通ZMK moduleをScreenKey専用にしない設計である。

- Keylink Studio: Codex連携と汎用AI Client State
- `zmk-rawhid-app`: 汎用AI Client State Core
- 対象キーボードfirmware: ScreenKey固有描画

この境界は仕様書とObsidianの`decisions.md`で一致している。

## どこまで進んでいるか

### 完了済み

#### Gate A

- Observer方式ではapproval requestが観測接続にも届くことを実測した。
- 安全に観測専用接続を維持できないため、Broker方式を正式採用した。
- 詳細: `docs/codex-gate-a-results.md`

#### Broker成立確認

- request、response、notificationを透過転送した。
- approval requestと同一JSON-RPC IDのCLI responseも往復成功した。
- CLI側とApp Server側のtokenを分離した。
- 詳細: `docs/codex-broker-gate-results.md`

#### Gate B

- USBを抜かずにHostだけを再起動し、`HOST_HELLO`／`DEVICE_HELLO`で同一device identityとcapabilityを再取得した。
- revision `60000`の後に、再起動後のrevision `1`を受理する到着順Last-Write-Winsも実証した。
- 詳細: `docs/host-link-rehandshake-gate-b.md`

#### Keylink Studio実装

- Broker lifecycle
- Codex activity reducer
- Host Link AI Client State送信
- 5秒heartbeat
- 設定UI
- 状態・送信関連テスト

#### Firmware実装

- `zmk-rawhid-app`のAI Client State CoreとZMK event
- ScreenKey Renderer
- revision、heartbeat、timeout処理

#### 実機E2E

- Keylink Studioから連携開始
- Codex CLI接続
- Turn開始・完了
- `WORKING`、`COMPLETED`、`AVAILABLE`、inactive送信
- firmwareで受理確認

#### ScreenKey実機表示

- CodexアイコンのRGB565変換修正
- Codexロゴを64×64 pixelから96×96 pixelへ拡大し、128×128画面中央へ配置
- 黄色枠の色修正
- 反時計回り90度を正方向に確定
- 外周開始位置を左下から左辺上方向に修正
- ユーザー実機確認PASS

### 2026-07-24までに完了した追加項目

#### 開始・停止ライフサイクル

- Windows Job ObjectでKeylink Studioが起動したApp Serverのprocess treeを所有し、停止、起動失敗、manager dropで子孫ごと終了するよう修正した。
- 同一アプリ起動中の開始→停止→再開始を実機確認した。
- 停止後にport `4500`／`4501`のlistener、App Server／Brokerの子孫process、一時token directoryが残らないことを確認した。
- 再開始時にsocket error `10048`が再発しないことを確認した。
- 修正commit: `efa7078 fix: Codex停止時の子プロセス終了と不要なスキャンエラー表示を修正`

#### 起動時scan表示

- Keylink Studio起動時に不要な包括scan失敗通知が表示されないことを実機確認した。
- Host Link／Studioの個別状態、個別警告、操作エラーは維持している。

#### CLI接続中の停止確認UI

- 停止直前に最新のBroker状態を取得し、Codex CLI接続中だけ確認モーダルを表示する。
- 既定フォーカス、Enter、Esc、背景クリックはいずれもキャンセルとして扱う。
- 明示的に「停止する」を選んだ場合だけ停止処理へ進む。
- Restore側と同じ`bg-background`、`rounded-2xl`、`shadow-2xl`系のモーダル表現へ統一した。
- ユーザー実機確認PASS。
- 実装commit: `b5e8bb7 feat: confirm before stopping connected Codex CLI`

#### 2026-07-24の検証

- `cargo fmt --all -- --check`: PASS。
- `cargo test -p rawhid-host-core`: PASS（195 tests）。
- `cargo test -p rawhid-host-tauri`: PASS（14 tests）。
- `cargo build -p rawhid-host-tauri`: PASS。
- `npm --prefix ui run build`: PASS。
- `git diff --check`: PASS。

### 2026-07-25に完了した追加項目

#### Codex CLI `0.145.0` App Server互換性

- 更新前の既知正常値は`codex-cli 0.144.6`、生成Schema SHA-256は
  `85EA836927D6CFDD3C68A9BDA17DBA48D2573BBC282AB2D5775A5005E40BC9C3`。
- 更新後は`codex-cli 0.145.0`、生成Schema SHA-256は
  `1F66700D1CC3DE4A5004E5614A6098878B405C7E7C5F8C9BE97FC900D0AD6C68`。
- 既存の起動前検査が`0.145.0`を安全に拒否し、App Server／Broker portと
  `keylink-codex-*`一時directoryを残さないことを確認した。
- 新旧Schemaのmethod集合は202件から208件へ増加し、削除は0件。
- Adapterが使用する14 methodと主要fieldはすべて存在し、互換な形状を維持していた。
- 対応基準を更新した後、Keylink Studio Broker経由の
  `initialize`／`initialized`／`thread/start`／`thread/started`を確認した。
- 公式debug clientで実モデルTurnを実行し、`turn/started`、agent response
  `COMPAT_0145_OK`、`turn/completed(completed)`を確認した。
- 開始→停止→再開始、App Server／Broker異常終了、portと一時directoryの解放を確認した。
- Adapter変更は不要と判定し、`SUPPORTED_CODEX_VERSION`と
  `SUPPORTED_SCHEMA_SHA256`を`0.145.0`へ更新した。

#### App Server起動失敗

- 「Codex 実行ファイル」に存在しないパス`C:\__keylink_start_failure_test__\codex.exe`を設定し、実行ファイル不在による起動失敗を注入した。
- UIの起動失敗表示、port `4500`／`4501`、一時token directoryの後始末を確認した。
- 設定を元へ戻した後に連携を再開始できることを確認した。
- 追加ハーネスでApp Server port競合、Broker port競合、初期化エラー、
  10秒listen timeoutも確認した。
- いずれもUI相当の状態が`Error`となり、接続情報を生成せず、port、
  管理対象process、`keylink-codex-*`一時directoryを残さなかった。
- 仕様§19.13の4ケースをPASSとした。

#### App Server／Broker異常終了

- App Serverはport `4500`の所有processを特定し、対象のnative `codex` processだけを終了して異常終了を注入した。
- BrokerはKeylink Studio内のTokio taskであり外部processとして安全に終了できないため、検証時は一時的なdebug専用経路からtaskだけをabortした。この注入経路は検証完了後に製品コードから削除した。
- いずれもUIがエラーへ遷移し、相手側とCLI接続が停止し、自動再起動しないことを確認した。
- port `4500`／`4501`と、その試験で作成された一時token directoryが解放され、条件を戻した後に再開始できることを確認した。
- 仕様§19.14のApp Server／Broker両ケースをPASSとした。

#### 実機ライフサイクル

- USB再列挙: ScreenKeyを抜き差しし、対応device数の減少・復帰と、次のTurnを待たずに現在の`AVAILABLE`が再送されることを確認した。仕様§19.15 PASS。
- PCスリープ・復帰: App Server、WebSocket、Raw HIDが復帰し、状態と表示が再同期することを確認した。仕様§19.35 PASS。
- `COMPLETED`表示: 緑枠が約30秒後に消え、Codexロゴは残り、heartbeatでは30秒timerが再開始せず、次のTurnも正常に表示されることを確認した。現在のScreenKey構成について仕様§19.43 PASS。
- 対応デバイス0台からの後挿し: ScreenKey未接続でもCodex連携を開始でき、
  後から接続すると次のTurnを待たず現在の`AVAILABLE`が送信され、
  対応device数が0台から1台へ変わることを確認した。仕様§19.24 PASS。
- 複数デバイス: 対応deviceが1台しかないため、仕様§19.23は
  対応実機完成後の拡張検証へ`DEFERRED`とした。

#### 全activity／30分連続動作

- `AVAILABLE`、`WORKING`、`WAITING_APPROVAL`、`WAITING_INPUT`、
  `COMPLETED`、`ERROR`、`NONE`のScreenKey表示を実機で確認した。
- 仕様§19.26の5ケースを各30分確認した。
  - `WORKING`
  - `WAITING_APPROVAL`または`WAITING_INPUT`
  - `ERROR`
  - `COMPLETED`
  - 状態遷移の反復
- 描画停止、周期異常、ロゴ破損、Raw HIDエラー、明確なメモリ／CPU異常は
  発生しなかった。仕様§19.26 PASS。

#### Core／Capability構成別確認

- `zmk-rawhid-app`の状態モデルtestとCore単体testをホスト実行し、PASSした。
- heartbeatで`state_generation`を維持し、状態変化、同revision異Payload、
  revision逆行、session終了、Host timeout、timeout後の同一Payload再受理を確認した。
- 実際の`rawhid_app_identity_get_capabilities()`を次の3構成でcompile・実行した。
  - Core無効、Renderer 0件: `CAP_AI_CLIENT_STATE`なし。
  - Core有効、Renderer 0件: `CAP_AI_CLIENT_STATE`なし。
  - Core有効、Renderer 1件: `CAP_AI_CLIENT_STATE`あり。
- 仕様§19.38の静的Capability初期化条件と、仕様§19.41のCore単体／
  Renderer 0件条件をPASSとした。

#### Host Link v2共有基盤

- ScreenKeyについて、64 byte Host Link Payload
  （HID API bufferはReport IDを含む65 bytes）で`HOST_HELLO`、
  `DEVICE_HELLO`、`STATE_UPDATE`、`NONE`が動作することを実機確認した。
- USB再列挙後も通信が復帰し、size mismatch、partial write、
  Raw HID送信エラーは発生しなかった。仕様§19.36はScreenKey範囲で
  `PARTIAL PASS`とする。
- Keylink Studioの既存Feature回帰はCore 196 tests、Tauri 14 testsと
  UI production buildで合格した。全実機Targetの回帰は仕様§19.37の
  `DEFERRED`部分として残す。

#### 検証用fault injectionの撤去

- Broker異常終了とTurn失敗（ERROR）の実機検証に使用したdebug専用の注入経路は、検証完了後に製品コードへ残さない方針へ変更した。
- Coreの注入API、Tauri command、状態フラグ、Settings UI、翻訳を削除した。
- 通常のBroker異常終了検知、実際のTurn失敗から`ERROR`へ遷移する処理、ScreenKeyの`ERROR`表示は維持する。

#### 2026-07-25の検証

- `cargo fmt --all -- --check`: PASS。
- `cargo test -p rawhid-host-core`: PASS（196 tests）。
- `cargo test -p rawhid-host-tauri`: PASS（14 tests）。
- `cargo build -p rawhid-host-tauri`: PASS。
- `npm --prefix ui run build`: PASS。
- `git diff --check`: PASS。

### 2026-07-26の最終レビュー

- feature branchの全commitをCore、Tauri、UI、文書の順にレビューした。
- CLI切断後の3秒grace timerが残った状態で再接続と再切断が起きると、
  古いtimerが新しい`Reconnecting`状態を早期終了させる競合を検出した。
- 接続世代を導入し、古いtimerが新しい切断状態を変更できないよう修正した。
- 再現testを追加し、Core 196 tests、Tauri 14 tests、debug／release build、
  UI production build、format、差分検査が合格した。
- strict Clippyは既存コードの警告9件によりbaselineとしては未合格だが、
  今回変更したCodex Brokerには新しい指摘がないことを確認した。
- 修正commit: `530f339 fix(codex): isolate reconnect grace timers`

#### feature統合と検証用fault injection撤去

- Codex ScreenKey featureを
  `269fbd8 merge: complete Codex ScreenKey prototype`で`develop`へ統合し、
  `origin/develop`へpushした。
- 実機検証に使用したBroker異常終了とTurn失敗（ERROR）のdebug専用注入経路を、
  `3ad29e1 refactor(codex): remove debug fault injection`で製品コードから撤去した。
- 通常のBroker異常終了検知、実際のTurn失敗から`ERROR`へ遷移する処理、
  ScreenKeyの`ERROR`表示は維持した。
- 撤去後にCore 196 tests、Tauri 14 tests、debug build、release check、
  UI production build、format、差分検査が合格した。

#### ScreenKeyロゴ96×96 pixel正式採用

- 元画像
  `/home/onigiri/zmk-workspace/config/zmk-config-screenkeytest/assets/codex_icon_transparent.png`
  から96×96 pixelのRGB565 Assetを生成し、128×128画面中央へ配置した。
- 実機表示を確認し、96×96 pixelを正式サイズとして採用した。
- ScreenKey firmware commit:
  `c401779 feat: enlarge Codex logo to 96px`
- 正式UF2:
  `/home/onigiri/zmk-workspace/firmware/screenkeytest.uf2`
- UF2 SHA-256:
  `aea3340c650d2d8632db0ac40f3a330430fa76a796a00c1a4be1ca2fd6649db4`

## 次にやること

### Claude Code Gate C後の開始地点

Claude Code `2.1.224`／`2.1.226`で、主要event、Esc中断、`/clear`、手動`/compact`、
`/resume`、fork、plugin再読込を実測した。全確定runでReceiverの
`unauthorized`／`malformed`／`oversized`／通常・priority overflowは0だった。

製品実装前レビューを行い、次の初期値を確定した。

1. 最後の関連eventから120秒後、詳細表示だけを`WORKING + UNSPECIFIED`へ縮退する。
   staleはTurn終了eventにせず、`AVAILABLE`／`COMPLETED`へ遷移させない。
2. `SessionEnd`は個別session、wrapper終了通知はlaunch全体をretireする。最初の通知だけを状態へ反映する。
3. HTTP hookはevent別に1～3秒、SessionStart command hookは2秒とする。Helper／wrapper内部は
   1回500 ms、再試行1回、待機100 msとする。
4. manual permissionは`PermissionRequest`で`WAITING_APPROVAL`とし、許可を推測せず、
   Post系eventまたは120秒stale化まで維持する。

上記を基準に、Host coreへobserver plugin生成、Receiver、Helper、wrapper終了通知を実装した。
Receiverは通常128件＋priority 16件のbounded queueへnon-blocking投入し、空bodyの204を返す。
Helperとwrapperは配送失敗をClaude Codeの失敗へ変換しない。続けてClaude Code専用の
Adapter／Normalizer、`tool_use_id` tombstone、session単位の冪等Reducerを実装した。最後の関連eventから
120秒後は`WORKING + UNSPECIFIED`へ詳細だけを縮退し、`SessionEnd`／wrapper終了と同期不能通知は
終端の推測をせず安全側に処理する。Reducer単体test 10件を含むHost core 226件がPASSした。
Session Registry、Tauri launcher、Settings画面、Host Link送信まで接続した。初回実機確認は
`Codex + CLI` identityを暫定利用し、その後HostとFirmwareを正式`CLAUDE_CODE = 0x02`へ切り替えた。
Hostはcapability bit 12のない既存deviceへ未知のclient typeを送らない。

2026-08-08、Settingsから起動したClaude Codeでobserver／Registry／暫定Host Link表示を実機確認した。
toolなし応答、tool実行、manual permission、permission中Escから120秒後の`WORKING + UNSPECIFIED`への
縮退、次promptでの回復、`/clear`、`/exit`、連携停止→再起動はすべて期待どおりだった。manual permissionは
許可直後を示すhookがないため、短いcommandでは黄色から緑へ直接遷移しても正常である。

WSL正本の`zmk-rawhid-app`へClaude Code client typeとcapability bit 12、ScreenKey側へ96×96ロゴと
Renderer切り替えを実装し、UF2を書き込んだ。正式Claude Code identityで、ロゴ、実行中、許可待ち、入力待ち、
完了、`/exit`、連携停止を実機確認した。Claude Codeの完了期限修正後にHost core 236件、Tauri 27件、
UI production build、workspace全binary buildもPASSした。
失敗したcommandの同一Turn内自動再試行で、2回目の許可画面だけ青表示になった1例は既知の境界事例として残す。
Firmware側は`zmk-rawhid-app`を`1a2ee78`、`zmk-config-screenkeytest`を`7b24ec9`としてcommit済み。
両commitはそれぞれの`origin/develop`へpush済み。

同日、1台のScreenKeyでCodex／Claude Codeの表示sessionを共通に切り替えるHost側機能を追加した。
共通セレクタが現在のCodex 1件とClaude Codeの全有効sessionを保持し、`HOST_ACTION`の`cycle_ai_session`で
次候補へ進む。非選択sessionのeventは表示を奪わず、選択中session終了時だけ次へ自動移動する。
Firmware packet変更はなく、ScreenKey keymapから既存`&host_action <ID> 0`を送る。Settingsから複数の
Claude Codeを追加起動でき、各起動のReceiver／token／plugin／`launch_id`を分離する。現行Codex Brokerは
現在追跡中の1 threadだけを候補にする。自動テストはPASS、実機確認は未実施。詳細は
[`ai-session-display-switching.md`](ai-session-display-switching.md)を参照する。

その後、単一ScreenKeyのキー押下でCodex／Claude Codeを循環する実機確認を完了した。
`HOST_ACTION`が稀に届かない事象はFirmware側のRawHID送信修正後に大幅に改善し、現行実機範囲では許容する。
また、Claude Codeの緑枠表示中にCodexへ切り替え、30秒を超えてから戻すと緑枠が再表示される問題を確認した。
原因はClaude Code ReducerにHost側の30秒完了期限がなかったことであり、非選択中も期限を進めて
`AVAILABLE`へ更新するよう修正した。Firmware変更は不要で、Reducerの期限境界、新Turnによる解除、
Registry内の非選択session更新を自動テストへ追加した。修正後の実機再確認は未実施。

ScreenKey単体プロトタイプの実装、検証、feature branchの最終レビュー、
`develop`への統合・push、検証用fault injectionの撤去、96×96 pixelロゴの
実機採用まで完了した。現時点でプロトタイプ完了に必要な作業は残っていない。

Turn中の「考えている」と「何かを実行している」をScreenKeyで区別する表示細分化は、
Host、Host Link、Firmware、Rendererの実装と単一ScreenKey・USB接続での実機確認まで完了した。
`/btw`（side chat）の子Thread表示と親Thread復帰も同じ完了範囲に含む。設計と検証結果は
[`codex-screenkey-activity-detail-design.md`](codex-screenkey-activity-detail-design.md)を参照する。

| 対象 | 次の表示 | 現状との差 |
|---|---|---|
| 入力待ち（`WAITING_INPUT`） | オレンジの外周をゆっくり呼吸 | 承認待ちと共通の黄色点滅から分離 |
| 推論中 | 現行`WORKING`と同じ青色の外周をゆっくり呼吸 | 青い移動線から分離 |
| コマンド／ツール実行中 | 現行の青い外周移動線 | 表示は変更しない |
| Web／ファイル検索中 | 現行の青い外周移動線 | 表示は変更しない |

`CONNECTING`、`RECONNECTING`、`INTERRUPTED`／`CANCELLED`、`RETRYING`は
既存の上位`activity_state`は維持しつつ、Turn内の表示を`work_phase`として追加した。
承認待ち、完了、エラー、セッションなし、利用可能の表示は従来どおり維持している。

設計で、既存`activity_state`を上位状態として維持し、Turn内の
`UNSPECIFIED`／`THINKING`／`EXECUTING`／`SEARCHING`を`work_phase`として分離した。
Host Linkはbit 11 `CAP_AI_CLIENT_WORK_PHASE`でgateし、旧Firmwareへは従来6 byte、
対応Firmwareへは末尾に`work_phase`を追加した7 byte Payloadを送る。

実装は次の順で完了した。

1. `codex_broker.rs`へ`item_type`抽出、`codex_activity.rs`へitem lifecycle、
   work phase集約、debounceをtest-firstで実装する。
2. Host packet codecとcapability別の6 byte／7 byte senderを実装する。
3. WSL正本`/home/onigiri/zmk-workspace/config/zmk-rawhid-app`の
   Core／packet decode／capabilityを更新する。Windows側
   `C:\01.keyboards\OriginalKeyboards\02.SW\zmk-rawhid-app`は参照専用とし、変更しない。
   Firmware側は今後もWSL上のrepositoryを正本とし、Windows上に同名フォルダがあっても
   書き込まない。
4. ScreenKey Rendererへ入力待ちと推論中の呼吸表示を追加する。
   オレンジは`#F97316`、呼吸は20 frame、100 ms/frameの2秒周期、
   opacity 64→255→64の三角波とする。
5. Host単体test、Firmware test、fresh build、全状態遷移の実機確認を行う。

将来、対応実機が完成した時点で、下記`DEFERRED`項目を別の拡張検証として再開する。

## プロトタイプ完了条件の棚卸し

全体は仕様書§20の4区分で管理する。

| 区分 | 状態 | 完了済み | 残り |
|---|---|---|---|
| 20.1 アーキテクチャ成立 | 完了 | Gate A/B、Broker採用、透過転送、Host再握手、revision逆行LWW | なし |
| 20.2 基本機能 | 対象範囲で完了 | 開始／停止／再開始、token認証、全activity、起動失敗4ケース、異常終了、切断、USB再列挙、15秒timeout、LWW、COMPLETED 30秒、Codex CLI 0.145.0互換性、30分5ケース | なし |
| 20.3 Core／Renderer分離 | 対象範囲で完了 | Core単体、Renderer 0／1件Capability、ScreenKey Renderer、callbackのwork委譲、COMPLETEDポリシー分離 | 複数Renderer、LED-onlyを`DEFERRED` |
| 20.4 共有基盤回帰 | 対象範囲で完了 | ScreenKeyの64 byte、既存自動回帰、DEVICE_HELLO、PCスリープ・復帰、ST7735四辺 | 他Target、非64-byte interface、BLE、全実機Target回帰を`DEFERRED` |

合意したScreenKey単体の対象範囲では4区分すべて完了した。

### DEFERRED項目

| 仕様 | 項目 | 再開条件 |
|---|---|---|
| §19.23 | 対応デバイス2台以上 | 2台目の対応実機完成 |
| §19.36 | 他Target、非64-byte interface、BLE | 対象実機／経路の準備完了 |
| §19.37 | 全実機Targetの既存Feature回帰 | 対象Targetの準備完了 |
| §19.42 | 複数Renderer、callback失敗分離 | 複数Renderer実機完成 |
| §19.44 | LED-only | LED Renderer実機完成 |

## 現在のリポジトリ状態

- path: `C:\01.keyboards\OriginalKeyboards\02.SW\Keylink-Studio`
- branch: `develop`
- Claude Code ScreenKey対応は
  `cfc15ae merge: integrate Claude Code ScreenKey support`で`develop`へ統合済み。
  この統合には`28f8e18`、`58e3c57`、`5c157ac`を含む。
- 統合commitと`679ce0c docs: record Claude Code develop integration`は`origin/develop`へpush済み。
- Codex CLI `0.145.0`対応基準更新:
  `bce8bed chore(codex): support Codex CLI 0.145.0`
- プロトタイプ完了記録:
  `81c252f docs: mark Codex ScreenKey prototype complete`
- 再接続grace timer競合修正:
  `530f339 fix(codex): isolate reconnect grace timers`
- Codex ScreenKey featureは`269fbd8 merge: complete Codex ScreenKey prototype`で
  `develop`へ統合・push済み。
- debug専用fault injectionは
  `3ad29e1 refactor(codex): remove debug fault injection`で撤去・push済み。
- プロトタイプ完了記録は本書とレビュー済み仕様書に反映済み。

### ScreenKey target firmware

- path: `/home/onigiri/zmk-workspace/config/zmk-config-screenkeytest`
- branch: `feat/codex-screenkey-renderer`
- HEAD: `c401779 feat: enlarge Codex logo to 96px`
- `origin/feat/codex-screenkey-renderer`へpush済み、作業ツリーclean。
- 正式UF2:
  `/home/onigiri/zmk-workspace/firmware/screenkeytest.uf2`
- SHA-256:
  `aea3340c650d2d8632db0ac40f3a330430fa76a796a00c1a4be1ca2fd6649db4`

未追跡の`docs/keylink-studio-codex-screenkey-prototype-spec.md`と3つのzipは
ユーザー所有物として扱い、stageまたは削除しない。Gate C結果を反映した本書と
`docs/claude-code-screenkey-multisession-design.md`はClaude Code featureへcommit済みであり、
`cfc15ae`で`develop`へ統合済みである。

## 2026-08-02 Turn内状態細分化

- Keylink Studio Host側は`AiWorkPhase`、構造化`item/started`／`item/completed`、
  active item集約、150 ms／250 ms debounceを実装済み。
- Host Link v2はbit 11 `CAP_AI_CLIENT_WORK_PHASE`で7 byte形式をgateし、
  bit 10のみのdeviceには従来6 byte形式を送る。
- phase-only変更ではbase revisionを増やさず、bit 11対応deviceだけへ送る。
- Firmware／ScreenKey RendererはWSL正本で実装され、生成UF2の実機投入まで実施済み。
  実装指示の記録は`docs/codex-screenkey-work-phase-firmware-implementation-prompt.md`に残す。
- firmware正本はWSL上だけにあり、Windows上の同名folderは今後も参照専用とする。
- 実機確認で、期限切れ`COMPLETED`を保持したままUSB再接続すると緑枠が再表示される問題を確認した。
  Host snapshotを30秒後に`AVAILABLE`へ遷移させる修正を追加し、再接続時に期限切れ
  `COMPLETED`を再送しないようにした。firmware変更は不要。
- 緑枠表示中に`/btw`を実行するとScreenKeyが消灯したまま復帰しない問題を修正した。
  `/btw`（side chat）の`thread/fork` responseで子Threadを表示対象へ切り替え、親Threadを
  終了させず、子ThreadのTurnを青、完了を緑で表示する。切替時に`NONE`を送らないため
  ScreenKeyは消灯しない。親へ戻った後も、親Threadの次の`turn/started`で表示対象を自動的に
  親へ戻す。Host単体testで確認した。firmware変更は不要。
- 上記に加え、通常Turn、入力待ち、承認待ち、Turn中断、`COMPLETED`の30秒解除、
  期限後／実行中のUSB再接続、`/btw`の子Threadと親Thread復帰を実機確認した。
  すべて期待表示となり、消灯固定、ロゴ固定、期限切れ緑枠の再表示は発生しなかった。
- Host core 207件、Tauri 21件の単体test、format、`git diff --check`が成功した。
- したがって、Turn内状態細分化と`/btw`表示対応は、現在の単一ScreenKey・USB接続の
  対象範囲で完了とする。複数Renderer、LED-only、BLE、非64-byte interface、旧6 byte Firmwareの
  実機後方互換確認は従来どおり`DEFERRED`である。

## 変更しない境界

- Observer方式を再採用しない。
- Keylink Studio側はScreenKey固有にしない。
- `zmk-rawhid-app`にはAI Client State Coreまでを置く。
- ScreenKey Rendererは対象キーボードfirmware側に置く。
- Gate A／Bの生ログ、token、認証情報、一時ファイルをcommitしない。
- ユーザーのCodex認証、恒久設定、`CODEX_HOME`を変更しない。
