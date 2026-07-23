# Codex Broker／ScreenKeyプロトタイプの現在地と次の作業

- 最終更新日: 2026-07-24
- 用途: Codex Broker／ScreenKeyプロトタイプの実施結果、完了条件、次の開始地点を管理する進捗文書
- 主な確認元:
  - `docs/keylink-studio-codex-screenkey-prototype-spec-reviewed-v10.md`
  - Obsidian `Keylink Studio/handoff.md`
  - Obsidian `Keylink Studio/decisions.md`
  - Obsidian `Keylink Studio/gotchas.md`
  - Obsidian `Keylink Studio/log/2026-07-20.md`
  - Obsidian `Keylink Studio/log/2026-07-21.md`
  - Gate A／Broker Gate／Gate B結果文書

## 結論

このプロトタイプは「Codexの状態をScreenKeyへ表示する」主要経路の実装・実機確認に加え、Keylink Studio、Broker、Codex App Server、Codex CLIの開始・停止・再開始ライフサイクルと停止確認UIまで完了している。

仕様書の完了条件は4区分ある。2026-07-24時点では、アーキテクチャ成立条件が完了、基本機能、Core／Renderer分離、共有基盤回帰の3区分は一部未確認である。このため、主要機能は成立しているが、仕様上の「プロトタイプ完了」はまだ宣言しない。

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

## 次にやること

次回は実装追加ではなく、仕様書§19／§20の未確認試験を上から確定する。

### 最初に行う試験

App Server／Brokerの起動失敗・異常終了試験から始める。

1. App Serverの起動失敗を発生させ、UIのエラー表示、Broker／子孫process、port、一時token directoryを確認する。
2. App Server稼働中に異常終了させ、BrokerとCLI接続が停止し、自動再起動しないことを確認する。
3. Broker稼働中に異常終了させ、App ServerとCLI接続が停止し、自動再起動しないことを確認する。
4. 各ケース後に再開始できることを確認する。

### その後の順序

1. USB再列挙後の再同期。
2. PCスリープ・復帰。
3. 複数デバイス。
4. Core単体、Renderer 0件、Renderer 1件以上、複数Renderer、LED-onlyの構成別試験。
5. Host Link v2既存FeatureとBLE除外条件の回帰。
6. `COMPLETED`の30秒表示と、仕様§19.26の5ケースを各30分実行する連続動作試験。

## プロトタイプ完了条件の棚卸し

全体は仕様書§20の4区分で管理する。

| 区分 | 状態 | 完了済み | 残り |
|---|---|---|---|
| 20.1 アーキテクチャ成立 | 完了 | Gate A/B、Broker採用、透過転送、Host再握手、revision逆行LWW | なし |
| 20.2 基本機能 | 一部未確認 | 開始／停止／再開始、token認証、主要activity、停止確認、15秒timeout、heartbeat、LWW | 起動失敗、異常終了、USB再列挙、全activityの最終実機回帰、COMPLETED 30秒、30分連続動作 |
| 20.3 Core／Renderer分離 | 一部未確認 | Core／eventとScreenKey Renderer実装、実機表示 | Renderer無効、Renderer 0件、複数Renderer、callback非blocking、LED-only、30秒ポリシー分離の構成別確認 |
| 20.4 共有基盤回帰 | 一部未確認 | Gate B、DEVICE_HELLO再握手、ST7735 offset／四辺外周 | 全対象の64 byte、既存Feature、BLE除外、PCスリープ・復帰 |

完了区分は4区分中1区分。残る3区分は主要実装が不足しているという意味ではなく、仕様が要求する異常系、構成別、長時間、共有基盤回帰の完了記録が不足している。

## 現在のリポジトリ状態

- path: `C:\01.keyboards\OriginalKeyboards\02.SW\Keylink-Studio`
- branch: `feat/codex-screenkey-broker-integration`
- 停止確認UIの実装commit: `b5e8bb7 feat: confirm before stopping connected Codex CLI`
- 変更は未push。正確なHEADとahead数は作業再開時に`git status --short --branch`で確認する。

未追跡の`.claude/`と`docs/keylink-studio-codex-screenkey-prototype-spec.md`はユーザー所有物として扱い、stageまたは削除しない。

## 変更しない境界

- Observer方式を再採用しない。
- Keylink Studio側はScreenKey固有にしない。
- `zmk-rawhid-app`にはAI Client State Coreまでを置く。
- ScreenKey Rendererは対象キーボードfirmware側に置く。
- Gate A／Bの生ログ、token、認証情報、一時ファイルをcommitしない。
- ユーザーのCodex認証、恒久設定、`CODEX_HOME`を変更しない。
