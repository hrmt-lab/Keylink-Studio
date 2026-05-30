# RawHID Host

RawHID Host は、Windows 上で動く ZMK キーボード向けのホストアプリです。
前面にあるアプリに応じてキーボードのレイヤーを切り替えたり、キーボードの表示用に時刻や AI 使用量の情報を Raw HID で送信できます。

このリポジトリに含まれるのは **PC 側のホストアプリのみ** です。キーボード側の ZMK ファームウェアには、`HL` packet protocol を Raw HID で受け取る実装が別途必要です。

## 主な機能

- Windows の前面アプリ / プロセス監視
- `path` / `exe` / `title` によるアプリ判定
- アプリごとの ZMK レイヤー切り替え
- Raw HID デバイス列挙と `HOST_HELLO` / `DEVICE_HELLO` 検証
- 検証済みデバイスへの packet 送信
- キーボード表示向けの `TIME_SYNC`
- Codex / Claude Code 使用量を送る `AI_USAGE`
- Tauri + React の GUI
- CLI によるデバッグ / スクリプト実行

## 構成

```text
rawhid-host/
├─ crates/
│  ├─ rawhid-host-core/   # 設定、packet、HID、runner、AI usage などの中核処理
│  ├─ rawhid-host-cli/    # CLI
│  └─ rawhid-host-tauri/  # Tauri コマンドと監視スレッド
├─ ui/                    # React + TypeScript + Vite UI
├─ docs/                  # 詳細ドキュメント
├─ examples/              # 設定例
├─ dev.ps1
└─ build-release.ps1
```

## クイックスタート

GUI を開発モードで起動:

```powershell
.\dev.ps1
```

Raw HID 候補を確認:

```powershell
cargo run -p rawhid-host-cli -- list-devices
```

CLI で監視を開始:

```powershell
cargo run -p rawhid-host-cli -- run
```

設定ファイル例を作成:

```powershell
cargo run -p rawhid-host-cli -- init-config --output rawhid-host.toml
```

設定ファイルの探索先を確認:

```powershell
cargo run -p rawhid-host-cli -- config-path
```

## 設定例

```toml
[polling]
interval_ms = 500

[hid]
usage_page = 65376 # 0xFF60
usage = 97         # 0x61
hello_timeout_ms = 200

[layer_switch]
enabled = true

[[layer_switch.rules]]
name = "Notepad"
exe = "notepad.exe"
layer = 1

[time]
enabled = false
format_hint = "time_hm"
clock_mode = "24h"
periodic_sync_sec = 60
# tz_offset_min = 540

[ai_usage]
enabled = false
poll_interval_sec = 300
stale_after_sec = 900

[ai_usage.codex]
enabled = true
# sessions_dir = "C:\\Users\\<user>\\.codex\\sessions"
history_fallback_enabled = true
allow_activity_baseline = false
activity_five_hour_token_baseline = 0
activity_seven_day_token_baseline = 0

[ai_usage.claude_code]
enabled = true
# credentials_path = "C:\\Users\\<user>\\.claude\\.credentials.json"
api_timeout_sec = 10
```

`activity_*_token_baseline` は Codex local history fallback の割合表示に使う仮の分母です。実際の quota limit ではありません。

## GUI 画面

- Dashboard: 監視開始 / 停止、接続状況、現在レイヤー、ログ、AI Usage 簡易サマリ
- Layer Rules: アプリごとのレイヤールール編集。変更は自動保存です。
- Time Sync: `TIME_SYNC` の有効化、表示形式、同期間隔などの設定
- AI Usage: Codex / Claude Code 使用量送信の設定、状態表示、手動更新
- Devices: Raw HID 候補のスキャンと HELLO 検証結果
- Settings: polling と HID の基本設定

UI は日本語 / 英語切り替えに対応しています。

## AI Usage について

AI Usage は既定で無効です。有効にすると Codex / Claude Code の 5h / 7d 使用率と reset 時刻を `AI_USAGE` packet として送信します。

- Codex は session history 内の `rate_limits` を優先します。取得できた場合は quota source として扱います。
- Codex local history fallback は activity estimate です。quota ではないため reset 時刻は送りません。
- Claude Code は OAuth usage API を experimental / best-effort source として扱います。schema 変更、認証失敗、token 期限切れ、credentials 不在が起こり得ます。
- access token、credentials JSON、Authorization header、HTTP body、API response、raw parse error は UI / log / status / Raw HID packet に出しません。

## ビルド

UI のみ:

```powershell
cd ui
npm run build
```

Rust / CLI:

```powershell
cargo build
```

Tauri 開発起動:

```powershell
.\dev.ps1
```

配布用ビルド:

```powershell
.\build-release.ps1
```

生成物は `target/` と `ui/dist/` に作られます。これらはリポジトリに含めず、配布物は GitHub Releases に添付する運用を想定しています。

## 詳細ドキュメント

- [セットアップガイド](docs/manual-setup.md)
- [アプリ操作マニュアル](docs/manual-app-usage.md)
- [技術スタックと仕組み](docs/technology-overview.md)
- [技術仕様](docs/spec.md)
- [Packet 仕様](docs/packet-spec.md)

---

## English Summary

RawHID Host is a Windows host application for ZMK keyboards. It monitors the foreground application, sends app-layer packets over Raw HID, can synchronize time for keyboard displays, and can optionally send Codex / Claude Code usage snapshots.

This repository contains the host-side app only. The ZMK firmware side must implement the compatible `HL` packet receiver.

See the documentation under `docs/` for setup, usage, architecture, and packet details.
