# 変更履歴

Keylink Studio の主な変更点をこのファイルに記録します。

## [Unreleased]

## [1.1.0] - 2026-06-27

### Changed

- アプリ UI、CLI、設定ファイル名、ユーザーデータ保存先、リリース出力名、ドキュメント内のアプリ名を Keylink Studio に変更。
- アプリアイコン素材とサイドバーのロゴを現在の Keylink Studio アイコンセットに更新。

### Fixed

- `BATTERY_STATUS` を送信するデバイスで、`DEVICE_HELLO` の capability に BATTERY が含まれていない場合でも、Devices 画面とタスクトレイにバッテリー残量を表示できるように修正。

## [1.0.1] - 2026-06-27

### Changed

- Devices 画面とキーマップ編集へのナビゲーションを調整。

### Fixed

- Windows で `wsl.exe` を起動するときにコンソールウィンドウが一瞬表示される問題を修正。

## [1.0.0] - 2026-06-22

### Added

- BLE HOG 経由の Host Link 接続対応。USB HID と BLE HOG で同一 packet wire format / capability を使い、バッテリー表示・レイヤー切り替え・KEY_STATS 送受信などを BLE 経由でも利用できるようにした。
- ZMK Studio BLE transport でのキーマップ表示・編集対応。USB serial と同様に layout / keymap / behavior labels を取得し、実配列・ラベル表示・キー書き込みを BLE Studio でも行えるようにした。
- ZMK Studio キーマップの export / restore 機能を追加。現在の ZMK Studio / NVS キーマップ状態を `-keymap.json` として保存し、同ファイルからキーボードへ書き戻せるようにした。Restore は未保存変更として扱い、保存ボタンで永続化・破棄ボタンで取り消せる。
- キーマップ編集でキーへの書き込みが成功した際に、アクセントカラー塗り潰し円＋白抜き ✓ を 1.2 秒フラッシュ表示するマイクロインタラクションを追加。

### Changed

- Keymap Viewer の Host Link 紐付けを、ZMK Studio `get_device_info().serial_number` が返す 16 桁 hex UID と Host Link `device_uid_hash` の UID 優先照合へ変更。BLE Studio でもヒートマップ / キーテスターを同じ個体の統計へ紐付けられるようにした。
- Devices 画面の Host Link 一覧を `device_uid_hash` 単位で集約。同じキーボードが USB HID と BLE HOG の両方で見えている場合は 1 カードにまとめ、USB / Bluetooth の両方のアイコンと各 HID path を表示するようにした。
- キーマップ export のデフォルトファイル名を `<name>.rawhid-keymap.json` から `<name>-keymap.json` に変更した。

### Fixed

- キーテスターで同じキーの pressed event が連続受信されたとき、直近入力表示に同じ文字が重複して流れる問題を修正。
- キーテスターの `リセット` で直近入力表示が残る問題を修正し、キーボード切り替え時にも表示をクリアするようにした。

## [0.9.1] - 2026-06-18

### Changed

- 設定の保存方法を画面ごとに統一。保存ボタンのない画面 (ダッシュボード / レイヤールール) は即時保存にし、保存完了のチェック表示を廃止。設定画面の「ログイン時に起動」は保存ボタンで確定するように変更 (アクセント色は従来どおり即時反映)。
- 失敗時のエラー表示を、生のシステムエラー文字列から利用者向けの日本語 / 英語メッセージに変更。
- キーマップ編集ポップオーバー「その他」タブの Sticky 表記を `Sticky Keys` / `Sticky Layer` に変更し並び替え。

### Fixed

- キーマップ表示で、現在レイヤーを示すドットがスクロール領域の端で欠けて半円になる問題を修正。あわせてレイヤータブと編集ボタンの高さのズレを修正。

## [0.9.0] - 2026-06-17

### Added

- ZMK Studio キーマップ編集を拡張し、`MO` / `TG` / `TO`、`MT` / `LT`、Sticky、Bluetooth、Output、Mouse、Utility、System 系 behavior を追加。
- キーマップ編集にレイヤー追加 / 名前変更 / 削除を追加。
- `KEY_PRESS` uplink を使うキーテスター表示を追加。

### Changed

- キーマップ表示のレイヤータブを横スクロール化し、キーボード側のレイヤー変更で対象レイヤータブへ自動スクロールするようにした。
- キーマップ画面の編集ボタンをレイヤータブ行の右端固定に変更。
- キーマップ画面の Studio serial port 表示を削除。
- README と docs 配下を現状仕様に合わせて更新し、文字化けしていたドキュメントを日本語で復元。

### Fixed

- 編集モードの保存 / 破棄メッセージ表示を調整。
- ZMK Studio 編集中の `port_busy` や session 保持の挙動をドキュメントに明記。

## [0.8.5] - 2026-06-16

### Added

- ZMK Studio RPC 経由のキーマップ編集 v1 を追加。通常キー、透過、無効の割り当てをキーマップ画面から編集できるようにした。
- 編集セッションを保持して複数変更をステージし、保存 / 破棄 / 編集終了をまとめて扱う UI と Tauri command を追加。
- キー選択ポップオーバーに ZMK Keyboard keycode ベースのカテゴリ、検索、Names tooltip、US 配列前提の数字・記号表示を追加。

### Changed

- キーマップキャンバスを表示領域に合わせて自動縮小し、左右余白が偏らないように調整。
- キーマップ画面とダッシュボードの複数台デバイス表示を名前順に変更。デバイス画面は検知順を維持。
- 編集中は表示レイヤーがキーボード側レイヤー通知で戻らないようにし、変更したレイヤーにとどまるようにした。

### Fixed

- 編集開始直後に未変更でも「保存済み」のように見える表示を修正。
- キー選択ポップオーバーで通常数字とテンキー数字を区別しやすい表示に調整。

## [0.8.1] - 2026-06-14

### Added

- アクション画面に **「起動中アプリから選ぶ」ピッカー** を追加 (`launch`)。レイヤールールと同じ起動中アプリ一覧 (アイコン付き) から選ぶと実行ファイルパスが自動入力され、前面化判定が確実になる
- パス入力に **参照ボタン** を追加。`launch` はファイル選択ダイアログ (exe / `.lnk`)、`open_folder` はフォルダ選択ダイアログ (`tauri-plugin-dialog`)
- `launch` の **`.lnk` ショートカット / 関連付け起動** に対応 (`ShellExecuteW`)。`.lnk` は前面化判定時にリンク先 exe を解決して照合する
- `launch` バインディングに **「前面化する exe 名」の任意上書き** (折りたたみの詳細設定) を追加。ランチャー型アプリ (例: Autodesk Fusion は起動が `FusionLauncher.exe`、ウィンドウ本体が `Fusion360.exe`) で前面化を確実にするための上書き。空のときは自動判定

### Changed

- HOST_ACTION の **`launch` (アプリを起動)** を、起動済みなら**前面化**・未起動なら起動する挙動に変更。実行ファイル**名 (basename) が一致**する可視ウィンドウを探して前面化 (最小化からの復帰を含み、最大化状態は保持) し、見つからなければ起動する。判定はレイヤールールの `exe` 一致と同じ方式で、versioned / hashed フォルダにインストールされるアプリ (例: Autodesk Fusion の webdeploy) にも対応する (ベストエフォート)

## [0.8.0] - 2026-06-14

### Added

- HOST_ACTION に **`open_folder`** を追加。指定フォルダを Windows Explorer で開く。すでに同じフォルダが開かれていればそのウィンドウを前面化し、開かれていなければ新規ウィンドウで開く (Explorer 未起動時は起動する)
- `open_folder` バインディングに **`prefer_tab`** トグル (バインディング単位、既定オフ) を追加。オンのときは新規ウィンドウではなく既存 Explorer ウィンドウの新規タブで開こうとする。タブ操作の公開 API が無いためベストエフォートで、失敗時は新規ウィンドウにフォールバックする
- アクション画面で、設定済みバインディングを**編集**できるようにした (上部フォームに値を読み込んで「更新」。従来は削除→再追加が必要だった)
- システムトレイアイコンの**ツールチップにバッテリー残量**を表示。監視中はデバイス毎に1行 (例 `Corne: L 90% / R 88%`) で残量を反映し、ホバーで確認できる

### Changed

- アクション一覧を**アクションID昇順**で表示するようにした (削除→再追加しても順序が崩れない)

### Fixed

- 「このアプリの画面を開く」(`show_window`) およびトレイメニュー / トレイ左クリック / 二重起動時の前面化で、ウィンドウがタスクバーに**最小化されている場合に復帰しなかった**問題を修正 (`unminimize()` を追加)

## [0.7.0] - 2026-06-13

### Changed

- **UI デザインを全面刷新** (デザイン名「スタジオ・ガジェット」)。青みグレー背景 × 白カード × アクセント 1 色のライトテーマに変更し、操作できる要素 (ボタン / トグル / 選択状態) だけをニューモーフィズムの凹凸で表現。方針の詳細は `design-mock/ui-redesign-direction.md`
- フォントを変更しアプリに同梱 (UI: Zen Kaku Gothic New / 数値・タイムスタンプ: Spline Sans Mono)。実行時の Google Fonts 読み込みを廃止
- 稼働ステータスの緑をアクセント色に一元化 (「動いている = ランプ点灯」)。停止・無効はグレー、警告アンバーとエラー赤は従来どおり
- アプリアイコンを新デザイン (白丸 + アクセントのキーボードグリフ) に変更。`create-icons.ps1` で再生成
- マイクロインタラクションを追加: トグルのバウンス、ボタン押下の沈み込み、保存 ✓ のフェード、数値カウントアップ、レイヤー切替パルス、サイドバー / 編集行のホバー演出。`prefers-reduced-motion` で全演出を無効化
- キーマップ表示で隣接キーが融合して見えていたのを改善 (キー間に隙間を追加し、盤面を凹プレート化)
- Dashboard の AI 使用量カードから取得間隔行と Polling 注記を削除

### Added

- 設定 > 外観に**アクセント色**を追加。プリセット 6 色 + カスタム色 (カラーピッカーで追加、最大 8 件)。UI 専用設定として localStorage に保存 (設定ファイルには書かない)

## [0.6.0] - 2026-06-12

### Added

- **device→host (uplink) packet 対応**。キーボード側から host へ送る 4 つの packet type と capability bit を Host Link v1 の拡張として追加 (protocol は `v1` のまま)
  - `BATTERY_STATUS (0x40)`: 本体/左右ペリフェラルのバッテリー残量。Dashboard と Devices に表示
  - `HOST_ACTION (0x50)`: キーボードから host 側アクションを起動する仕組み。`[actions]` config の許可リスト制 (既定オフ) で、バインディングはキーボード単位 (`actions.devices."uid:..."`)。専用の「アクション」ページから UI で設定可能。組み込み action: `show_window` / `start_monitoring` / `stop_monitoring` / `refresh_ai_usage` / `launch`。未定義 id はログのみ
  - `KEY_STATS (0x60)`: キー位置別の打鍵数差分。日別にローカルファイルへ永続化 (`[stats]` config、既定オン)
  - `LAYER_STATE (0x70)`: キーボード側レイヤー変更の逆同期 (表示専用。ルールエンジンには影響しない)
- キーマップビューアーに「ヒートマップ」タブを追加。物理レイアウト上に打鍵数を色分け表示し、期間フィルタ (今日/7日/全期間)、総打鍵数、TOP5、左右バランスを表示
- キーマップビューアーのレイヤーボタンに、キーボードが報告した現在レイヤーをハイライト表示 (シリアル番号での best-effort 対応付け)
- Tauri command `get_key_stats` / `list_key_stats_devices`、debug ビルド限定の `debug_inject_uplink` (firmware なしでの uplink 動作確認用)
- Codex の使用量を複数 sessions ディレクトリからマージ取得。`sessions_auto_detect` (既定オン) で Windows default に加え、各 WSL ディストロの `~/.codex/sessions` (`include_wsl_sessions`、既定オン) と `extra_sessions_paths` を自動で読み込む。WSL 上の Codex CLI 使用分が host 側にも反映される (rate_limits は全ディレクトリ中で最も新しいものを採用、history fallback は全ディレクトリのトークンを合算)。AI Usage 設定ページにトグルと追加パス入力を追加

### Fixed

- HELLO 検証中に他の packet が届くと検証に失敗していた問題を修正 (応答以外の正当な HL packet を読み飛ばし、uplink packet は保持)

### Notes

- uplink は best-effort。監視停止中の packet は失われる (KEY_STATS はアンダーカウント許容)
- HOST_ACTION の応答は最大で polling 間隔 (既定 500ms) 遅延する。即時化 (専用リーダースレッド) は将来課題
- firmware 側は新 capability bit を立てた機能のみ段階的に実装可能

## [0.5.0] - 2026-06-11

### Changed

- **破壊的変更**: グローバル共通のレイヤールール (Global fallback) を廃止。レイヤールールはデバイス単位 (`layer_switch.devices."uid:..."`) でのみ設定する。デバイス専用設定がないキーボードはレイヤー切り替えの対象外になる。既存の `[[layer_switch.rules]]` は読み込み時に無視されるため、必要な場合はデバイスセクションへ手動で移行する
- レイヤールールのデバイス選択に接続状態 `(未接続)` を表示
- タイムゾーンの「自動」ラベルを短縮

### Added

- レイヤールール画面に「このデバイスの設定を削除」ボタンを追加。デバイス専用設定を UI から削除できる

### Fixed

- `input` 共通クラスの `w-full` が幅指定 (`w-20`〜`w-48`) を上書きし、入力欄・セレクトが全幅に広がっていた問題を修正

## [0.4.0] - 2026-06-11

### Added

- `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` によるフォアグラウンドアプリの即時検知。ポーリング待ちなしでレイヤーが切り替わる
- 設定 `[app] start_monitoring_on_launch` — アプリ起動時に自動で監視を開始する
- Windows ログイン時起動 — HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run に登録する UI を Settings に追加
- シングルインスタンス guard — 2重起動時は既存ウィンドウをフォーカス
- システムトレイメニューに「監視開始」「監視停止」を追加
- レイヤールールのアプリ一覧とルールカードに実アプリアイコン (exe から抽出した PNG) を表示
- サイドバーのステータスドットがレイヤー切替時にパルスアニメーションする
- 使用率バーの width 変化に CSS トランジション (500ms) を追加
- Dashboard のトグル即時保存後に「✓ 保存しました」を 2 秒表示
- `Toggle` コンポーネントに `aria-label` プロップを追加し、全主要トグルへ付与
- ハードコードされた `title` 属性を i18n 化

### Changed

- Time Sync のタイムゾーン指定を分単位の数値入力から「自動 (PC の設定) + UTC±hh:mm プリセット」のドロップダウンに変更。TOML を手編集したプリセット外の値も選択肢として保持される

### Fixed

- デバイス個別の `unmatched_action` が global 設定を正しく上書きするよう修正
- `refresh_ai_usage` が監視停止中でも完了後に `status-update` を発行するよう修正。UI の推測ロジック (`pendingRefreshSignature`) を削除
- Settings の HID 使用量 hex 入力を 0x0000–0xFFFF で検証してエラー表示するよう修正
- Dashboard のログ時刻表示を UI 言語 (ja/en) に連動させた
- Rules の「更新」「削除」ボタンの `title` を i18n 化
- `window.hide()` の `unwrap()` によるパニックを除去

### Refactored

- `ai_usage.rs`: 大きな JSONL ファイルの読み取りを末尾 4 MB に上限化し、メモリ使用量を削減
- `commands.rs`: `run_monitor_loop` の重複処理を `process_command` / `apply_runner_view` ヘルパーで整理
- `runner.rs`: 未使用フィールド `last_rule_id` を削除
- UI: `lib/format.ts` にフォーマット関数を集約、`hooks/useConfigSection.ts` で draft/dirty/save/error を共通化、`Ui.tsx` に共通 `Notice` / `SavedIndicator` コンポーネントを追加

## [0.3.1] - 2026-06-08

### Fixed

- Keymap Viewer で ZMK の回転値を正しく degree 表示するよう修正

## [0.3.0] - 2026-06-06

### Added

- ZMK Studio 対応キーボードの Keymap Viewer を追加
- AI Usage 画面と状態表示を拡充
- Devices 画面で Host Link と ZMK Studio の対応状態を確認できるように変更
- device capability に基づく機能表示と送信制御を追加

## [0.2.1] - 2026-05-30

### Fixed

- AI Usage の手動更新時の応答性を改善
- 更新中の UI 状態と worker 側の多重実行制御を改善

## [0.2.0] - 2026-05-30

### Added

- Codex / Claude Code 使用量を Raw HID で送信する `AI_USAGE` に対応
- Codex session history / rate_limits から使用率を取得
- Claude Code OAuth usage API を experimental / best-effort source として追加
- AI Usage の stale / error / estimated / quota source 情報を扱う packet 仕様を追加

## [0.1.2] - 2026-05-26

### Added

- 技術概要ドキュメントを追加
- Keylink Studio の構成、Rust core、Tauri UI、ZMK firmware 側との関係を整理

## [0.1.1] - 2026-05-26

### Changed

- Release build 手順と配布物の扱いを整理
- example config を source 管理対象として扱う方針を整理
- 生成物は GitHub Releases に置き、repository には含めない運用を明確化

## [0.1.0] - 2026-05-24

### Added

- Keylink Studio の初期版
- Windows 前面アプリ監視
- `path` / `exe` / `title` によるアプリ判定
- Raw HID device scan
- `HOST_HELLO` / `DEVICE_HELLO` verification
- ZMK layer switching 用 `APP_LAYER` packet 送信
- `TIME_SYNC` packet 送信
- Rust core / CLI / Tauri + React UI の基本構成
- `keylink-studio.toml` による設定管理
