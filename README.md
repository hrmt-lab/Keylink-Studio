# Keylink Studio

[English README](README_EN.md)

Keylink Studio は、ZMK キーボードと組み合わせて使う Windows 向けのホストアプリです。使用中のアプリに応じてキーボードのレイヤーを自動で切り替えたり、PC の時刻をキーボードのディスプレイに表示したり、キーボードのキーから PC の操作を呼び出したりできます。ZMK Studio のキーマップ編集にも対応していて、レイヤー構成やエンコーダの割り当て、Combo の設定まで GUI から行えます。

このリポジトリに含まれるのは PC 側のアプリだけです。レイヤー切り替えや時刻表示などの Host Link 機能を使うには、[対応する ZMK firmware](https://github.com/hrmt-lab/zmk-rawhid-app) が必要です。キーマップ編集は ZMK Studio RPC という別経路を使うため、Host Link に対応していない firmware でも ZMK Studio 対応であれば利用できます。firmware 側の対応状況は [互換性情報](docs/compatibility.md) にまとめています。

## 主な機能

- アプリに応じたレイヤー自動切り替え。前面のアプリによって、キーボードのレイヤーを切り替えます。ルールはキーボードごとに設定可能で、キーボードによってレイヤー構成が違っても問題ありません。
- キーボードからの PC 操作。キーボード側のキーに割り当てた操作で、アプリの起動、フォルダを開く、監視の停止などを PC 側で実行できます。誤操作を避けるため既定では無効で、有効にするまでは実行されません。
- 時刻同期。PC の時刻をキーボードの画面に表示できます。表示形式や 12/24 時間表示、タイムゾーンも設定できます。
- AI 使用量の表示。Codex や Claude Code の使用率をキーボード側へ送って確認できます。こちらも既定では無効です。
- バッテリー残量の表示。対応キーボードのバッテリー残量を Devices 画面やシステムトレイのツールチップで確認できます。
- タイピング統計とキーテスター。キーごとの打鍵回数をヒートマップで見たり、押しているキーをリアルタイムに確認したりできます。
- ZMK Studio キーマップの表示・編集。レイヤーとキー割り当てを GUI で確認・編集できます。通常のキーだけでなく、レイヤー移動、タップホールド、Sticky Key、Bluetooth 操作などの behavior も扱えます。
- エンコーダと Combo の編集。対応キーボードでは、エンコーダの回転動作の割り当てや、Combo の追加・編集も GUI から行えます。
- キーマップのバックアップと復元。通常キー・エンコーダ・Combo の設定を JSON として書き出し、あとから復元できます。firmware を焼き直して設定が消えてしまった場合の保険として使えます。

## クイックスタート

アプリを試すには、開発環境を用意して以下を実行します。

```powershell
.\dev.ps1
```

これで GUI が開発モードで起動します。キーボードを USB または Bluetooth で接続し、`Devices` 画面の `スキャン` で認識状況を確認してください。初回の確認手順や firmware 側の要件は [セットアップガイド](docs/manual-setup.md) に詳しくまとめています。

配布用にビルドする場合:

```powershell
.\build-release.ps1
```

生成物は `target/` にまとまります (リポジトリには含めません)。

## GUI 画面

- デバイス: 監視の開始 / 停止、Host Link / ZMK Studio の検出状況、バッテリー残量、直近ログを確認します。
- レイヤールール: アプリごとのレイヤールールを編集します。変更はその場で自動保存されます。
- アクション: キーボードのキーから PC 操作を呼び出すバインディングを設定します。
- 時刻同期: 時刻同期の有効 / 無効、表示形式、同期間隔を設定します。
- AI使用量: Codex / Claude Code の使用量送信を設定し、状態を確認します。
- キーマップ表示: キーマップの表示・ヒートマップ・キーテスター・編集を行います。
- 設定: 外観、Polling、HID、起動時の挙動を設定します。

UI は日本語 / 英語を切り替えられ、アクセント色も Settings から変更できます。各画面の詳しい操作は [アプリ操作マニュアル](docs/manual-app-usage.md) を参照してください。

## ZMK Studio キーマップ編集

Keymap Viewer の編集モードでは、実機のキーマップをその場で書き換えられます。キーの割り当てだけでなく、対応キーボードならエンコーダの CW / CCW 割り当てや Combo の追加・編集・削除もできます。変更はキーを選んだ時点でデバイス側の未保存状態に反映されますが、再起動後も残すには `保存` が必要です。`変更を破棄` でいつでも元に戻せます。

いくつか知っておくとよい点があります。

- ZMK Studio が保存するのはデバイスの settings / NVS 側の状態で、firmware の `.keymap` ソースそのものではありません。firmware をフルイレースしたり settings reset を伴う書き込みをしたりすると、編集したキーマップが失われることがあります。
- Studio が locked の場合は編集できません。キーボード側で `&studio_unlock` を実行してから開いてください。
- エンコーダと Combo は Studio とは別の通信経路 (Host Link Config RPC) を使うため、対応 capability を持ち、Studio と同じ UID の Host Link 接続が必要です。保存や破棄が片方だけ失敗することもあり、その場合は経路ごとの結果を画面に表示します。

`Export` / `Restore` を使うと、通常キー・エンコーダ・Combo の設定をバックアップとして書き出し、復元できます。ただしこれは運用復旧用のバックアップであり、`.keymap` ソースの生成や firmware への反映は行いません。編集モードの詳しい挙動、制約、BLE 接続時の注意点などは [アプリ操作マニュアル](docs/manual-app-usage.md) にまとめています。

## AI Usage について

AI Usage は既定で無効です。有効にすると、Codex / Claude Code の 5 時間 / 7 日の使用率と reset 時刻を対応キーボードへ送信します。Codex はセッション履歴内の `rate_limits` を優先し、取得できない場合のみ local history から見積もった参考値を使います。Claude Code は OAuth usage API を experimental な情報源として扱います。いずれの場合も、access token や credentials、API レスポンスの内容そのものが UI やログに出ることはありません。

## 開発者向け情報

### 構成

```text
Keylink-Studio/
├─ crates/
│  ├─ rawhid-host-core/   # 設定、packet、HID、runner、AI usage、ZMK Studio などの中核処理
│  ├─ rawhid-host-cli/    # CLI
│  └─ rawhid-host-tauri/  # Tauri command と監視スレッド
├─ ui/                    # React + TypeScript + Vite UI
├─ docs/                  # 詳細ドキュメント
├─ examples/              # 設定例
├─ create-icons.ps1
├─ dev.ps1
└─ build-release.ps1
```

### CLI

GUI を使わずに動作確認やスクリプト実行を行う場合は CLI が使えます。

```powershell
cargo run -p rawhid-host-cli -- list-devices
cargo run -p rawhid-host-cli -- run
cargo run -p rawhid-host-cli -- init-config --output keylink-studio.toml
cargo run -p rawhid-host-cli -- config-path
```

### 設定ファイル

GUI の Settings から一通り設定できるため、通常は `keylink-studio.toml` を直接編集する必要はありません。トラブルシュートや詳細調整で設定ファイルを直接見たい場合は、項目一覧と探索順を [セットアップガイド](docs/manual-setup.md) にまとめています。

### ビルド

```powershell
cd ui && npm run build   # UI のみ
cargo build              # Rust / CLI
.\dev.ps1                # Tauri 開発起動
.\build-release.ps1      # 配布用ビルド
```

## 詳細ドキュメント

- [互換性情報](docs/compatibility.md)
- [セットアップガイド](docs/manual-setup.md)
- [アプリ操作マニュアル](docs/manual-app-usage.md)
- [技術スタックと仕組み](docs/technology-overview.md)
- [技術仕様](docs/spec.md)
- [Packet 仕様](docs/packet-spec.md)
