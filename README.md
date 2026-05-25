# RawHID Host

## 日本語

RawHID Host は、Windows ホスト上で動作する ZMK キーボード向け Raw HID ホストアプリです。アクティブな前面アプリに応じてキーボードのレイヤーを切り替えたり、ディスプレイ付きキーボードへ現在時刻を同期したりできます。

このリポジトリは **ホスト側アプリのみ** を扱います。ZMK ファームウェア側は、別途 `zmk-raw-hid` と `HL` packet protocol に対応した受信処理が入っている前提です。

### 主な機能

- アクティブウィンドウ / プロセスの監視
- `path` / `exe` / `title` によるアプリ判定
- アプリごとの ZMK レイヤー切り替え
- Raw HID デバイス列挙と HELLO 検証
- HELLO に成功したデバイスへの packet 送信
- キーボードディスプレイ向け TIME_SYNC
- Tauri + React の GUI
- CLI によるデバッグ操作

### 構成

```text
raw-hid-host/
├── crates/
│   ├── rawhid-host-core/   # 設定、packet、HID、runner などの再利用可能な core
│   ├── rawhid-host-cli/    # CLI
│   └── rawhid-host-tauri/  # Tauri アプリ本体
├── ui/                     # React + TypeScript + Vite UI
├── docs/                   # 詳細ドキュメント
├── examples/               # 設定例
└── raw-hid-host-icon-256.png
```

### クイックスタート

開発中に GUI を起動する場合:

```powershell
.\dev.ps1
```

CLI で Raw HID デバイスを確認する場合:

```powershell
cargo run -p rawhid-host-cli -- list-devices
```

CLI で監視を開始する場合:

```powershell
cargo run -p rawhid-host-cli -- run
```

設定ファイルの雛形を作る場合:

```powershell
cargo run -p rawhid-host-cli -- init-config --output rawhid-host.toml
```

### 設定例

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
enabled = true
format_hint = "weekday_hm"
clock_mode = "24h"
periodic_sync_sec = 60
# tz_offset_min = 540
```

設定は GUI から編集できます。監視中に保存した設定は、監視スレッドへ即時反映されます。

### 設定ファイルの場所

通常の探索順:

1. CLI の `--config <path>`
2. カレントディレクトリの `rawhid-host.toml`
3. OS 標準ユーザー設定ディレクトリ内の `RawHID Host/config.toml`

開発中はプロジェクトルートの `rawhid-host.toml` を使う運用が一番わかりやすいです。配布時やポータブル運用でも、起動ディレクトリに `rawhid-host.toml` を置くと設定の場所を固定しやすくなります。

### ビルド

UI だけ確認:

```powershell
cd ui
npm run build
```

Rust / CLI を確認:

```powershell
cargo build
```

Tauri アプリを開発起動:

```powershell
.\dev.ps1
```

配布用ビルド:

```powershell
.\build-release.ps1
```

生成物は `target/` や `ui/dist/` に作られます。これらは GitHub リポジトリには含めず、配布物は GitHub Releases に添付する運用を推奨します。

### GitHub に含めるもの / 含めないもの

含めるもの:

- `Cargo.toml`
- `Cargo.lock`
- `crates/`
- `ui/` のソースと npm 設定
- `docs/`
- `examples/`
- `README.md`
- `dev.ps1`
- `build-release.ps1`
- `create-icons.ps1`
- アイコン元画像と Tauri icons

含めないもの:

- `target/`
- `ui/node_modules/`
- `ui/dist/`
- 個人用の `rawhid-host.toml`
- 生成済み exe / installer / build cache
- ログや一時ファイル

### 詳細ドキュメント

- [セットアップガイド](docs/manual-setup.md)
- [アプリ操作マニュアル](docs/manual-app-usage.md)
- [技術仕様書](docs/spec.md)
- [Packet 仕様](docs/packet-spec.md)

---

## English

RawHID Host is a Windows host application for ZMK keyboards. It can switch keyboard layers based on the active foreground application and synchronize time information to keyboards with displays.

This repository contains the **host-side application only**. The ZMK firmware side is expected to implement the `HL` packet protocol over `zmk-raw-hid`.

### Features

- Active window / process monitoring
- App matching by `path`, `exe`, or `title`
- App-specific ZMK layer switching
- Raw HID enumeration and HELLO verification
- Packet sending only to HELLO-verified devices
- TIME_SYNC for keyboard displays
- Tauri + React GUI
- CLI for debugging and scripting

### Project Layout

```text
raw-hid-host/
├── crates/
│   ├── rawhid-host-core/
│   ├── rawhid-host-cli/
│   └── rawhid-host-tauri/
├── ui/
├── docs/
├── examples/
└── raw-hid-host-icon-256.png
```

### Quick Start

Start the GUI in development mode:

```powershell
.\dev.ps1
```

List Raw HID candidates:

```powershell
cargo run -p rawhid-host-cli -- list-devices
```

Start CLI monitoring:

```powershell
cargo run -p rawhid-host-cli -- run
```

Create a sample config:

```powershell
cargo run -p rawhid-host-cli -- init-config --output rawhid-host.toml
```

### Build

Build UI only:

```powershell
cd ui
npm run build
```

Build Rust / CLI:

```powershell
cargo build
```

Start Tauri development app:

```powershell
.\dev.ps1
```

Build release bundles:

```powershell
.\build-release.ps1
```

Build outputs are generated under `target/` and `ui/dist/`. Do not commit them to the repository. Attach release artifacts to GitHub Releases instead.

### Documentation

- [Setup Guide](docs/manual-setup.md)
- [App Usage Manual](docs/manual-app-usage.md)
- [Technical Specification](docs/spec.md)
- [Packet Specification](docs/packet-spec.md)
