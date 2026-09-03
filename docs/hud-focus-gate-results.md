# HUD フォーカス Gate (KO-1) 結果

- 状態: 完了
- 実施日: 2026-09-03
- 最終判定: **KO-1 成立**（C7 は不成立だが設計上の非ブロッカー）
- 対象: Keylink Studio Host（Tauri `2.11.2` / WebView2）
- 実行環境: Windows 11 Pro `10.0.26200.9278`、DPI 150% および 200%、シングル／マルチモニタ
- 検証コード: `crates/rawhid-host-tauri/src/hud_window.rs`（本実装へ残す）、
  `crates/rawhid-host-tauri/src/hud_probe.rs`（検証ハーネス）、`ui/public/hud-probe.html`
- 起動方法: `cargo run -p rawhid-host-tauri -- --hud-focus-probe`（要 Vite dev server）

---

## 1. 背景

ScreenKey搭載キーボードからAI（Codex / Claude Code）の承認待ち・入力待ちへ回答する機能を
検討する過程で、ScreenKey（0.85インチ / 128×128 / ST7735）には要求の全文もdiffも
物理的に載らないことが判明した。視距離60cmで必要な文字高は約29px、載る量はASCII 30字程度である。

そこで役割を分割する設計に至った。

- **ScreenKey** = 気づく／どのセッションか（色・アイコン・数文字）
- **HUD**（モニタ上の小さな常時最前面パネル）= 読む（要求全文と選択肢）
- **物理キー・エンコーダ** = 決める

この設計の価値は「**承認のたびに作業画面を奪われない**」という一点に集約される。既存案が
必ずターミナルを前面化していたのに対し、HUD方式はフォーカスを一切動かさずに決裁できる。

したがって **HUDが表示・更新・非表示のいずれでもフォアグラウンドを動かさないこと** が
成立の絶対条件であり、これをノックアウト要因 KO-1 として最優先で検証した。

同じウィンドウ層は `ai-response-transfer-design.md` §12 の転送プレビューウィンドウ
（別 WebviewWindow / 常に最前面 / フォーカスを奪わない）とも共通であり、本Gateの結果は
両機能の成否を同時に決める。

---

## 2. 判定

| # | 条件 | 結果 | 根拠 |
|---|---|---|---|
| C1 | 表示・反復・更新でフォアグラウンドが変化しない | **PASS** | P1/P2/P3 が2つの独立した計測器でイベント0件 |
| C2 | 前後で前面ウィンドウが同一 | **PASS** | `foreground_hwnd` / `pid` / `title` 不変 |
| C3 | 下のアプリのフォーカス所有者が不変 | **PASS** | `gui_focus_hwnd` / `gui_caret_hwnd` 不変 |
| C4 | 打鍵が1文字も落ちない | **PASS** | 202文字送出 → キャレットが1664px前進（8.24px/文字） |
| C5 | Alt+Tab とタスクバーに出ない | **PASS** | 目視 |
| C6 | 最大化ウィンドウより前面に出る | **PASS** | Windows Terminal（Win32）、VS Code（Electron）の両方 |
| C7 | クリックでフォーカスが移らない | **FAIL** | 20ms未満の瞬間的な前面化。3ラン再現 |
| C8 | 計測器が生存していたことの証明 | **PASS** | P1〜P3 の両端で両計測器が発火、静定も確認 |

**結論: HUDを表示しても、50回点滅させても、内容を書き換えても、作業中のウィンドウから
フォーカスは一度も移らない。最大化ウィンドウの上に描画され、Alt+Tabにもタスクバーにも現れない。**

C7 の扱いは §5 を参照。

---

## 3. 検証方法

### 3.1 ウィンドウの作り方（`hud_window.rs`）

`WebviewWindowBuilder` を `visible(false)` / `focused(false)` / `always_on_top(true)` /
`skip_taskbar(true)` / `decorations(false)` / `resizable(false)` で生成し、
**生成直後・初回表示前に** 生Win32で ExStyle へ `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` を追加する。

表示・非表示はTauriの `show()` / `hide()` を使わず、生Win32で行う。

```text
表示: SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW)
非表示: ShowWindow(hwnd, SW_HIDE)
```

WebView2の初期化は起動時に1回だけ行い、以後は表示／非表示のみとする。初回のフォーカス奪取
リスクを起動直後という無害な瞬間に閉じ込めるためである。

### 3.2 計測器

**2つの独立した計測器を使い、両方が沈黙したときだけPASSとする。** 片方だけでは不十分である
（理由は §6.2）。

| 計測器 | 方式 | 失敗の方向 |
|---|---|---|
| `ForegroundProbe` | `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT)` | **過少報告**（配信を落とす） |
| `ForegroundSampler` | 20ms間隔で `GetForegroundWindow()` をポーリング | 20ms未満の瞬断を見逃す |

補助として `GetGUIThreadInfo` によるフォーカス／キャレットのスナップショット比較と、
`SendInput` で既知文字列を送出して着弾先を検証する `Typist` を用いる。

**`WINEVENT_SKIPOWNPROCESS` は付けてはならない。** 本検証で捕まえたい失敗は
「HUD（＝自プロセスの窓）が前面を奪った」ことそのものであり、このフラグを残すと
最も検出したい事象だけが握りつぶされ、確実に偽のPASSが出る。既存の
`crates/rawhid-host-tauri/src/foreground.rs` はレイヤー切替用途のためこのフラグを付けており、
そのまま流用してはならない。

### 3.3 フェーズ

```text
P0   起動（ウィンドウ生成 → WebView2初期化待ち2秒 → baseline）
P0b  計測器事前確認（最大3回リトライ、末尾で静定待ち）
P1   初回表示                  ┐
P2   反復（50回 × show/hide、typist稼働）  ├ 採点対象（KO-1 core）
P3   内容更新（eval × 20回）    ┘
P3b  計測器事後確認（最大3回リトライ、末尾で静定待ち）
P4   クリック（採点対象、C7）
P4b  計測器最終確認（参考、判定に不使用）
```

### 3.4 判定を汚染させないための3つの規律

本Gateで最も重要なのは、**測っていないものを合格と書かない**ことである。次の3つを実装した。

1. **VOIDルール**: 計測器の生存が P0b と P3b の両方で確認できなければ、
   P1〜P3 がどれだけ綺麗でも `VOID` とし `PASS` を出さない。
   フックが死んでいるだけの「0件」を合格と誤読しないため。
2. **クリック検出**: P4のクリックを `GetAsyncKeyState` + `GetCursorPos` で実際に検出する。
   検出できなければ `PASS` ではなく `C7 UNVERIFIED` とする。
   人がクリックし忘れただけの「0件」を合格と誤読しないため。
3. **判定の分離**: `KO-1 core (P1-P3)` と `C7 click (P4)` を別々に報告する。
   前者は設計の成立条件、後者は副次条件であり、1つのFAILに丸めると設計判断ができなくなる。

---

## 4. 確定ランの結果

`hud-focus-probe-20260903-151512.log`

```text
Phase                    | hook | sampler | activation unchanged | verdict
P0b-instrument-precheck  | 2    | 2       | true                | PASS (both instruments confirmed alive)
P1-first-show            | 0    | 0       | true                | PASS
P2-repeat                | 0    | 0       | true                | PASS
P3-content-update        | 0    | 0       | true                | PASS
P3b-instrument-postcheck | 2    | 2       | true                | PASS (both instruments confirmed alive)
P4-click                 | 1    | 0       | true                | FAIL
P4b-instrument-final     | 0    | 1       | false               | INFO (not scored)

Instrument health : OK (hook: fired at P0b attempt 1 and P3b attempt 1;
                        sampler: fired at P0b attempt 1 and P3b attempt 1)
KO-1 core (P1-P3) : PASS
C7 click   (P4)   : FAIL (hook may be degraded after the click — see P4b)

Overall judgement: PASS (KO-1 core); C7 FAILED
```

- 静定待ちは P0b / P3b とも `settled after 530ms`
- Typist 202文字、キャレット x が 48 → 1712（1664px、8.24px/文字）、y不変
- HUD webview URL は `http://localhost:5173/hud-probe.html`

### 目視確認（プローブでは測れない項目）

| 確認 | 環境 | 結果 |
|---|---|---|
| 最大化 Windows Terminal より前面 | DPI 150% | OK |
| 最大化 VS Code（Electron）より前面 | DPI 150% | OK |
| タスクバーに出ない | DPI 150% | OK |
| Alt+Tab に出ない | DPI 150% | OK |
| 位置が正しい | DPI 200% | OK |
| 文字が読める | DPI 200% | OK |
| マルチモニタ（エディタを別ディスプレイ） | DPI 150% | OK（`144604` ラン） |

---

## 5. C7（クリック時の前面化）の詳細

```text
P4-click  hook=1  t=33292ms  hwnd=HUD  pid=probe
          sampler=0
          before/after のスナップショットは完全に同一
```

sampler（20msポーリング）が見逃し、hookだけが捉えた。つまり前面化は **20ms未満の瞬間的なもの**
であり、直後に元のウィンドウへ自力で戻っている。`141948` / `144604` / `151512` の3ランで再現した。

`WS_EX_NOACTIVATE` はウィンドウマネージャ層のクリック活性化を抑止するが、WebView2の子ウィンドウが
クリック時に内部で `SetFocus` 相当を呼ぶため、瞬間的な活性化が発生していると考えられる。

**設計上の非ブロッカーと判断する。**

- 本設計でHUDは表示専用であり、入力はすべてHID経由でStudioプロセスへ届く。
  ユーザーがHUDをクリックする操作は設計上存在しない
- 仮にクリックされても20ms以内に元のウィンドウへフォーカスが戻る
- 完全に排除する必要が生じた場合の定石は、`WM_MOUSEACTIVATE` を処理して `MA_NOACTIVATE` を返すこと

---

## 6. 計測器側で潰した欠陥

**確定判定に至るまでに7ランを要した。7ランすべてが「HUDの失敗」ではなく「計測の失敗」であり、
プローブが自ら測定不成立を申告して止まった結果である。** 以下は本Gate固有ではなく、
今後HUDや転送プレビューを触るたびに再発見しうる知見である。

### 6.1 `WINEVENT_OUTOFCONTEXT` フックは単独で信用してはいけない

`032802` ランで、フックが**確実に起きた前面遷移を2件取りこぼした**。
before/afterスナップショットが遷移の発生を証明しているにもかかわらず、イベントが届かなかった。

このためポーリング型の第2計測器 `ForegroundSampler` を追加した。両者は**逆方向に失敗する**。

- フックは**過少報告**する（配信を落とす）が、**過剰報告はしない**（起きていない遷移を作らない）
- サンプラーは20ms未満の瞬断を見逃すが、継続する遷移は原理的に取りこぼさない

したがって **片方だけを判定に使ってはならない。両方がゼロのときだけPASSとする。**
この規律の正しさは C7 で実証された（サンプラー単独判定なら、クリック時の瞬断はPASSに化けていた）。

### 6.2 HUDをクリックするとフックが停止する

`141948` と `144604` の2ランで再現した。**どちらもフックの最後の発火はP4のクリックであり、
それ以降フックは二度と発火しない。** ランダムな取りこぼしではなく系統的な現象である。

この結果、当初 `P0b → P1 P2 P3 → P4 → P5(事後確認)` としていたフェーズ順序では、
事後確認がクリックの向こう側にあるため**構造的に必ず失敗**していた。

**生存確認は「ゼロを信じたいフェーズ」を前後から挟み込んでいなければ意味がない。**
順序を `P0b → P1 P2 P3 → P3b(事後確認) → P4(クリック)` に変更して解決した。
P4自体は陽性検出（hook=1）なので独立した生存証明を必要としない。陽性は計測器が
動いていたことの証拠そのものだからである。

### 6.3 フェーズ境界からのイベント漏れ

生存確認は意図的に前面を2回動かす（HUDへ → 元のウィンドウへ戻す）。その**後始末の遷移が
次のフェーズに帰属してしまう**現象が `150049` ランで発生し、`KO-1 core: FAIL` を誤って出した。

サンプラーは20msポーリングのため、フックより18〜19ms遅れて同じ遷移を検出する。その差で
フェーズ境界を跨いでいた。

| | HUDへ | 元へ戻る | 帰属 |
|---|---|---|---|
| P0b hook | t=2141ms | **t=3144ms** | 両方 P0b |
| P1 sampler | — | **t=3162ms**（18ms差） | **P1へ漏れた** |
| P3b hook | t=28641ms | **t=29645ms** | 両方 P3b |
| P4 sampler | — | **t=29664ms**（19ms差） | **P4へ漏れた** |

1つの原因が3つの症状（P1のsampler=1、P4のsampler=1、P4の`activation unchanged: false`）を
出していた。生存確認フェーズの末尾（フェーズの時間窓の**内側**）に `wait_for_quiescence` を
置いて解決し、3症状が同時に消えた。

静定条件は「両計測器のイベント数が500ms増えない」「前面が基準ウィンドウに戻っている」
「`gui_caret_hwnd != 0`（キャレット再作成済み）」の3つである。

### 6.4 `tauri::is_dev()` はビルドプロファイルでは変わらない

```rust
pub const fn is_dev() -> bool { !cfg!(feature = "custom-protocol") }
```

`custom-protocol` は `cargo tauri build` が付与する **`tauri` crateのコンパイル時feature** であり、
`cargo run --release` では有効にならない。したがって **`--release` を付けてもdevモードのまま**
`devUrl` を見にいく。

また `tauri.conf.json` の `build.devUrl` は静的な設定値であり、**ビルドプロファイルとは無関係に
常に `Some`** である。dev判定に使ってはならない。

`--hud-focus-probe` の実行には **Vite dev server の起動が必須**である。

### 6.5 `WebviewWindow::url()` は読み込み内容の証拠にならない

`141948` ランで `about:blank` を返したが、ページは正常に描画されていた（P4の日本語
カウントダウンが表示・更新されていた）。この値は参考情報として扱い、ページ読み込みの
成否判定に使ってはならない。

代替として、devモード時に `GET <dev_url>/hud-probe.html` の本文へ `id="hud-probe-text"` が
含まれることを確認している。

### 6.6 releaseビルドにはコンソールがない

`main.rs` の `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` により、
releaseビルドは `println!` の出力先を持たない。`135214` ランでは、プローブが正常に動作して
VOIDログを書いていたにもかかわらず、ユーザーからは「何も起きない」ように見えた。

終了時に `MessageBoxW` で総合判定とログの絶対パスを表示するようにした。
表示は**全フェーズ終了後**に限る（計測中に出すとフォアグラウンドを奪って測定を汚す）。

### 6.7 一発測定ツールは自分で終了しなければならない

プローブが `app.exit(0)` を呼んでいなかったため、測定後もプロセスが常駐し続けた。
2つのウィンドウは隠れており `skip_taskbar` のためタスクバーにも現れず、**完全に不可視のまま
自分の .exe をロックし続け、後続のビルドを3回失敗させた**。

### 6.8 スナップショット比較にキャレット矩形を含めてはいけない

P2はプローブ自身がタイプする唯一のフェーズであり、キャレットが動くのは必然である。
`rcCaret` を不変性の判定に含めていたため、初回ランで誤ってFAILを出した。

判定に使うのは `foreground_hwnd` / `pid` / `title` / `gui_active_hwnd` / `gui_focus_hwnd` /
`gui_caret_hwnd` の6項目とし、`rcCaret` は**判定ではなく証拠**として別途出力する
（「x移動量 ÷ 送出文字数 = px/文字」が妥当なら、打鍵が期待した宛先へ届いた裏付けになる）。

---

## 7. 本実装へ引き継ぐ事項

1. **`hud_window.rs` はそのまま本実装のウィンドウ層として使える。**
   `show_at` / `hide` でTauriの `show()` を使わない理由、ExStyleを生成直後に適用する理由は
   コメントとして残してある。後から「普通に `show()` すればいい」と戻さないこと
2. **HUDサイズはDPIを考慮した論理単位で指定すること。** 現在の 420×260 は物理ピクセル指定で、
   高DPI環境では表示面積が実質的に縮む。DPI 200%でも読めることは確認済みだが、本実装では対応が要る
3. **本番のHUDページはViteにバンドルされる通常のルートとして作ること。**
   `ui/public/hud-probe.html` が素のHTMLなのはプローブが使い捨てだからである。バンドルされた
   ルートなら `@tauri-apps/api` をimportでき、`withGlobalTauri`（本番の `main` ウィンドウにも
   `window.__TAURI__` を露出させる設定）を有効にする必要がない
4. **本実装時にフル構成で1回再測定すること。** 本プローブは最小の `tauri::Builder` で動作しており、
   トレイ・多数のコマンド・監視スレッドを持つ本番構成ではない
5. **モニタ選択は未検討。** 現在の実装はプライマリモニタの右下固定である

---

## 8. 非対象

- 転送プレビュー用の「自由入力時だけフォーカスを取り、入力確定で返す」挙動
  （`ai-response-transfer-design.md` §12。転送機能の実装時に別途検証する）
- HUDを別モニタへ出す場合の挙動
- 排他的フルスクリーンアプリ（ゲーム等）との共存
- HUDの見た目・レイアウト
- ScreenKey側の表示、Host Link packetの変更
- Codex / Claude Code への回答経路（KO-2 / KO-3）

---

## 9. 次のノックアウト要因

| # | 内容 | 落ちた場合 |
|---|---|---|
| KO-3 | Claude Code の `PermissionRequest` hook をブロックして決定を返せるか | Claude Codeのみ「HUDで内容は見える／回答はターミナル」へ縮退。設計全体は生きる |
| KO-2 | Codexへ代理responseを送ったときTUIのプロンプトが正しく閉じるか | Brokerが要求を保持してCLIへ転送しない方式へ切り替え |
