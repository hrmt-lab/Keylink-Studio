# ScreenKey と HUD による AI 承認・回答 設計

- 状態: 段階1は実装・実機確認済み。段階2は`race`実機検証を反映して設計確定、実装未着手
- 作成日: 2026-09-03
- 対象: Keylink Studio Host、Codex Broker、Claude Code Observer、Tauri UI、Firmware 描画
- 対象ハードウェア: ScreenKey 4個（0.85インチ / 128×128 / ST7735）、通常キー 7個、エンコーダ 1個
- 基準環境: `codex-cli 0.153.2`、Claude Code `2.1.259`、Windows 11 Pro `10.0.26200.9278`

## 本書が置き換える文書

| 文書 | 扱い |
|---|---|
| `screenkey-ai-interaction-design.md` | **本書が置き換える**（in-band方式・ScreenKeyへ選択肢を位置対応させる案） |
| `screenkey-ai-prompt-response-design.md` | **本書が置き換える**（キー入力送出方式・同上） |

両文書は「ScreenKey の位置＝選択肢の番号」「回答前にターミナルを前面化する」という点で一致し、
回答経路の結論だけが逆だった。**本書はその共通前提のほうを破棄する。** 理由は §2。
両文書が行った Codex / Claude Code のプロトコル調査は、本書で実測に置き換えられている。

## 関連文書

| 文書 | 関係 |
|---|---|
| `hud-focus-gate-results.md` | KO-1。HUD がフォーカスを奪わないことの実測 |
| `claude-permission-hook-gate-results.md` | KO-3。Claude Code へ回答を注入できることの実測 |
| `codex-approval-proxy-gate-results.md` | KO-2。Codex へ代理応答できることの実測 |
| `ai-response-transfer-design.md` | 別主題（AI間の回答転送）。**HUD と同じウィンドウ層を共有する** |
| `ai-session-display-switching.md` | 実装済み。表示候補の選択規則 |
| `ai-display-slot-multiscreen-host-design.md` | 実装済み。論理 `display_slot` |
| `screenkey-terminal-focus-design.md` | 実装済み。`FocusAiTerminal` |
| `packet-spec.md` | Host Link v2 wire contract |

---

## 1. 目的

同時に最大4セッションを表示するキーボードで、あるセッションが承認待ち／入力待ちになったとき、
**作業中の画面を一度も奪わずに**、手元の操作だけで回答を完了できるようにする。

---

## 2. 設計の起点：ScreenKey は「読む面」になれない

ScreenKey は 0.85インチ対角・128×128 である。一辺 15.3mm、画素ピッチ 0.119mm。

可読性の目安（視角20分）で必要な文字高は `視距離 × 0.0058`。

| 視距離 | 必要な文字高 | ピクセル | 入る行数 |
|---|---|---|---|
| 50cm（前傾） | 2.9mm | 24px | 5行 |
| **60cm（通常の着座）** | **3.5mm** | **29px** | **4行** |

実際に載る量は **和文16字**または **ASCII 30字**程度である。キートップは斜めから見るため、
実効的にはさらに減る。

```text
$ git push origin feat/ai-response-transfer-keys   ← 45字。入らない
```

diff は何インチあっても載らない。**したがって「ScreenKey に選択肢や要求内容を出し、それを読んで
番号で答える」という置き換え対象2文書の共通前提は成立しない。**

置き換え対象文書はこの制約を「文字列を wire へ載せない」という設計方針として持っていたが、
それは状態表示機能から引き継いだ制約であって、物理的制約とは別物である。**物理的制約のほうが
厳しく、そちらが設計を決める。**

---

## 3. 設計原則：役割を3つに分ける

| 軸 | 担当 | 理由 |
|---|---|---|
| **気づく／どれか** | ScreenKey ×4 | 色・アイコンなら視距離が効かない。4画面あるので並行して待機中のセッションが一目で分かる |
| **読む** | **HUD**（モニタ上の小さな常時最前面パネル） | 解像度が要る。既に見ている面である |
| **決める** | 物理キー・エンコーダ | **現在のフォーカスを一切動かさずに確定できる唯一の入力装置** |

### 3.1 なぜ HUD がターミナル前面化より優れるか

承認1回のコストは打鍵時間ではない。**対象の画面へ移動すること**と**元の作業へ戻ること**である。
置き換え対象2文書はどちらも必ずターミナルを前面化するため、このコストを毎回払わせる。

HUD は非アクティブ・最前面ウィンドウとして表示するため、

- **フォーカスを取らない。** エディタのキャレットは残り、打鍵も止まらない
- **ウィンドウ配置を壊さない。** だから「元に戻る操作」が発生しない
- 常に同じ場所に出る。探す必要がない
- Codex にも Claude Code にも同じ UI を出せる

これは KO-1 で実測済みである（§4）。

### 3.2 なぜ決定を物理キーで行うか

HUD が出ているならマウスでクリックすればよい、とはならない。**クリックはフォーカス移動を伴う**ため、
その瞬間に「戻る操作」が復活する。物理キーは、現在のフォーカスを1ミリも動かさずに決裁できる
唯一の手段である。

### 3.3 選択肢の序数一致問題が消える

置き換え対象2文書が共通で抱えていた「Host が思う選択肢の順番と、ターミナル画面の順番が一致するか」
という問題は、**選択肢を HUD に出した瞬間に消滅する**。ユーザーが読むリストと Host が返すリストが
同一のデータになるためである。ターミナルの表示順は無関係になる。

---

## 4. 実測で確定した前提（3つのゲート）

| # | 問い | 結果 |
|---|---|---|
| KO-1 | HUD は表示・反復・更新でフォーカスを奪わないか | ✅ 2つの独立した計測器でイベント0件。最大化ウィンドウより前面、Alt+Tab・タスクバーに出ない、DPI 150%/200%、マルチモニタで確認 |
| KO-3 | Claude Code へ回答を注入できるか | ✅ `PermissionRequest` hook の decision で承認・拒否が通る。拒否理由がモデルに届く |
| KO-2 | Codex へ代理応答できるか | ✅ **保持・同時転送（`race`）とも成立**。`race`ではHUD/CLIの先着回答だけで要求が解決し、遅着回答で再実行もJSON-RPCエラーも起きない |

### 4.1 どちらのクライアントもフォールバックが自動的に存在する

- **Claude Code**: hook の応答を待たずに約3秒でターミナルにもプロンプトが出る。
  Studio が落ちていれば、いつもどおりターミナルで答えられる。固まらない
- **Codex**: 要求を最初から CLI へ転送する。HUDとTUIのどちらでも直ちに回答でき、片方が
  使えなくてももう片方を待つ必要がない

**HUD は「あれば速い」便利層であり、無くても何も壊れない。** 当初の懸念（Studio 停止時に
AI クライアントが固まる）は実測で否定された。

---

## 5. 体験の全体像

```text
[平常] 4画面がセッション状態を表示。作業に集中している
   │
   │ Codex が承認待ちになる
   ▼
[気づく] 対象の ScreenKey が黄色く点滅し、要求の種別アイコンを出す
   │      同時にモニタ隅へ HUD が現れる（フォーカスは動かない）
   │
   │ 視線を HUD へ。全文と選択肢を読む
   ▼
[決める] ✅ を押す ／ エンコーダを回して選び押す
   ▼
[復帰] HUD が消える。ScreenKey が状態表示へ戻る
       作業画面は最初から一度も動いていない
```

複数が同時に待機しているときは、答えたいセッションの **ScreenKey を押す**と HUD の内容が
それに切り替わる。4画面の色とアイコンで、読まずに識別できる。

---

## 6. ハードウェア割当

### 6.1 ScreenKey ×4

意味は **「このセッションを相手にする」** で一貫させる。

| 状況 | 短押し | 長押し |
|---|---|---|
| 待機中（`WAITING_APPROVAL` / `WAITING_INPUT`） | **HUD の対象をこのセッションにする** | 対応ターミナルを前面化 |
| それ以外 | 対応ターミナルを前面化（**現行 `FocusAiTerminal` の挙動を維持**） | — |

**回答の意味には使わない。** 位置で意味が変わるとモード錯誤が起き、
「許可のつもりで拒否」という最悪の誤操作を生む。また4画面が状態表示を失うことは、
最も忙しい瞬間にこの製品の中心価値を手放すことになる。

### 6.2 通常キー 7個

| キー | 単押し | 長押し / Fn |
|---|---|---|
| **✅ 承認**（緑キーキャップ） | 現在の選択を決定 | `Fn` ＋押下 = 「常に許可」（§9.3、Codex のみ） |
| **❌ 拒否**（赤キーキャップ） | 拒否 | 長押し = 中断（Turn ごと止める） |
| **⤢ 開く** | 対象ターミナルを前面化 | — |
| **☑ トグル** | 複数選択のトグル | — |
| **Fn** | 修飾 | — |
| **TRANSFER** | `ai-response-transfer-design.md` の転送モード | — |
| 予備 | ユーザー自由 | — |

物理キーキャップの色と刻印で意味が固定されるため、学習が要らない。

### 6.3 エンコーダ

一貫した意味は **「中身を進める」**。

| 状況 | 回す | 押す |
|---|---|---|
| 選択肢が出ている | HUD のハイライトを進める | 決定（✅ と同じ） |
| 平常時 | 音量など普段の用途 | — |
| `Fn` ＋ 回す | セッション巡回（既存 `cycle_ai_session`） | — |

**回すだけでは何も確定しない。** 可逆な操作にだけ回転を割り当てる。
5個以上の選択肢でもページ送りは不要である。

### 6.4 レイヤ切替

capability bit 0 `APP_LAYER`（Host→Firmware の layer set/clear）が既にある。
**HUD 表示中だけ AI 回答レイヤをセットし、解除時にクリアする。**

これにより、非表示時はエンコーダも通常キーも普段の用途のまま使える。
7つの通常キーとエンコーダを AI 専用に固定的に潰さずに済む。

---

## 7. HUD の仕様

### 7.1 ウィンドウ

Tauri の別 `WebviewWindow` として実装する。`crates/rawhid-host-tauri/src/hud_window.rs`
（KO-1 で作成済み）をそのまま使う。

- `visible(false)` / `focused(false)` / `always_on_top(true)` / `skip_taskbar(true)` /
  `decorations(false)` / `resizable(false)` で生成
- 生成直後・初回表示前に ExStyle へ `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`
- 表示は `SetWindowPos(HWND_TOPMOST, ..., SWP_NOACTIVATE | SWP_SHOWWINDOW)`、
  非表示は `ShowWindow(SW_HIDE)`。**Tauri の `show()` / `hide()` は使わない**
- WebView2 の生成は起動時に1回だけ

**HUD は入力を一切受けない。** 承認キーもエンコーダも HID 経由で Studio プロセスへ直接届くため、
HUD にフォーカスを渡す理由が構造的に存在しない。

### 7.2 表示内容

```text
┌──────────────────────────────────────────┐
│ Codex · keylink-studio                   │  どのセッション
├──────────────────────────────────────────┤
│ コマンド実行の許可                          │  要求の種別
│                                          │
│  mkdir ko2-test                          │  ★主表示
│                                          │
│  作業ディレクトリ: C:\01.keyboards\...      │
│  理由: ワークスペース内に ko2-test ディレク  │  AIが書いた理由
│        トリを作成してよいですか？            │
├──────────────────────────────────────────┤
│  ▸ 許可                                  │  ★選択肢
│    mkdir を今後許可                        │  エンコーダで ▸ が動く
│    中止                                  │
├──────────────────────────────────────────┤
│  ⟳ 選ぶ    ✅ 決定    ⤢ ターミナルを開く    │
└──────────────────────────────────────────┘
```

| 表示項目 | Codex | Claude Code |
|---|---|---|
| **主表示** | `commandActions[].command` | `tool_input.command` |
| コマンド全文 | `command`（`powershell.exe -Command '...'` を含む冗長な形。副次表示） | — |
| 理由 | `reason`（**AI がユーザーの言語で書く**。実測は日本語） | — |
| 作業ディレクトリ | `cwd` | `cwd` |
| ツール種別 | `kind` | `tool_name` |
| 選択肢 | `availableDecisions`（**要求ごとに読む**） | Host 側で `許可 / 拒否` を正規化 |

**Codex の `command` をそのまま主表示にしてはならない。** `powershell.exe -Command '...'` の
ラッパに本質が埋もれる。

### 7.3 実装上の注意

- **HUD のサイズは DPI を考慮した論理単位で指定すること。** KO-1 のプローブは物理ピクセル
  指定（420×260）だったため、高DPI環境では表示面積が実質的に縮む
- **本番の HUD ページは Vite にバンドルされる通常のルートとして作ること。**
  素の HTML を `ui/public/` に置くと `@tauri-apps/api` を import できず、
  `withGlobalTauri`（本番 `main` ウィンドウにも `window.__TAURI__` を露出させる設定）が必要になる
- クリックすると 20ms 未満だけ HUD が前面化する（KO-1 の C7）。表示専用なので実害はないが、
  排除するなら `WM_MOUSEACTIVATE` で `MA_NOACTIVATE` を返す

---

## 8. ScreenKey の表示

`AI_CLIENT_STATE` を **8→9バイト**へ1バイト拡張する。capability **bit 14** を新設する
（bit 13 まで使用済み）。

| 値 | 意味 | 描画 |
|---|---|---|
| 0 | 通常 | 従来どおり |
| 1 | 待機中・**HUD の対象** | 明るい枠 ＋ 種別アイコン |
| 2 | 待機中・対象ではない | 通常の点滅 ＋ 種別アイコン |
| 3 | 回答を送信した直後（0.6秒） | 確定表示 |

種別アイコンは固定 enum とし、**文字列は wire へ載せない**。

```text
▶ 実行   ✎ 編集   ⬇ 取得   ？ 質問   ⚠ 要注意
```

- 未広告のデバイスへは従来の 8バイトを送る
- 対象表示は**押した個体へ unicast する**（同報すると他のキーボードの表示が変わる）

> ScreenKey に `git push` のような短文を出すことは技術的には可能だが（ASCII 8字程度）、
> 判断の根拠にはならない量である。**初期実装ではアイコン enum のみとし、テキスト downlink を
> 作らない。** これにより Firmware のフォント実装と分割送信も不要になる。

---

## 9. 回答経路

### 9.1 Codex（同時転送＋first-wins方式）

**Broker は `item/commandExecution/requestApproval` を記録しつつCLIへ転送する。HUDとTUIを
同時に生かし、先に確定した回答だけをApp Serverへ送る。**

```text
App Server ──requestApproval──> [Broker: Pending] ──requestApproval──> CLI/TUI
                                     │                         │
                                     └──────> HUD               │
                                               │                │
                               先に回答した側 ─┴────────────────┘
                                               │
App Server <────────────response(decision)─────┘
```

2026-09-05のKO-2追加検証では、`race`モードでHUD相当の回答が先着するとTUIの承認画面が
自動的に閉じ、コマンド結果は1回だけ表示された。CLIの`accept`が先着したランでは、要求から
約1.94秒後にCLIが応答し、1ms後に`serverRequest/resolved`、約3.28秒後に
`turn/completed(status=completed)`を観測した。さらに約8秒後、検証用プローブが意図的に
重複`accept`を送ってもJSON-RPCエラーと再実行は起きなかった。CLI先行`cancel`でも、遅着した
プローブの`accept`で実行へ反転せず、turnは`interrupted`のままだった。

プローブは重複耐性を測るため遅着responseも意図的に送ったが、**製品実装はその耐性へ依存しない。**
要求ごとにfirst-winsの排他制御を置き、先着回答を転送した後のHUD操作／CLI responseは破棄する。
App Serverの`serverRequest/resolved`を観測した時点でもHUDを閉じ、要求を`Resolved`へ進める。

- `decision` は `availableDecisions` の要素を**そのまま不透明値として**返す。
  文字列とオブジェクトが混在するため、再構築してはならない
- `availableDecisions` の**集合は要求ごとに変わる**。実測では `accept` /
  `acceptWithExecpolicyAmendment` / `cancel` の3つで、`decline` も `acceptForSession` も
  含まれていなかった。Host が固定の選択肢集合を持ってはならない
- `availableDecisions` が無い／空なら HUD からの回答を有効にせず、そのまま CLI へ転送する

保持方式は技術的な縮退候補として残すが、通常経路には採用しない。要求をCLIへ見せないため、
HUDが使えない場合に遅延転送タイマーが必要となり、ユーザーが回答できるまで不要に待たされるためである。

### 9.2 Claude Code（hook decision）

**`claude_observer.rs:323` の 204 応答を、`PermissionRequest` に限り decision へ差し替える。**

```json
{"hookSpecificOutput":{"hookEventName":"PermissionRequest",
  "decision":{"behavior":"allow"}}}
```

```json
{"hookSpecificOutput":{"hookEventName":"PermissionRequest",
  "decision":{"behavior":"deny","message":"<理由>"}}}
```

- **拒否には理由を添える。** KO-3 で `message` がモデルに届くことを確認済み。
  ターミナルにも `Denied by PermissionRequest hook` と明示される
- hook の `timeout` を `CLAUDE_TOOL_HOOK_TIMEOUT_SECONDS = 1` から延長する。
  KO-3 の Q6 のとおり、延長してもユーザーが固まるリスクはない
- **他の hook は 204 のまま維持する**
- Studio が回答を持たない場合は必ず 204 へ縮退させ、ターミナルに委ねる

### 9.3 「常に許可」

| クライアント | 提供 | 理由 |
|---|---|---|
| **Claude Code** | **しない** | `ruleContent` が完全一致の文字列で、Claude は毎回違うコマンドを生成するため実効性が乏しい。かつ `destination: "localSettings"` で恒久権限になる。ターミナルの選択肢のほうが広いルール（`New-Item *`）を作る |
| **Codex** | 検討可 | `proposedExecpolicyAmendment` はプログラム単位（`mkdir`）で実効性がある |

Codex で提供する場合も **`Fn` 併用必須**とし、単押しでは出さない。

### 9.4 二重回答の調停

**ターミナルと HUD の両方が生きているため、先に確定したほう1件だけを採用する排他制御が要る。**

- 要求ごとに状態を持ち、`Pending → Resolving → Resolved` を1回だけ通す
- Codex: HUD/CLIのどちらから回答が来ても、`Pending`を先に`Resolving`へ変えた側だけをApp Serverへ
  転送する。遅着responseは記録したうえで破棄する
- Codex: `serverRequest/resolved`を受信したら、未処理のHUD操作を無効化してHUDを閉じる
- Claude Code: hook の応答を返す直前に、ターミナル側で既に解決していないかを確認する
- Codexは最初からCLIへ転送するため、**30秒後の遅延転送は設けない**

---

## 10. Host 状態機械

`AiDisplaySlots` の隣に、単一インスタンスの「HUD 対象」状態を置く。

| 遷移 | 条件 |
|---|---|
| 対象を設定 | 待機中セッションの ScreenKey 短押し、または待機が1件だけになったとき自動 |
| 対象を解除 | 回答した／外部で解決された／セッション終了／デバイス切断／監視停止 |

- **HUD の表示自体はセッションごとではなく1つ。** 対象を切り替えて中身を差し替える
- 待機中セッションが複数あっても、ScreenKey は全件が自分の状態を表示し続ける
  （HUD だけが1件を指す）
- 解除時は全 slot の `AI_CLIENT_STATE` を full send で再送する
  （既存の selection epoch と同じ手法）

---

## 11. Host Link への影響

| 項目 | 変更 |
|---|---|
| `AI_CLIENT_STATE` | 8 → **9バイト**（末尾に ScreenKey 表示状態 1バイト） |
| capability | **bit 14** を新設 |
| `HOST_ACTION` uplink | 変更なし。既存の `action_id` / `value` を使う |
| `APP_LAYER` | 既存機能を再利用（HUD 表示中のレイヤ切替） |
| 新規 packet type | **不要** |
| protocol version | **変更なし** |

**文字列を wire へ載せる経路は作らない。**

### uplink の解釈

| 条件 | 動作 |
|---|---|
| 待機中 slot の ScreenKey 短押し | HUD 対象を設定 |
| それ以外の ScreenKey 短押し | `FocusAiTerminal`（現行どおり） |
| ✅ / ❌ / エンコーダ | HUD 対象への回答。対象が無ければ無視してログ |

**Firmware は AI の request ID も選択肢も保持しない。**

---

## 12. 安全性と設定

| 項目 | 方針 |
|---|---|
| 既定 | **無効。** Settings から明示的な警告つき opt-in |
| 権限 | 既存 host action の制約を継承（device 単位の許可リスト、監視中のみ、同一 `seq` は1回） |
| 誤爆防止 | HUD 出現直後 400ms は ✅ を受け付けない |
| 「常に許可」 | `Fn` 併用必須。単押しでは出さない |
| 中断 | ❌ 長押しのみ。取り消せない操作への逃げ道を1つ用意する |
| 縮退 | 失敗時は必ずターミナルへ委ねる。**自動許可へ倒さない** |
| 監査ログ | セッション、要求種別、押下、送出可否、失敗理由を記録 |
| 機微情報 | **コマンド文字列や書き込み内容が Studio の UI に表示される**（新規の経路）。ログへ残すか、画面共有時の伏字モードを別途決める |

---

## 13. 段階的実装計画

| 段階 | 内容 | 得られる体験 |
|---|---|---|
| **1** | HUD ウィンドウ（表示のみ・回答なし）。Codex / Claude Code の要求内容を表示 | **これだけで価値がある。** 画面を切り替えずに「何を聞かれているか」が分かる |
| **2** | Codex の同時転送＋first-wins代理応答。✅ / ❌ とエンコーダ選択 | HUDとターミナルのどちらからでも直ちに回答できる |
| **3** | Claude Code の decision 経路とfirst-wins調停 | 両クライアントが揃う |
| **4** | ScreenKey の 9バイト拡張と種別アイコン、`APP_LAYER` 切替 | 気づきやすさと誤爆防止 |
| **5** | 異常時のターミナル縮退、監査ログ、Settings | 実運用に耐える |

**段階1が単独で意味を持つ**のが良いところで、回答機能の是非を決める前に価値を確認できる。

---

## 14. 未確定事項

1. **本番フル構成での HUD 再測定。** KO-1 のプローブは最小の `tauri::Builder` であり、
   トレイ・多数のコマンド・監視スレッドを持つ本番構成ではない
2. **HUD のモニタ選択。** 現在の実装はプライマリモニタ右下固定
3. Claude Code の hook `timeout` の上限値
4. `item/fileChange/requestApproval` / `item/permissions/requestApproval` /
   `item/tool/requestUserInput` の各要求の扱い（本書は command approval を初期対象とする）
5. Codex の `proposedExecpolicyAmendment` を適用したときの永続範囲
6. 複数 thread / 複数 connection が同時に承認待ちになる場合

---

## 15. 非対象

- ScreenKey へ要求内容やコマンド文字列を表示すること（§2、§8）
- 排他的フルスクリーンアプリとの共存（HUD を出さず ScreenKey の点滅のみへ縮退）
- テキスト入力を伴う回答（Codex の「拒否して指示を伝える」、`isOther` の自由記述）
- MCP elicitation
- Keylink Studio 外から起動されたセッションへの回答
- 転送プレビューウィンドウの仕様（`ai-response-transfer-design.md` が正本。
  ただしウィンドウ層は §7.1 を共有する）

---

## 16. 置き換えた文書から引き継いだ判断

置き換え対象2文書の結論のうち、実測を経てもなお有効なものを明示する。

| 判断 | 出典 | 扱い |
|---|---|---|
| AI protocol が明示した配列順だけを正本とする | interaction-design §6.1 | **維持**。KO-2 で `availableDecisions` の混在型を実測し補強 |
| 回答値は意味を再構築せず不透明 JSON として保持する | interaction-design §6.1 | **維持**。KO-2 で必要性が裏付けられた |
| セッションごとの専用ターミナルウィンドウ | 両文書 | **維持**。`FocusAiTerminal` として実装済み |
| 既定無効・device 単位の許可リスト・監視中のみ | 両文書 | **維持** |
| 失敗時は常に「ターミナルで訊く」へ縮退し、自動許可へ倒さない | prompt-response-design §10 | **維持**。実測でフォールバックが自動的に存在することも判明 |
| 選択肢モード中の ScreenKey は HID キーコードを送らない | prompt-response-design §5.5 | **維持**（`APP_LAYER` 切替で実現） |
| ScreenKey の位置＝選択肢の番号 | 両文書 | **破棄**（§2、§6.1） |
| 回答前にターミナルを前面化する | 両文書 | **破棄**（§3.1）。`⤢` による任意の前面化として残す |
| キー入力送出（`SendInput`）で回答する | prompt-response-design §5 | **破棄**。両クライアントとも in-band 経路が実測で成立した |
| Claude Code の回答は後続の別課題 | interaction-design §10 | **破棄**。KO-3 で成立を確認 |
