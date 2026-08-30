# ScreenKey押下によるAIターミナル前面化

## 1. 目的と成立範囲

ScreenKeyを押下したとき、そのScreenKeyが表示しているAIセッションの
専用Windows Terminalウィンドウを前面化する。

### 1.1 対象

- Keylink Studioから起動したCodex CLIセッション（Windows／WSL両方）
- Keylink Studioから起動したClaude Codeセッション
- 前面化のみ

### 1.2 対象外

approval／inputへの順序ベース回答（ScreenKey 2回目押下による代理response）は
本書の対象外である。設計案は
[ScreenKeyによるAIセッション前面化と順序ベース回答](screenkey-ai-interaction-design.md)
にあり、Broker代理responseのGate 1が未検証であるため、前面化とは切り離して扱う。

前面化を独立機能として先に成立させる理由は次の2点である。

- Gate 1が通らない場合に代理response方式を再設計する必要があり、前面化をそこに
  巻き込まない。
- 前面化だけでも「4画面から目的のターミナルへ即座に飛ぶ」という単独の価値がある。

将来Armed／代理回答を載せる場合も、本書の仕様は「押下 = 常に前面化、かつ待機中なら
追加でArmedへ遷移」と重ねられる形にしてある。

### 1.3 前提

先行コミット`cc55b75`により、Keylink Studioから起動するAIクライアントは
起動ごとに専用のWindows Terminalウィンドウで開く。各起動には一意な
`terminal_target_id`（`codex-<16 byte hex>` / `claude-<16 byte hex>`）が割り当てられ、
`AiDisplayTarget`もこのIDだけでセッションを指す。本書はこの状態を出発点とする。

## 2. 実測した事実

2026-08-30、Windows 11 Pro 26200 / Microsoft.WindowsTerminal 1.24.11911.0で実測した。
再検証コストが高いため数値ごと記録する。

| # | 事実 | 設計への影響 |
| --- | --- | --- |
| F1 | Windows Terminalの複数ウィンドウは**単一の`WindowsTerminal.exe`プロセス**に同居する（3ウィンドウ / PID 1個を確認） | PIDによるウィンドウ同定は不可能。HWND列挙かwt委譲の二択になる |
| F2 | `wt -w <name> focus-tab`は**バックグラウンドプロセスから実行しても対象ウィンドウを前面化できる**。所要時間は約1.1秒（`Start-Process -Wait`で1141 ms） | `SetForegroundWindow`の前面化権限規則を回避する手段になる |
| F3 | 対象ウィンドウが**最小化されている場合は復元されるだけで前面化されない**（foregroundは別アプリのまま） | wt委譲だけでは不十分。復元は自前で行う必要がある |
| F4 | **存在しない`-w`名を指定すると新しいターミナルウィンドウが生成される**（既定プロファイルの空ウィンドウを確認） | 「ウィンドウが無ければ何もしない」を満たすには委譲前の存在確認が必須 |
| F5 | ウィンドウタイトルはアクティブタブのタイトルであり、`--suppressApplicationTitle`により`display_name`（例`Claude Code · Keylink-Studio · BDAE7519`）で固定される | タイトル一致でHWNDを引ける |
| F6 | ウィンドウクラスは`CASCADIA_HOSTING_WINDOW_CLASS` | 列挙時の1次フィルタに使う |
| F7 | 既存`app_launch`はEnumWindows → `IsIconic`なら`SW_RESTORE` → `SetForegroundWindow`という手順を持つ（exe名一致） | 手順を流用できる |
| F8 | `actions::execute`は監視ループ内で同期実行される（`commands.rs:4878`） | 1.1秒のブロックはHIDポーリングとAI state同期を止める。非同期化が必須 |
| F9 | host action失敗時のフィードバックはログ1行のみ。キーボードやトーストへ返す経路は存在しない | 失敗通知の選択肢はログか無言に限られる |
| F10 | `ui/src/pages/Actions.tsx`の`ACTION_KINDS`とi18n `actions.kind.<kind>`への追記で設定UIに露出する | UI追加は定型作業で済む |
| F11 | `ManagedClientConnected`の本番送出は`codex_broker.rs:1258`の1箇所のみ。`ClientConnected`は`codex_activity.rs:1333`の内部再ディスパッチとテストにしか現れない | Codexセッションがregistryに載る時点で`terminal_target_id`は必ず埋まっている |
| F12 | Brokerは起動ごとのcapability tokenを持つ接続しか受理しない（未発行tokenは401） | Keylink Studio以外が起動したCodex CLIは接続できず、表示候補にならない |
| F13 | Claude側も`ClaudeLaunchIntegration`がlaunchごとに`terminal_target_id`を持ち、snapshotの`launch_id`は当該launch専用receiver由来である | 対応表の引き当ては必ず成功する |
| F14 | slotからターミナルへの解決は`AiDisplaySelection::slots()[slot].assigned` → `AiDisplayTarget::{Codex,Claude}{terminal_target_id}`で完結する | 相関のための追加実装は不要 |
| F15 | **素の`SetForegroundWindow`はバックグラウンドプロセスから成功する**。前面がBrave、最後のユーザー入力から234 ms、`ForegroundLockTimeout = 200000`（既定値、ロック有効）という最悪条件で、戻り値True かつ`GetForegroundWindow()`が実際に対象へ変化した。呼び出し元は前面プロセスの子孫でも、最後の入力の受信者でもない | 3.3の速い経路が実際に機能する。wt委譲はほぼ使われない保険となる。「戻り値Trueだが点滅するだけ」の既知の偽陽性も観測されなかった |

F15の測定にあたり、既存`show_window` actionでの間接確認は**代理テストにならない**ことが判明した。
Tauriの`set_focus()`は`tao`の`force_window_active()`を呼び、素の`SetForegroundWindow`が失敗すると
Altキー押下を`SendInput`で偽装して権限を奪う二段構えになっている
（`tao-0.35.3/src/platform_impl/windows/window.rs:1500`）。どちらの経路で成功したか区別できないため、
素の`SetForegroundWindow`だけを呼ぶ独立したプロセスで測定した。
本実装はこのAltキー偽装を使わない。

F11〜F13より、次はいずれも**到達不能**である。防御コードもログも置かない。

- `terminal_target_id`が空のセッション
- Keylink Studio以外が起動したセッション
- 本変更以前の旧形式ウィンドウ（`Codex: <project>`）

これらは表示候補にならないため、押下しても「slotが空で何も起きない」が既定の正しい動作になる。

## 3. 確定仕様

### 3.1 発火条件

新しいhost action `focus_ai_terminal`を追加する。

- `HostActionKind::FocusAiTerminal`
- `value` = ScreenKeyの物理index = `display_slot` index
- `path`不要（`needsPath()`はfalse）
- 既存制約をそのまま継承する。device単位の許可リスト制、既定disabled、監視中のみ実行

対象slotにセッションが割り当たっていれば、**AI activity stateを問わず**前面化する。
待機中（`WAITING_APPROVAL`／`WAITING_INPUT`）に限定しない。限定すると
Working中やIdleのセッションを表示しているScreenKeyが無反応になり、キーが死んでいる
時間が大半を占めるためである。

`cycle_ai_session`への相乗りは採用しない。cycleは表示を変え、前面化は表示を変えないため、
同一キーに載せるとユーザーがどちらが起きるか予測できない。
Firmware側での短押し／長押し分岐も、Firmwareに状態を持たせることになるため採用しない。

### 3.2 ウィンドウの同定

押下のたびにEnumWindowsで探索する。**HWNDをキャッシュしない。**

1. ウィンドウクラスが`CASCADIA_HOSTING_WINDOW_CLASS`であること
2. ウィンドウタイトルが期待サフィックスで終わること

期待サフィックスは`terminal_target_id`の**最後の`-`以降の先頭8桁**を大文字化した値である
（`state.rs`の`AiDisplayTarget::label()`および`commands.rs`の`terminal_display_name()`と
同じ導出）。

```text
codex-0123456789abcdef0123456789abcdef  ->  "01234567"
```

ウィンドウタイトルは`terminal_display_name()`が生成する`display_name`
（例`Codex · project · 01234567`）なので、必ずこの値で終わる。両者の導出が食い違うと
機能が無言で壊れるため、この結合をテストで固定する。

- **キャッシュしない理由**: 起動時にHWNDを保持すると、ウィンドウを閉じた後にOSが同じ
  HWND値を別ウィンドウへ再利用したとき、**誤ったウィンドウを前面化する**。都度探索は
  stale化しようがなく、F4が要求する存在確認をそのまま兼ねる。
- **完全一致ではなくサフィックス一致とする理由**: プロジェクト名の表記に依存しなくなり、
  押下時に`display_name`を引く登録簿参照が不要になる。サフィックスは起動ごとの
  ランダム32 bitなので衝突しない。
- **複数ヒット時は何もしない**: 本来起こり得ない異常状態であり、先頭を選ぶと誤ったウィンドウを
  前面化し得る。安全側に倒す。

既知の制約として、ユーザーが当該ウィンドウに別タブを追加してアクティブタブを切り替えると、
ウィンドウタイトルが変わり探索が空振りする。空振りは「何もしない」という安全側の失敗であり、
元のタブへ戻せば復帰する。

### 3.3 実行順序

F8より監視ループを塞げないため、**別スレッドへ投げて即座に戻る**。
スレッド内では速い経路から順に試す。

1. EnumWindowsでHWNDを探索する（数ミリ秒）。**見つからなければ即終了**する。
   新しいウィンドウを開かない（F4回避）。
2. `IsIconic`が真なら`SW_RESTORE`で復元する。最大化ウィンドウに`SW_RESTORE`を
   無条件で撃たない（最大化が解除されるため）。
3. `SetForegroundWindow`を実行し、**戻り値**で成否を判定する。
4. 失敗した場合のみ`wt -w <terminal_target_id> focus-tab`へフォールバックする。

`SetForegroundWindow`を先に置くのは、成功する場合の遅延をほぼ0にするためである。
F2の1.1秒は前面化権限が拒否されたときだけのコストになる。
手順2で自前復元するため、F3（最小化時にwtがactivateしない）も回避できる。

### 3.4 多重実行の抑止

`AtomicBool`で同時1件に制限し、実行中の押下は弾く。既存の`refresh_ai_usage`が
`ai_usage_refreshing`で採っているパターンと同じにし、新しい同期プリミティブを持ち込まない。

前面化処理は冪等なので取りこぼしても害はなく、ユーザーから見れば
「連打しても1回分だけ効く」という挙動になる。

### 3.5 失敗時の扱い

**ログを出さない。** 次はいずれも無言で終了する。

- `value`が`display_slot`の範囲外
- 対象slotにセッションが割り当たっていない
- 対象ウィンドウが見つからない（閉じた直後、タブ名変更）
- 複数ヒット
- `SetForegroundWindow`とwt委譲の両方が失敗

前面化の失敗は「画面が変わらない」という形でユーザーに即座に見えるため、追加の通知は冗長である。
ScreenKeyへ通知を返す案は、Host Linkへdownlinkを追加することになるため採用しない。

実装上は、これらのケースで`Err`ではなく`Ok(ActionOutcome::Continue)`を返す。
`Err`を返すと`commands.rs`の共通ディスパッチが`host action N failed: ...`を
error levelで記録してしまうためである。

なお3.4の多重実行抑止だけは`refresh_ai_usage`と同じく`Err`を返すため、
連打時に`host action N failed: focus_in_progress`がログに残る。

### 3.6 影響範囲

- Host Link wire formatの変更: なし
- Firmwareの変更: なし
- capability bitの追加: なし
- Host Link protocol versionの変更: なし

Firmware keymapでは、左端のScreenKeyから順に`&host_action <ID> 0`、`1`、`2`、`3`を割り当てる。

## 4. 進捗状況

### 4.1 実装項目

| # | 項目 | 状態 | 実装箇所 |
| --- | --- | --- | --- |
| 1 | `HostActionKind::FocusAiTerminal`をconfigへ追加 | 完了 | `config.rs` |
| 2 | 期待サフィックス導出とタイトル照合を純粋関数として実装 | 完了 | `ai_terminal_focus.rs` `expected_suffix` / `title_matches_suffix` |
| 3 | HWND探索（EnumWindows + visible + class + サフィックス、複数ヒット検出） | 完了 | `ai_terminal_focus.rs` `windows_impl::enum_proc` |
| 4 | 前面化シーケンス（復元 → SetForegroundWindow → wt委譲） | 完了 | `ai_terminal_focus.rs` `windows_impl::focus` |
| 5 | 別スレッド実行と`AtomicBool`による多重実行抑止 | 完了 | `ai_terminal_focus.rs` `spawn_focus` / `FocusGuard` |
| 6 | `actions::execute`への結線（slot解決、無言failure） | 完了 | `actions.rs` |
| 7 | 設定UI（`ACTION_KINDS`、i18n 和英） | 完了 | `Actions.tsx` / `types.ts` / `i18n.tsx` |
| 8 | 純粋関数のユニットテスト | 完了 | `ai_terminal_focus.rs` tests 11件 |
| 9 | 実機確認（4.2） | 完了 | 全9項目合格 |

純粋関数として切り出すのは、Win32 API呼び出しがユニットテストできないためである。
テストは正常系、空文字`terminal_target_id`、`-`なし、末尾空、大文字化、旧形式タイトル不一致、
大文字小文字、空サフィックスに加え、`commands.rs`の`terminal_display_name()`が生成した
タイトルが`expected_suffix()`の出力で終わることをCodex／Claude Code／非ASCIIプロジェクト名で
固定している。この結合が壊れると機能だけが無言で停止するため、テストで直接押さえる。

自動テストは`cargo test -p rawhid-host-core -p rawhid-host-tauri`で263 + 50件が通過し、
`cargo fmt --check`も通過している（2026-08-30時点）。

#### 4.1.1 レビューで修正した点

| # | 指摘 | 対応 |
| --- | --- | --- |
| 1 | `enum_proc`に可視性フィルタがなく、非表示ウィンドウがサフィックス一致すると誤って「複数ヒット」となり、正しいウィンドウがあるのに無言で何もしない | `IsWindowVisible`で除外。最小化ウィンドウは`WS_VISIBLE`を保持するため3.3手順2に影響しない |
| 2 | `AtomicBool`をslot解決の前に確保しており、早期returnごとに手動`store(false)`が必要で重複していた。リセット漏れが起きると以降の押下が永久に`focus_in_progress`で弾かれる | 確保をspawn直前へ移動。手動`store(false)`を全廃し、解放を`FocusGuard`のDropへ一本化 |
| 3 | `terminal_display_name()`との結合を固定するテストがなく、書式変更でコンパイルもテストも通ったまま機能が壊れる | `terminal_display_name`を`pub(crate)`にして結合テストを3件追加 |

#### 4.1.2 受容した残存リスク

`SetForegroundWindow`が失敗してwt委譲へ進む間にウィンドウが閉じられると、F4により新しい
ターミナルが開く。ただし窓は数ミリ秒であり、そもそもウィンドウが1件見つかった直後にしか
到達しない経路である。回避コードの複雑さに見合わないため受容する。

### 4.2 実機確認項目

| # | 項目 | 結果 |
| --- | --- | --- |
2026-08-30に全項目を実機で確認し、合格した。

| # | 項目 | 結果 |
| --- | --- | --- |
| 1 | 待機中・作業中いずれのslotを押しても正しいウィンドウが前面化する | **合格** |
| 2 | 最小化されたウィンドウが復元されて前面化する | **合格** |
| 3 | 最大化されたウィンドウが最大化のまま前面化する | **合格** |
| 4 | ウィンドウを閉じた後の押下で新しいターミナルが開かない（F4の回帰確認） | **合格** |
| 5 | 同じプロジェクトから4セッション起動しても取り違えない | **合格** |
| 6 | 連打しても1回分だけ効く（連打後の単発押下が効くこと＝`FocusGuard`の解放確認を含む） | **合格** |
| 7 | WSL環境で起動したCodexも前面化できる | **合格** |
| 8 | Claude Codeのウィンドウも前面化できる | **合格** |
| 9 | `cycle_ai_session`とAI state表示が回帰しない | **合格** |

以上により、既存文書[ScreenKeyによるAIセッション前面化と順序ベース回答](screenkey-ai-interaction-design.md)の
**Gate 0（Terminal所有権）は通過**した。同文書§13「実装前に確定する事項」のうち
「Windows Terminalの専用window作成と安定したwindow target取得方法」も解決済みである。
残るGate 1（Codex代理response）以降は本書の対象外である。

## 5. 更新履歴

| 日付 | 変更点 |
| --- | --- |
| 2026-08-30 | 初版。F1〜F14の実測と3章の仕様を確定。実装は未着手 |
| 2026-08-30 | 3.2の期待サフィックス導出の記述誤りを訂正（「末尾8桁」→「最後の`-`以降の先頭8桁」）。実装完了によりPC側の実装項目1〜8を完了へ更新。レビュー指摘3件と受容した残存リスクを追記。実機確認（4.2）は未実施 |
| 2026-08-30 | F15を追加。素の`SetForegroundWindow`がバックグラウンドプロセスから成功することを実測し、3.3の経路順序の妥当性を確認。`show_window`による間接確認が代理テストにならない理由も記録 |
| 2026-08-30 | 実機確認9項目すべて合格。実装・検証とも完了。[ScreenKeyによるAIセッション前面化と順序ベース回答](screenkey-ai-interaction-design.md)のGate 0通過 |
