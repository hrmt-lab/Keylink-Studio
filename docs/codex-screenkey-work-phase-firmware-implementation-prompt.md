# ScreenKey状態細分化 Firmware実装指示

以下を、WSL上のfirmware実装を担当する別Codexセッションへそのまま渡す。

---

`/home/onigiri/zmk-workspace`で、ScreenKeyのTurn内状態細分化を実装してください。

最重要のrepository境界:

- firmwareの正本はすべてWSL上にあります。
- 共通moduleの正本は
  `/home/onigiri/zmk-workspace/config/zmk-rawhid-app`です。
- ScreenKey rendererの正本は
  `/home/onigiri/zmk-workspace/config/zmk-config-screenkeytest`です。
- Windows上に同名folderが存在しても参照専用です。絶対に変更しないでください。
- 特に
  `C:\01.keyboards\OriginalKeyboards\02.SW\zmk-rawhid-app`
  は変更禁止です。
- 各repositoryとworkspaceの`AGENTS.md`、Obsidianのgotchas／handoff／最新logを先に読み、
  既存の未追跡fileやユーザー差分を保持してください。
- version bump、commit、push、実機flashは行わないでください。

Host側で確定・実装済みのcontract:

- Host Link v2のまま、既存bit 10 `CAP_AI_CLIENT_STATE`に加えてbit 11
  `CAP_AI_CLIENT_WORK_PHASE = 1 << 11`を追加します。
- bit 11対応firmwareはbit 10も必ずadvertiseします。
- `STATE_UPDATE=0xA0`、`FEATURE_AI_CLIENT=0x0A`、op/flagsは従来どおり0です。
- bit 10のみのdeviceは従来の6 byte payloadを受信します。
- bit 11対応deviceは末尾に`work_phase`を足した7 byte payloadを受信します。

Payload:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 1 | `client_type` |
| 1 | 1 | `client_variant` |
| 2 | 1 | `session_active` |
| 3 | 1 | `activity_state` |
| 4 | 2 | `revision` (`u16 LE`) |
| 6 | 1 | `work_phase`（7 byte形式のみ） |

`work_phase` enum:

- `0x00 UNSPECIFIED`
- `0x01 THINKING`
- `0x02 EXECUTING`
- `0x03 SEARCHING`

実装要件:

1. `/home/onigiri/zmk-workspace/config/zmk-rawhid-app`

   - capability bit 11を追加し、ScreenKey構成でbit 10とbit 11をadvertiseする。
   - AI Client State packet decoderが6 byteと7 byteの両方を受理するようにする。
   - 6 byte packetは`work_phase=UNSPECIFIED`として扱う。
   - 7 byte packetの未知の`work_phase`値はbase stateをrejectせず、
     `UNSPECIFIED`へnormalizeして診断可能なlogを残す。
   - session/activityの不正な組み合わせは従来どおりpacket全体をrejectする。
   - Core state、fingerprint／state equality、ZMK eventへ`work_phase`を追加する。
   - base revisionはHost側でphase-only変更時に増えない。同じrevisionでも
     `work_phase`が異なれば新状態として受理する。
   - heartbeatで同一stateが再送されても、新しいeventやanimation再始動を起こさない。
   - 既存のcore-only、renderer 0件、renderer 1件構成を壊さない。

2. `/home/onigiri/zmk-workspace/config/zmk-config-screenkeytest`

   - `WORKING + THINKING`: 青色 `#3B82F6` の呼吸する外周。
   - `WORKING + EXECUTING`、`SEARCHING`、`UNSPECIFIED`: 現行の青い移動線。
   - `WAITING_INPUT`: オレンジ `#F97316` の呼吸する外周。
   - `WAITING_APPROVAL`、`COMPLETED`、`ERROR`、`AVAILABLE`、`NONE`は既存表示を維持する。
   - 呼吸animationは20 frame、100 ms/frame、2秒周期とする。
   - opacityは64→255→64の三角波で、開始値は64とする。
   - opacity計算はrenderer modelのpure functionに置き、境界と周期をunit testする。
   - `activity_state != WORKING`では`work_phase`を表示判断に使わない。

必須test:

- 6 byte legacyと7 byte詳細形式のdecode。
- bit 11 advertise時にbit 10もadvertiseされること。
- 不正なsession/activityのreject、未知phaseのnormalize。
- 同revision・異phaseを受理し、完全同一heartbeatを重複eventにしないこと。
- Core-only、renderer 0件、renderer 1件のcapability回帰。
- Renderer modelでTHINKING／WAITING_INPUT／既存状態の選択と呼吸opacity周期。
- repository既存のAI Client State／Renderer testをすべて実行すること。

fresh build:

1. workspace rootで
   `nix develop --command just init config/zmk-config-screenkeytest`
   を実行する。
2. build参照checkout
   `/home/onigiri/zmk-workspace/zmk-rawhid-app`
   へ、正本`config/zmk-rawhid-app`における今回のtracked source差分だけを一時適用する。
   build参照checkoutにはcommitしない。作業後に元へ戻さなくてよいが、残ったdirty diffを報告する。
3. `nix develop --command just build screenkeytest -p always`を実行する。
4. `/home/onigiri/zmk-workspace/firmware/screenkeytest.uf2`の存在、更新日時、size、
   SHA-256を報告する。

完了報告には、変更file、test/build結果、未commitのgit status、build参照checkoutに残した差分、
生成UF2のSHA-256、実機E2Eが未実施であることを含めてください。

---
