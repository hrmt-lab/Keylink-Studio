# Gate B: Host再起動後の再握手

## 対象

Gate Bでは、FirmwareとUSB接続を維持したままHostだけを再起動し、Host Link v2の
Capabilityとdevice identityを再取得できることを検証する。Packet fingerprintの
リセット、Codex Broker、ScreenKey描画は対象外とする。

## 数値割り当て

2026-07-20時点のKeylink Studioと`zmk-rawhid-app`の実装を突合し、既存割り当てと
衝突しない次の値をGate Bで固定する。

| 項目 | 値 | 根拠 |
|---|---:|---|
| `FEATURE_SYSTEM` | `0x00` | 既存`HOST_HELLO`が使用している値 |
| `FEATURE_AI_CLIENT` | `0x0A` | Config RPCのfeature `0x01`、`0x02`と分離した未使用値 |
| `PACKET_STATE_UPDATE` | `0xA0` | 既存packet typeの次の未使用Feature帯 |
| `CAP_AI_CLIENT_STATE` | `1 << 10` | capability bit 0～9の次の未使用bit |

`STATE_UPDATE`では`op = 0x00`、`status_or_flags = 0x00`を使用する。

## 自動試験

Host単体試験では次を確認する。

1. 初回の`DEVICE_HELLO`応答を2回失っても、500ms timeout・最大3回で復旧する
2. 初回3回とも未応答の候補には、5秒巡回時に1回だけ再送する
3. 同じdevice identityの応答を複数受信しても登録を1件へ集約する
4. capabilityが変化した重複応答では登録情報を更新する
5. `CAP_AI_CLIENT_STATE`対応デバイスだけへ`STATE_UPDATE`を送る

Firmware側では次を確認する。

1. 空Payloadの`HOST_HELLO`ごとに同じseqの`DEVICE_HELLO`を返す
2. 応答には現在のcapabilityとdevice identityを含める
3. 不正な`HOST_HELLO`には応答しない
4. 正常な`STATE_UPDATE`をrevisionの大小に関係なく到着順で受理する

## 実機試験

1. Gate B対応Firmwareを書き込み、USB接続を維持する
2. Keylink Studioからprobeし、identityと`CAP_AI_CLIENT_STATE`を記録する
3. Keylink Studioだけを終了して再起動する
4. 同じidentityとcapabilityを再取得する
5. 現在状態を送信し、Firmwareが受理したことを確認する
6. 応答ロスト、重複応答、5秒巡回を診断用fixtureで確認する

実機結果は試験完了後に追記する。

## 2026-07-20 自動検証結果

- `cargo test --workspace`: 成功（Core 186件、Tauri 10件、その他0件）
- `cargo test -p rawhid-host-core hid::tests::`: 39件成功
  - `initial_probe_retries_hello_three_times_with_fresh_sequences`: 応答2回ロスト後に3回目で復旧
  - `unresponsive_candidate_gets_one_hello_on_periodic_retry`: 5秒巡回で1回だけ再送
  - `duplicate_device_uid_is_registered_once_and_updates_capabilities`: 同一identityを1件に集約しcapabilityを更新
- spike期間中の一時CLIから実機へ`STATE_UPDATE`を送信した。一時CLIは最終成果物から除外した
- `screenkeytest + raw_hid_adapter` Gate B build: 成功
- Firmware状態モデル単体試験: 成功
  - revision 60000の後のrevision 1を受理
  - 同一revision・同一Payloadをheartbeatとして扱う
  - 同一revision・異なるPayloadを受理して警告対象とする
  - 15秒timeout相当の状態無効化と、その後の同一状態再受理を確認
- Firmware artifact: `/home/onigiri/zmk-workspace/build/gate-b-screenkeytest/zephyr/zmk.uf2`
- Copied artifact: `/home/onigiri/zmk-workspace/firmware/screenkeytest.uf2`
- Build時に`CONFIG_RAWHID_APP_AI_CLIENT_STATE=y`を確認
- Build時に`CONFIG_RAWHID_APP_AI_CLIENT_STATE_RENDERER=y`を確認
- `git diff --check`: Host、Firmwareともに成功

## 2026-07-20 実機検証結果

- USBを抜かずに、Host CLIを終了後に再起動して再試験した。
- 両回の`HOST_HELLO`に対してFirmwareが`DEVICE_HELLO`を返し、同一identity
  `uid:8e38ae6ec37361b6`とcapability `0x000007e7`を再取得した。
- `0x000007e7`には`CAP_AI_CLIENT_STATE (0x00000400)`が含まれ、対応デバイス1台へ
  `STATE_UPDATE`を送信した。
- FirmwareのUSB CDCログで、revision `60000`の`STATE_UPDATE`受理後に、Host再起動後の
  revision `1`も受理したことを確認した。revisionの大小ではなく到着順で受理する条件を満たす。
- 各送信から15秒後に`AI client state timed out`を確認した。
- USB CDCログは実機確認時だけ有効化し、正式なGate Bハーネスからは除外した。

未確認: なし（Gate Bの範囲内）
