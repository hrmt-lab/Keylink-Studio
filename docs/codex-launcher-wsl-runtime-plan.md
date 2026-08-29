# CodexランチャーのWSL実行環境不一致と解消方針

- 記録日: 2026-08-02
- 対象: Keylink Studioの`Codexを開く`からWSL版Codex CLIを起動する経路
- 状態: runtime選択・WSL App Server起動・WSL preflightを実装。実機E2EはCodex CLI互換性ゲートで保留。

## 2026-08-02の実装・確認結果

- `CodexAppServerRuntime`を追加し、Settingsで`WSL`を選んだ場合は指定ディストリビューションのCodexを使ってApp Serverを起動するようにした。
- WSL preflightはシェル経由でCodexを解決し、versionとexperimental App Server schema SHA-256を検証する。Windows一時token fileは`wslpath -u`でLinux形式へ変換し、token値はcommand lineへ渡さない。
- このPCのUbuntuでは`codex-cli 0.146.0`、schema SHA-256は`D3992FEC1398AFDBEC658DA2C720C6993FBF3C1CE4900785694D2196679EDDFC`だった。0.145.0とのschema比較では削除0件、追加は未使用の`externalAgentConfig/import/recordHistory`のみで、Brokerが扱う初期化、Thread、Turn、承認、入力要求の定義は同一だった。
- schema比較と公式App Server実通信を根拠に、対応基準を0.146.0へ更新した。Broker経由で`thread/start`とTurn、入力待ち、承認待ちを確認し、停止後はWSL App Server、port `4500`／`4501`、一時token directoryが解放されること、再開始後にWSL App Serverが1件だけ起動することを確認した。

## 2026-08-08: Codex CLI 0.147.0対応

Windows側`codex-cli 0.147.0`からexperimental App Server schemaを再生成した。SHA-256は
`BABFD5C98CD978DD858B4762CDFBC9FBA941E1A0E4053DE0050E4082AE1F075A`である。
0.146.0との比較ではschema fileの削除は0件で、Client requestへの追加はthread sectionとplugin searchだけだった。
Keylink Studioが使用するThread／Turn／item、approval、input、`serverRequest/resolved`のmethodと主要fieldは維持されている。
`item/tool/requestUserInput`には必須field `isBlocking`が追加されたが、Adapterが相関に使う
`threadId`／`turnId`／`itemId`は不変である。

互換性ゲートはWindows側0.147.0を現行基準とし、検証済みのWSL側0.146.0もversionとschema hashの正しい組み合わせに限り受理する。
versionだけ、または別versionのschema hashだけが一致する組み合わせは引き続き拒否する。

## 2026-08-29: Codex CLI 0.150.1対応

Codex CLI `0.150.1`からexperimental App Server schemaを再生成した。SHA-256は
`E9BAD0A20736E7D3ABA18C0F04BEF59856FB212AE21049FE17D786682203CFAE`である。
Keylink Studioが使用する初期化、Thread／Turn、item、approval／input、`serverRequest/resolved`の
methodを確認し、Broker／Adapterの既存処理を維持した。

互換性ゲートは0.150.1を現行基準とし、検証済み旧版の0.149.1、0.149.0、0.147.0、WSL側0.146.0も
正しいversion／schema hashペアに限り受理する。未知schemaは引き続き拒否する。

Keylink Studioの実Brokerと0.150.1 App Serverを使い、version／schema preflight、capability-token認証、
`initialize`／`initialized`、`thread/start`、通常Turnの`turn/completed(completed)`を確認した。
入力要求とcommand approvalはschema上のmethod維持を確認し、0.150.1での実要求は未実施とする。
停止後は検証用listenerと一時token directoryが解放された。

## 2026-08-26: Codex CLI 0.149.1対応

Codex CLI `0.149.1`からexperimental App Server schemaを再生成した。SHA-256は
`4F4A8D8F53F971B97F818639F58C8D26BB68BFCDFA2D2F20572CB97E6761AB91`で、0.149.0と同一だった。
Keylink Studioが使用する初期化、Thread／Turn、item、approval／input、server requestの構造に差分がないため、
Broker／Adapter変更は不要と判定した。

互換性ゲートは0.149.1を現行基準とし、検証済み旧版の0.149.0、0.147.0、WSL側0.146.0も
正しいversion／schema hashペアに限り受理する。未知versionと不一致のschema hashは引き続き拒否する。

Keylink Studioの実Brokerと0.149.1 App Serverを使い、version／schema preflight、capability-token認証、
`initialize`／`initialized`、`thread/start`、Plan Turnの`item/tool/requestUserInput`と応答、Default Turnの
`item/commandExecution/requestApproval`と応答、`turn/completed(completed)`を確認した。
停止後は検証用listener `4560`／`4561`と一時token directoryが解放された。

頻繁なCLI更新でschemaが同一でもversion文字列だけを理由に起動不能となる問題を避けるため、
`ai_client.codex.version_check_enabled`を追加した。既定値の`false`ではWindows／WSLともversionを起動可否に使わず、
生成schemaのhashが検証済みhash集合に含まれる場合だけ起動する。`true`では従来どおりexact version／hashペアを要求する。
どちらのモードでも`codex --version`の実行、schema生成、schema hash確認は行い、未知schemaはfail closedとする。
一時ラッパーで既知schemaのCLIを未知version `codex-cli 0.149.2`として報告させ、OFFではpreflight、認証、
`initialize`、`thread/start`、入力要求、command approval、Turn完了まで成功し、ONではpreflightで拒否されることを確認した。

## 2026-08-22: Codex CLI 0.149.0対応

Codex CLI `0.149.0`からexperimental App Server schemaを再生成した。SHA-256は
`4F4A8D8F53F971B97F818639F58C8D26BB68BFCDFA2D2F20572CB97E6761AB91`である。
0.147.0との比較ではschema fileの削除はなく、Project／Queue／診断／Bedrock関連APIなどが追加された。
`thread/start`には任意の`projectId`、一部itemには任意field、approval responseには新しいenumが追加されたが、
Keylink Studioが相関に使う`initialize`、Thread／Turn／item、approval／input、`serverRequest/resolved`の
methodと必須fieldは維持されているため、Broker／Adapter変更は不要と判定した。

互換性ゲートは0.149.0を現行基準とし、検証済み旧版の0.147.0とWSL側0.146.0も正しいversion／schema hashペアに限り受理する。
未検証の0.148.0、未知version、versionと別versionのschema hashを組み合わせた場合は引き続き拒否する。

Keylink Studioの実Brokerと0.149.0 App Serverを使い、version／schema preflight、capability-token認証、
`initialize`／`initialized`、`thread/start`、Plan Turnの`item/tool/requestUserInput`と応答、Default Turnの
`item/commandExecution/requestApproval`と応答、`turn/completed(completed)`を確認した。
停止後は検証用listener `4560`／`4561`と一時token directoryが解放された。検証では一時配置した0.149.0を使い、
ユーザーのglobal Codex `0.147.0`、Keylink Studioの保存設定、認証情報を変更していない。

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

## 2026-08-02: 正式検証の完了

実装は `521d94f feat: add WSL Codex launcher runtime` としてコミットした。対応する
Codex CLI は `codex-cli 0.146.0`、experimental App Server schema の SHA-256 は
`D3992FEC1398AFDBEC658DA2C720C6993FBF3C1CE4900785694D2196679EDDFC` である。

実機で、直接の App Server (`initialize`、`thread/start`、`turn/start`) に加え、Keylink
Studio の Broker 経由で Turn、入力要求、承認要求、停止、再開始後の Turn を確認した。
停止時には App Server/Broker の listener (4500/4501) と一時 token directory が残らず、
再開始時には選択した WSL distribution 内に App Server が 1 個だけ起動することを確認した。

この版でサポートするのは **Settings で選択した 1 つの runtime** である。Windows または
選択した WSL distribution のどちらか一方を起動できる。Windows と WSL、または複数の WSL
distribution を同時に起動・管理する複数 runtime 構成は対象外であり、必要になった時点で
runtime manager と session 分離を設計してから扱う。
