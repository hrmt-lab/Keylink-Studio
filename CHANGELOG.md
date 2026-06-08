# Changelog

All notable changes to RawHID Host are documented in this file.

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
