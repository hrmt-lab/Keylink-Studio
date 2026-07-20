# Host Link Gate B

Gate Bは、FirmwareのUSB接続を維持したままKeylink Studioだけを再起動し、
`HOST_HELLO`によるcapabilityとdevice identityの再取得を確認する。

`gate-b.conf`はGate B用にAI Client State Coreと、capability確認用のRenderer存在宣言を
有効にする。Renderer実装やUSBデバッグログは含めない。

Host側のHELLO再試行、5秒巡回、identity集約、capabilityによる送信制御:

```powershell
cargo test -p rawhid-host-core hid::tests::
```

Firmware状態モデル単体試験:

```powershell
$repoWsl = (wsl.exe wslpath -a (Get-Location).Path).Trim()
wsl.exe bash -lc "bash '$repoWsl/tools/host-link-gate-b/test-firmware-model.sh'"
```

WSLでのbuild例:

```powershell
$repoWsl = (wsl.exe wslpath -a (Get-Location).Path).Trim()
wsl.exe bash -lc "bash '$repoWsl/tools/host-link-gate-b/build-gate-b.sh'"
```

ビルド成功後、生成されたUF2は次の場所へ自動的にコピーされます。既存ファイルがある場合は更新されます。

`/home/onigiri/zmk-workspace/firmware/screenkeytest.uf2`

実機試験で使用した一時的な送信コマンドとUSBデバッグログ設定は正式コードへ残さない。
確定結果は`docs/host-link-rehandshake-gate-b.md`を参照する。
