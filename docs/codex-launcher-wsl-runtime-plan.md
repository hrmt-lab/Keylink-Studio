# CodexランチャーのWSL実行環境不一致と解消方針

- 記録日: 2026-08-02
- 対象: Keylink Studioの`Codexを開く`からWSL版Codex CLIを起動する経路
- 状態: runtime選択・WSL App Server起動・WSL preflightを実装。実機E2EはCodex CLI互換性ゲートで保留。

## 2026-08-02の実装・確認結果

- `CodexAppServerRuntime`を追加し、Settingsで`WSL`を選んだ場合は指定ディストリビューションのCodexを使ってApp Serverを起動するようにした。
- WSL preflightはシェル経由でCodexを解決し、versionとexperimental App Server schema SHA-256を検証する。Windows一時token fileは`wslpath -u`でLinux形式へ変換し、token値はcommand lineへ渡さない。
- このPCのUbuntuでは`codex-cli 0.146.0`、schema SHA-256は`D3992FEC1398AFDBEC658DA2C720C6993FBF3C1CE4900785694D2196679EDDFC`だった。0.145.0とのschema比較では削除0件、追加は未使用の`externalAgentConfig/import/recordHistory`のみで、Brokerが扱う初期化、Thread、Turn、承認、入力要求の定義は同一だった。
- schema比較と公式App Server実通信を根拠に、対応基準を0.146.0へ更新した。Broker経由で`thread/start`とTurn、入力待ち、承認待ちを確認し、停止後はWSL App Server、port `4500`／`4501`、一時token directoryが解放されること、再開始後にWSL App Serverが1件だけ起動することを確認した。

## 現象

WSL2のmirrored networkingを有効にしてWSLを再起動した後、
Keylink Studioで実行環境を`WSL`にしてCodexを開くと、Codex TUIが次のエラーで停止する。

```text
Error: Failed to start a fresh session through the app server:
thread/start failed during TUI bootstrap:
thread/start failed:
Invalid request: AbsolutePathBuf deserialized without a base path (code -32600)
```

このエラーでは`thread/start`まで到達しているため、WSL側Codex TUIから
Windows側BrokerへのWebSocket接続とtoken認証は成立している。
`.wslconfig`やmirrored networkingの設定失敗ではない。

## 原因

現在の実装では、Codex TUIだけをWSLで起動し、Codex App Serverは
Keylink StudioからWindows processとして起動している。

```text
WSL Codex TUI
  cwd: /home/... または /mnt/c/...
        |
        | thread/start（Linux形式cwd）
        v
Windows Keylink Studio Broker
        |
        v
Windows Codex App Server
```

WSL側ランチャーは次の両方でLinux形式のproject pathを指定する。

- `wsl.exe --cd <Linux project path>`
- `codex -C <Linux project path> --remote <Windows Broker>`

Codex TUIはこのcwdを`thread/start`へ入れる。
一方、受信側のApp ServerはWindowsで動いているため、`/home/...`や`/mnt/c/...`を
Windowsの絶対パスとして解釈できない。Codex `0.145.0`のApp Server schemaでも、
作業パスは実行環境側で有効な絶対パスとして扱われる。

したがって、WSL側のproject pathを別のLinux pathへ変更することや、
mirrored networkingを再設定することでは根本解決しない。

## 解消方針

### 当面の対応範囲

WindowsとWSLを同時には使用せず、Settingsで選択した実行環境に対応する
App Serverを1つだけ起動する。

```text
Windows選択:
  Windows Codex TUI
    <-> Windows Broker
    <-> Windows App Server

WSL選択:
  WSL Codex TUI
    <-> Windows Broker
    <-> 同じWSLディストリビューションのApp Server
```

BrokerとKeylink Studio UIはWindows側に残す。
App Server、Codex TUI、cwd、コマンド実行環境を同じOS／WSLディストリビューションへ
揃えることで、OSをまたぐpath変換を不要にする。

### 実装時に変更するもの

1. Codex連携開始時に、ランチャーで選択されている実行環境を確定する。
2. Windows選択時は、現在どおりWindows側Codexでpreflight、schema確認、
   App Server起動を行う。
3. WSL選択時は、指定されたディストリビューション内のCodexでpreflight、
   schema確認、App Server起動を行う。
4. WSL側App Serverへ渡すcwd、token file path、実行ファイルpathは
   Linux形式に統一する。token値をcommand lineへ含めない。
5. Windows Brokerから選択中App Serverへの接続情報を環境ごとに管理し、
   Windows固定のApp Server executable／process前提を分離する。
6. Windows Job Objectだけに依存せず、WSL側App ServerのPID／process groupを
   識別して、停止、起動失敗、Keylink Studio終了時に確実に終了させる。
7. 環境切替時は現在のApp Server、Broker接続、一時tokenを停止・解放してから、
   選択先の環境で再起動する。

WSL pathをUNC pathへ変換してWindows App Serverへ渡す方法は、当面の解消策にしない。
その方法ではTUIだけがWSLとなり、ツールやcommandの実行主体はWindowsのままになる。
また、Linux filesystem、shell、権限、sandboxをWSL-nativeにできないため、
「WSLでCodexを起動する」という設定名と動作が一致しない。

## 将来の複数セッション対応

App Serverはセッションごとではなく、実行環境ごとに分離する。
同じ実行環境内の複数sessionは、可能な限り1つのApp Serverで管理する。

| 同時利用する環境 | 基本となるApp Server数 |
|---|---:|
| Windowsのみ | 1 |
| 同じWSLディストリビューションのみ | 1 |
| Windows＋Ubuntu | 2 |
| Windows＋Ubuntu＋Debian | 3 |

複数環境を同時利用する段階では、Keylink Studioに次を追加する。

- runtimeごとのApp Server／Broker接続manager
- runtime、App Server、client、thread／sessionの対応付け
- runtimeごとのport、token、一時file、process lifecycle管理
- 複数sessionのactivity集約
- ScreenKeyへ表示するsessionの選択または優先順位
- 一方のruntime障害が他方を停止させない分離

今回の修正では同時利用まで実装せず、単一のruntime managerを
将来複数へ拡張できる責務境界にしておく。

## 修正後の確認項目

- Windows projectで従来の起動、接続、停止、再起動が回帰しない。
- WSLの`/home/...`配下projectで`thread/start`が成功する。
- WSLの`/mnt/c/...`配下projectでも`thread/start`が成功する。
- WSL内でtool／shell commandが実行され、Windows側へフォールバックしない。
- App ServerとTUIが同じCodex version／schema基準を使用する。
- Broker経由でThread、Turn、approval、input requestのactivityを取得できる。
- 停止、起動失敗、Keylink Studio終了後にApp Server、listener、
  token fileが残らない。
- Windows／WSL切替後に古いruntimeの接続や状態が残らない。
- token値がcommand line、UI、logへ露出しない。

## 関連

- `docs/manual-app-usage.md`
- `docs/codex-screenkey-current-status-and-next-steps-2026-07-21.md`
- `crates/rawhid-host-tauri/src/codex_launcher.rs`
- `crates/rawhid-host-core/src/codex_broker.rs`
