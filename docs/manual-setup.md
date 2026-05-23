# RawHID Host セットアップガイド / Setup Guide

## 日本語

### 前提

RawHID Host は Windows ホスト上で動作するアプリです。ZMK ファームウェア側には、Raw HID で `HL` protocol を受信する実装が別途必要です。

#### ホスト側要件

| 項目 | 要件 |
| --- | --- |
| OS | Windows 10 / 11 64-bit |
| Rust | Windows ネイティブの Rust toolchain |
| Node.js | UI / Tauri 開発に必要 |
| WebView2 | Tauri アプリ実行に必要。Windows 11 では通常インストール済み |
| USB | Raw HID は USB 接続で使用 |

#### キーボード側要件

- ZMK が動作していること
- Raw HID が有効であること
- HID Usage Page / Usage がホスト側設定と一致していること
- `hello` / `hello_response` に対応していること
- レイヤー切り替えを使う場合は `set_layer` / `clear` に対応していること
- 時刻表示を使う場合は `time_sync` に対応していること

既定の HID 値は Usage Page `0xFF60`、Usage `0x61` です。

### インストール方法

#### インストーラー版

配布版を使う場合は、GitHub Releases から Windows 用インストーラーをダウンロードして実行します。

インストーラーや署名の有無は配布方法によって変わります。配布物はリポジトリへ直接 commit せず、GitHub Releases に添付する運用を推奨します。

#### ポータブル運用

zip などで配布する場合は、展開したフォルダ内の実行ファイルを起動します。設定ファイルの場所を固定したい場合は、起動ディレクトリに `rawhid-host.toml` を置いてください。

### 開発起動

実行場所: プロジェクトルート

```powershell
.\dev.ps1
```

`dev.ps1` は必要に応じて `ui/node_modules` を用意し、Tauri development app を起動します。Vite dev server は Tauri の `beforeDevCommand` から起動されるため、通常は別ターミナルで `npm run dev` を起動する必要はありません。

### ビルド方法

目的によって実行場所とコマンドが異なります。

| 目的 | 実行場所 | コマンド |
| --- | --- | --- |
| UI だけ確認 | `ui` | `npm run build` |
| Rust / CLI 確認 | プロジェクトルート | `cargo build` |
| Tauri 開発起動 | プロジェクトルート | `.\dev.ps1` |
| 配布用ビルド | プロジェクトルート | `cargo tauri build` |

UI だけ確認:

```powershell
cd ui
npm run build
```

Rust / CLI 確認:

```powershell
cargo build
```

配布用ビルド:

```powershell
cargo tauri build
```

生成物は `target/` や `ui/dist/` に作られます。これらはリポジトリに含めません。

### CLI の使い方

実行場所: プロジェクトルート

```powershell
cargo run -p rawhid-host-cli -- config-path
cargo run -p rawhid-host-cli -- init-config --output rawhid-host.toml
cargo run -p rawhid-host-cli -- list-devices
cargo run -p rawhid-host-cli -- run
```

サブコマンドを省略すると `run` と同じ動作になります。

### 初回確認

1. キーボードを USB で接続します。
2. アプリを起動します。
3. `Devices` 画面で `Scan` を実行します。
4. 対象デバイスが表示され、HELLO が成功することを確認します。
5. `Dashboard` で監視を開始します。

HELLO に失敗する場合は、ZMK 側の Raw HID、Usage Page / Usage、reserved bytes zero、`seq` の扱いを確認してください。

### 設定ファイル

推奨: 開発中やポータブル運用では、プロジェクトルートまたは起動ディレクトリに `rawhid-host.toml` を置くと場所がわかりやすくなります。

通常の探索順:

1. CLI の `--config <path>`
2. カレントディレクトリの `rawhid-host.toml`
3. OS 標準ユーザー設定ディレクトリ内の `RawHID Host/config.toml`

Tauri debug build では、プロジェクトルートの `rawhid-host.toml` を優先して読み込みます。

設定例:

```toml
[polling]
interval_ms = 500

[hid]
usage_page = 65376
usage = 97
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
```

### GitHub 公開時に除外するもの

`.gitignore` で少なくとも次を除外してください。

```gitignore
/target/
/ui/node_modules/
/ui/dist/

rawhid-host.toml
*.log
*.tmp
*.pdb

.DS_Store
Thumbs.db
```

`rawhid-host.toml` は個人設定になりやすいため、公開する場合は `examples/` にサンプルを置く運用を推奨します。

---

## English

### Requirements

RawHID Host is a Windows host application. The ZMK firmware side must implement the `HL` protocol over Raw HID.

Host requirements:

- Windows 10 / 11 64-bit
- Native Windows Rust toolchain
- Node.js for UI / Tauri development
- WebView2 Runtime for Tauri
- USB connection to the keyboard

Keyboard requirements:

- ZMK firmware with Raw HID enabled
- Matching HID Usage Page / Usage
- `hello` / `hello_response`
- `set_layer` / `clear` for layer switching
- `time_sync` for keyboard display time sync

Default HID values are Usage Page `0xFF60` and Usage `0x61`.

### Installation

For releases, download the Windows installer or portable package from GitHub Releases.

Release artifacts should not be committed directly to the repository. Attach installers and zip files to GitHub Releases instead.

### Development Startup

Run from the project root:

```powershell
.\dev.ps1
```

`dev.ps1` prepares frontend dependencies when needed and starts the Tauri development app. Vite dev server is started by Tauri's `beforeDevCommand`.

### Build

| Purpose | Location | Command |
| --- | --- | --- |
| UI only | `ui` | `npm run build` |
| Rust / CLI | Project root | `cargo build` |
| Tauri dev app | Project root | `.\dev.ps1` |
| Release bundle | Project root | `cargo tauri build` |

Build outputs are created under `target/` and `ui/dist/`. Do not commit them.

### CLI

Run from the project root:

```powershell
cargo run -p rawhid-host-cli -- config-path
cargo run -p rawhid-host-cli -- init-config --output rawhid-host.toml
cargo run -p rawhid-host-cli -- list-devices
cargo run -p rawhid-host-cli -- run
```

When no subcommand is supplied, the CLI defaults to `run`.

### Config

Recommended for development and portable use: put `rawhid-host.toml` in the project root or launch directory.

Normal lookup order:

1. CLI `--config <path>`
2. `rawhid-host.toml` in the current working directory
3. `RawHID Host/config.toml` in the OS user config directory

Tauri debug builds prefer the project-root `rawhid-host.toml`.
