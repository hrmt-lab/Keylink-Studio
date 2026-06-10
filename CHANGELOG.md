# Changelog

All notable changes to RawHID Host are documented in this file.

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
- RawHID Host の構成、Rust core、Tauri UI、ZMK firmware 側との関係を整理

## [0.1.1] - 2026-05-26

### Changed

- Release build 手順と配布物の扱いを整理
- example config を source 管理対象として扱う方針を整理
- 生成物は GitHub Releases に置き、repository には含めない運用を明確化

## [0.1.0] - 2026-05-24

### Added

- RawHID Host の初期版
- Windows 前面アプリ監視
- `path` / `exe` / `title` によるアプリ判定
- Raw HID device scan
- `HOST_HELLO` / `DEVICE_HELLO` verification
- ZMK layer switching 用 `APP_LAYER` packet 送信
- `TIME_SYNC` packet 送信
- Rust core / CLI / Tauri + React UI の基本構成
- `rawhid-host.toml` による設定管理
