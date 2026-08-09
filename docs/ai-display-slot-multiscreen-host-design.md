# 複数 ScreenKey への AI session 表示

## 目的と範囲

同じキーボードに複数の ScreenKey がある将来の構成で、AI session A を
ScreenKey A、session B を ScreenKey B に同時表示できるよう、Keylink Studio
側に論理表示slotを導入する。複数のキーボードが接続される場合は、同じslotの
内容を各キーボードへ同報してよい。

Keylink Studio側の実装範囲はHostのみである。Firmware側のbit 13対応は別workspaceの
正本で実装・検証する。2026-08-09に、2台のScreenKeyを搭載したキーボードでbit 13対応
Firmwareとの実機確認を完了した。Keylink Studioは論理slot状態を全対応keyboardへ同報し、
各keyboardはslot番号を自機の物理ScreenKey番号へ対応付ける。

## 論理slotと割当

- config の `[ai_client.display] slot_count` は `1..=8`、既定は `1`。
- slot 0 は従来の単一ScreenKey選択と完全互換である。
- 各slotは `Auto` または `Pinned(target)`。
- `Auto` は既存割当を維持し、空きslotだけを Codex と Claude Code 共通の初回
  有効登録順で埋める。1 sessionを複数slotへ重複割当しない。
- `Pinned` は対象sessionが一時的に存在しなければ `NONE` を維持し、同じ正規
  識別子が戻ったときだけ再表示する。
- 既に他slotにあるsessionを固定すると、両slotの現在の割当を交換する。
- slot数を減らしたときは、取り除かれたslotへ一度 `NONE` を送ってからHost側の
  trackerを破棄する。

Codexの正規識別子は`thread_id`、Claude Codeの正規識別子は
`(launch_id, session_id)`である。Codexの接続IDは所有者追跡専用であり、表示の
識別子には用いない。

## Host Link互換性

`AI_CLIENT_STATE`の既存payloadは変更しない。

| capability | payload length | 配送 |
| --- | ---: | --- |
| bit 10 | 6 | 従来状態のみ |
| bits 10+11 | 7 | work phase付き従来状態 |
| bits 10+11+13 | 8 | 7-byte payloadの末尾に`display_slot`を追加 |

bit 13 `AI_CLIENT_DISPLAY_SLOT`がないdeviceには、slot 0のみを既存の6/7-byte
形式で送る。slot 1以降は送らない。Host Link protocol versionは2のままであり、
wireへsession IDやthread IDを追加しない。

## 操作とUI

SettingsのCodex連携欄でslot数と各slotのAuto/固定先を設定する。候補はCodexと
Claude Codeを種別で分けない共通列で表示する。bit 13対応deviceが検出されない
場合、UIは複数slotが現行Firmwareでは表示されないことを警告する。

`cycle_ai_session`の`HOST_ACTION.value`は対象slot番号とする。`value=0`は従来の
slot 0操作であり、空きsessionがなければ表示を変えない。これは既存uplink packet
のフィールド解釈を限定的に拡張するだけで、packet形式は変更しない。

## 検証境界

自動テストはslot payloadのバイト列、capability別送信、Auto/Pinnedの重複禁止、
slot縮退時の退役通知を対象とする。2026-08-09の実機では、2画面への個別表示、slot間の
状態分離、Auto/Pinned、固定先の終了時の`NONE`、slot別`cycle_ai_session`、slot数縮退を確認した。
複数キーボード接続時の同報は、対応keyboardを複数用意したときの追加確認項目とする。
