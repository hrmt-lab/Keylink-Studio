> **この文書は置き換えられました。** 現行の正本は [ScreenKey と HUD による AI 承認・回答 設計](ai-approval-hud-design.md) です。
> 本書は当時の調査記録として残しています。引き継いだ判断と破棄した判断の対応は同文書 §16 にあります。

# ScreenKeyからのAI許可・入力応答設計検討書

- 文書状態: 調査結果と方式比較（実装仕様確定前、実装未着手）
- 作成日: 2026-08-24
- 対象: Keylink Studio Host、Host Link v2、ScreenKey Firmware描画
- 対象ハードウェア: ScreenKey 4個、通常キー7個、エンコーダ1個を搭載したキーボード
- 基準環境:
  - `codex-cli 0.149.0`、experimental App Server schema SHA-256
    `4F4A8D8F53F971B97F818639F58C8D26BB68BFCDFA2D2F20572CB97E6761AB91`
  - Claude Code `2.1.241`
  - Windows Terminal `1.24.11911.0`
- 関連文書:
  - `screenkey-ai-interaction-design.md`（**同一主題の別案**。操作フローと専用ウィンドウ化は
    一致するが、回答経路の結論が異なる。差分は §5.1 を参照）
  - `claude-code-screenkey-multisession-design.md`（本書は同文書 §18 非対象の
    「ScreenKeyからClaude Codeのpermission／inputへ回答する機能」を引き取る）
  - `ai-display-slot-multiscreen-host-design.md`
  - `ai-session-display-switching.md`
  - `packet-spec.md`

---

## 1. 目的

同時に最大4セッションを表示するキーボードで、あるセッションが許可待ち／入力待ちに
なったとき、ScreenKeyの押下だけで回答を完了できるようにする。

対象とする操作は次の一連の流れである。

1. 許可待ち（黄色点滅）または入力待ち（オレンジ呼吸明滅）のScreenKeyを押す
2. そのセッションのウィンドウが前面に出る
3. 4つのScreenKeyが選択肢モード（番号表示）へ切り替わる
4. 選択肢に対応するScreenKeyを押すと、その選択がAIへの回答になる

---

## 2. 決定事項

1. 選択肢はScreenKeyへ内容を表示せず、**上から順に左から順の位置対応**だけを扱う。
   一番左のScreenKeyが選択肢の1番目、左から2番目が2番目に対応する。
2. 権威ある選択肢表示はターミナル自身とする。ScreenKeyは番号ボタンに徹する。
3. セッションは新規タブではなく**新規ウィンドウ**で起動する方式へ変更する。
4. 回答の送出は、まず**前面ウィンドウへのキー入力送出**方式を採る。
   in-band方式（Broker注入／decision hook）は将来の強化案として保留する。
5. hookは観測専用のままとする。`claude-code-screenkey-multisession-design.md` の
   決定 #6 と §5.2 の構造的隔離は維持する。
6. 機能は既定で無効とし、明示的なopt-inでのみ有効化する。

---

## 3. 操作フローと状態

対象状態は `AiActivityState` の `WAITING_APPROVAL` と `WAITING_INPUT` である。
点滅色とFirmware描画の対応はFirmware側の責務とし、本書は状態enumで扱う。

```text
[通常表示]
   │ 押下（該当slotが WAITING_APPROVAL / WAITING_INPUT かつ回答可能）
   ▼
[前面化]  対象セッションのウィンドウをforegroundへ
   │
   ▼
[選択肢モード]  4 ScreenKeyへ 1 / 2 / 3 / 4 を表示
   │ 押下（N番目）
   │   直前検証: foregroundが対象HWND / 状態が待機継続中 / TTL内
   ▼
[回答送出]  前面ウィンドウへキー入力を送る
   │
   ▼
[通常表示へ復帰]  全slotの AI_CLIENT_STATE を full send
```

選択肢モードは次のいずれでも解除する。

- 回答した
- TTL経過（既定15〜20秒）
- 外部で解決された（ターミナルで回答、Esc、Turn中断、セッション終了）
- 監視停止、デバイス切断

---

## 4. 調査結果

### 4.1 Codex（App Server schemaを実生成して確認）

`codex app-server generate-json-schema --experimental` の出力を直接確認した。

| リクエスト | 順序付き選択肢 | 併せて取得できる情報 |
|---|---|---|
| `item/commandExecution/requestApproval` | **`availableDecisions`**（"Ordered list of decisions the client may present for this prompt."） | `command`、`commandActions`、`cwd`、`reason`、`proposedExecpolicyAmendment`、`networkApprovalContext`、`additionalPermissions` |
| `item/fileChange/requestApproval` | なし（固定4択） | `itemId`、`reason`、`grantRoot`。差分本文は含まれず`itemId`でitemと相関が必要 |
| `item/permissions/requestApproval` | なし（N択ではない） | `permissions`、`cwd`、`reason` |
| `item/tool/requestUserInput` | **`questions[].options[]`**（配列＝順序あり） | `header`、`question`、`options[].label`、`options[].description`、`isOther`、`isSecret`、`isBlocking` |

決定語彙も確定した。

- コマンド実行: `accept` / `acceptForSession` /
  `acceptWithExecpolicyAmendment{execpolicy_amendment[]}` /
  `applyNetworkPolicyAmendment{host, allow|deny}` / `decline`（拒否・Turn継続） /
  `cancel`（拒否・Turn即中断）
- ファイル変更: `accept` / `acceptForSession` / `decline` / `cancel`
- 権限: N択ではなく `permissions` ＋ `scope`（既定 `turn`）＋ `strictAutoReview` を返す構造
- 入力待ち: `{answers: {"<questionId>": {answers: ["<label>"]}}}`。
  回答は**label文字列**なので `options[]` のindexから直結できる

Brokerは全文を素通ししており、`classify_json_rpc()` がidだけを抽出している。
本文はすでにHostのメモリを通過しているため、抽出の追加自体は可能である。
ただし現状 `CodexBrokerEvent::Message` はmetadataしか運ばない設計であり、
選択肢を使うと本文の一部をHost側へ持ち出すことになる。機微情報の取り扱いが新たに発生する。

### 4.2 Claude Code（`2.1.241` のバイナリ内スキーマを確認）

`PermissionRequest` hookの**入力**:

```text
{ hook_event_name: "PermissionRequest", tool_name, tool_input, permission_suggestions? }
```

- `tool_input` に実際のコマンド／ファイル内容が入る
- `permission_suggestions` はTUIの「Yes, and don't ask again for X」のX相当
- `suppress_always_allow_rule` という概念があり、「常に許可」を提示してはいけない場合がある
- **画面に出る選択肢リストそのものは届かない**。Host側で再構成する必要がある

`PermissionRequest` hookの**出力**:

```text
hookSpecificOutput.decision =
    { behavior: "allow", updatedInput?, updatedPermissions? }
  | { behavior: "deny",  message?, interrupt? }
```

- decisionを返せることを確認した。`updatedPermissions` は `permission_suggestions` と同型で、
  「常に許可」は受け取ったsuggestionをそのまま返すだけで表現できる
- `ask` 相当の分岐は存在しない。decisionを返さなければ従来どおりターミナルで訊かれる。
  つまり**現在の204空bodyがそのまま安全なフォールバック**になる
- `permissionDecision` は `PreToolUse` 専用であり、`PreToolUse` はauto承認されるtoolにも
  発火するため本用途には使えない

その他:

- `Elicitation` / `ElicitationResult` hookの出力は `action: accept|decline|cancel` ＋ `content`
- `AskUserQuestion` の選択肢は `tool_input.questions[].options[].label` に入り取得できる。
  ただし**回答の返却方法は未確定**。tool定義に「`answers`: User answers collected by the
  permission component」があるため `behavior:"allow"` ＋ `updatedInput` に載せる形が有望

本節はClaude Code `2.1.241` に対する確認である。Codexのschema hash pinと同様に、
バージョン固定と再検証の対象として扱う。

---

## 5. 回答方式の比較と選定

### 5.1 比較

| | in-band方式（Broker注入／decision hook） | **キー入力送出方式（採用）** |
|---|---|---|
| 画面との序数一致 | ケースによりHost側の推測が入る | **定義上必ず一致**（画面のN番目にNを送る） |
| 画面の表示ズレ | Codex TUIのプロンプトが残る懸念 | なし。画面がそのまま進む |
| hook観測専用の設計 | 反転が必要 | **維持できる** |
| クライアント依存 | Codex／Claudeで別実装 | 共通。将来のクライアントにも効く |
| 実装量 | 大 | 小 |
| 「常に許可」 | 永続ルールとして正しく表現できる | TUIが出す選択肢をそのまま使うだけ |
| フォーカス依存 | なし | あり（前面化とセットで成立） |

同一主題の `screenkey-ai-interaction-design.md` は**in-band方式を採り、キー入力注入を
採用しない**という逆の結論を採っている。同文書は初期対象をCodex command approvalと
単一questionの`requestUserInput`へ絞ることで、序数をAI protocolが明示した配列順だけに
依拠させ、フォーカス状態とTUI実装への依存を排している。Claude Codeは別の成立境界として
後続で扱う。本書との差は初期対象範囲と依存先の取り方であり、両案とも操作フロー
（前面化してから順序で回答）と専用ウィンドウ化については一致している。
どちらを正本とするかは未決である。

### 5.2 選定理由

本設計の操作フローでは、ユーザーは前面に出たターミナルの実際の選択肢を読んでから押す。
したがってHostが考える序数と画面の序数が完全に一致していなければならない。
1つズレれば、ユーザーは「許可」のつもりで「拒否」を押すことになる。

§4.2のとおりClaude Codeの許可待ちは画面の選択肢リストがHostへ届かず、Hostが
`permission_suggestions` と `suppress_always_allow_rule` から再構成することになる。
ここに推測が入る余地がある。

キー入力送出方式は「画面のN番目にNを送る」だけなので、**選択肢リストをHostが解釈する必要
そのものが消え、一致問題と設計反転を同時に解決する**。

初期検討でキー入力送出を却下した理由はフォーカス奪取とウィンドウ特定の不安定さであったが、
本フローはステップ2で意図的に前面化し、§6の名前付きウィンドウでウィンドウを一意に特定するため、
その2つの前提が解消されている。

### 5.3 送出キー

- 第一候補: 数字キー `1`〜`4`。Claude Codeの許可プロンプトは番号付きで数字選択できる
- 代替: `↓` × (N-1) ＋ `Enter`。番号が振られていないUIでも効くが初期選択位置に依存する
- Codex TUIが数字キーを受けるかは未実測

Windows TerminalはGUIアプリでありConPTY経由で入力を受けるため、`PostMessage` ではなく
`SendInput` を用いる。送出先はフォーカスされたウィンドウであり、§5.4の検証で担保する。

### 5.4 送出直前の検証（fail-safe）

ScreenKey N 押下後、送出の直前に次を検証する。

1. `GetForegroundWindow()` が対象セッションのHWNDと一致するか
2. Hostのセッション状態がまだ `WAITING_APPROVAL` / `WAITING_INPUT` か
3. 選択肢モードのTTL内か

いずれかが外れたら**送出せず中止し、選択肢モードを解除してログへ記録する**。
§6.4のフォアグラウンド権取得が失敗していた場合も検証1で弾かれるため、
無関係なアプリへ数字が入力される事故は起きない。

### 5.5 Firmware側の必須条件

**選択肢モード中のScreenKeyはHIDキーコードを一切送ってはならない。**
通常キーコードを併送する設定だと、前面のターミナルへその文字が入力される。
ホストアクション専用キーとして扱う。

---

## 6. ウィンドウ前面化

### 6.1 現状と問題

`claude_launcher.rs` と `codex_launcher.rs` はどちらも次の形で起動している。

```text
wt.exe -w 0 new-tab --title "<Codex|Claude Code>: <proj>" --suppressApplicationTitle powershell ...
```

- `-w 0` は「直近に使ったWindows Terminalウィンドウ」であり、全セッションが1つのWTウィンドウの
  タブとして並ぶ。ユーザーが複数のWTウィンドウを持つ場合はどこに入るかも不定である
- `wt.exe` は既存WTプロセスへ委譲して即終了するため、セッションに対応するPIDもHWNDもHostは
  保持していない

実機で確認したところ、WTのウィンドウは次のとおりであった。

```text
class = CASCADIA_HOSTING_WINDOW_CLASS
title = "Claude Code: Keylink-Studio"
```

`--title` と `--suppressApplicationTitle` は意図どおり効いている。ただし**WTのウィンドウ
タイトルはアクティブなタブのタイトル**であるため、タイトル照合で特定できるのは既に前面に
出ているタブだけである。

### 6.2 採用方式: セッションごとの名前付きウィンドウ

新規タブ起動を新規ウィンドウ起動へ変更する。

```text
wt.exe -w keylink-<launch_id> --title "Claude Code: <proj> [<launch_id先頭>]" --suppressApplicationTitle ...
```

- 1セッション＝1ウィンドウとなり、`EnumWindows` でクラス `CASCADIA_HOSTING_WINDOW_CLASS`
  かつタイトル一致からHWNDを一意に確定できる
- 既存 `app_launch.rs` の `bring_running_to_front()` とほぼ同じ実装で足りる

### 6.3 検討して見送った方式

| 方式 | 見送り理由 |
|---|---|
| タブのまま `wt -w 0 focus-tab --target <index>` | `focus-tab`（別名 `ft`）はWT `1.24` に実在を確認したが、index指定でありタブの並びは開閉で変わる。Hostはセッションとindexの対応を維持する手段を持たない |
| UI Automationでタイトル一致のタブをInvoke | index問題は消えるが依存が重くWTのUIツリー変更に弱い。最後の手段 |

### 6.4 フォアグラウンド権

Windowsは `SetForegroundWindow` の呼び出しを制限しており、バックグラウンド常駐のKeylink
Studioからは条件を満たさず、タスクバーが点滅するだけになる可能性がある。ScreenKey押下は
`HOST_ACTION` として届くだけでStudioへキー入力は入らない。

既存の `app_launch.rs:81`（`launch`）と `explorer.rs:130`（`open_folder`）は素の
`SetForegroundWindow` を呼んでおり、**同じ条件下にある**。したがって本機能の実装前に、
既存 `launch` アクションをScreenKeyから実行して前面化が成立するかを実測する。
成立するなら同じ実装を流用でき、成立しないなら既存機能の不具合として先に扱う。

不成立時の対応候補:

| 手段 | 副作用 |
|---|---|
| `AttachThreadInput` で現foregroundスレッドへ一時アタッチしてから `BringWindowToTop` ＋ `SetForegroundWindow` | 小。定番手法 |
| `FlashWindowEx` へ縮退 | なし。フォールバックとして妥当 |
| `SPI_SETFOREGROUNDLOCKTIMEOUT` を0へ変更 | ユーザー環境の設定を書き換える。採用しない |

ScreenKeyに通常キーコードを併送してStudioの入力履歴を作る案は成立しない。その入力は前面
アプリへ入るだけである。

### 6.5 セッションとウィンドウの対応

- Claude Code: `launch_id` はStudioが発番しているため、ウィンドウ名とタイトルに埋めれば確実。
  `session_id` は起動後に決まるため使えない
- Codex: `thread_id` は接続後に決まるため、起動単位の識別子を別途発番してタイトルへ埋め、
  `thread_id` との対応はBrokerの既存の所有者追跡（`owned_thread_ids`）で解決する
- WSL上のCodexも同じwt起動であり扱いは同一
- Keylink Studio外から起動されたセッションは特定不能であり、前面化不可として扱う

### 6.6 副次利用

WTのウィンドウタイトルが常にアクティブタブのタイトルになるため、Hostは現在どのセッションが
前面かを読み取れる。既存 `ForegroundWatcher`（`EVENT_SYSTEM_FOREGROUND` フック）へ相乗り
させることで、次の用途に使える。

- すでに前面のセッションに対して前面化を実行せず、無駄なフォーカス奪取を抑止する
- 前面のセッションのScreenKeyを強調表示する

---

## 7. 選択肢の序数と個数

### 7.1 序数

キー入力送出方式では、ScreenKeyの位置Nに対して数字Nを送るだけであり、序数の一致は構成上
保証される。Host側で選択肢の意味を解釈しない。

### 7.2 個数

`availableDecisions` は最大6種類、`AskUserQuestion` も4択に加えて「その他」があり得るため、
選択肢が4個を超える場合が存在する。

- 初期実装は4個までとし、`option_count > 4` はログして非対応とする
- 将来はエンコーダ回転で 1-4 / 5-8 のページ送りとし、ScreenKeyは現在ページの番号を表示する

Hostが個数を確信できないケース（Claude Codeの許可待ち）の扱いは未確定である（§11）。

---

## 8. Host Link拡張

### 8.1 downlink

選択肢の内容を送らないため、payloadは極小で足りる。`STATE_UPDATE` へ新feature
（例 `0x0B AI_PROMPT`）を1種追加し、**4スロット分を1 packetで原子的に切り替える**。
分割送信は半端な描画状態を経由するため採らない。

| Payload Offset | Size | Field | Notes |
| --- | ---: | --- | --- |
| `0..1` | 2 | `prompt_id` | u16 LE。世代管理とstale検出用 |
| `2` | 1 | `origin_slot` | 押されたslot |
| `3` | 1 | `option_count` | `0` = 選択肢モード解除、`1..=4` |
| `4` | 1 | `flags` | 予約 |
| `5` | 1 | `ttl_sec` | 自動解除の猶予 |

capability bitを1つ新設する（例 bit 14 `AI_PROMPT_MENU`）。未広告のデバイスでは選択肢
モードへ入らず、従来動作（`cycle_ai_session`）に留める。

文字列はwireへ載せない。既存のenum描画方針を維持する。

### 8.2 uplink

初期実装ではuplinkを変更しない。Firmwareは従来どおり `&host_action <ID> <slot>` を送り、
Hostが文脈で解釈する。

| 条件 | 動作 |
|---|---|
| 選択肢モード表示中 | 押下slot＝選択肢番号として回答 |
| 未表示 かつ 該当slotが `WAITING_APPROVAL` / `WAITING_INPUT` かつ回答可能 | 前面化して選択肢モードを開く |
| それ以外 | 従来どおり `cycle_ai_session` |

この方式は「どのプロンプトへの回答か」をuplinkが名乗らないため、解決直後の押下が誤回答に
なり得る。§5.4の検証とHost側のguard時間で吸収し、将来は `prompt_id` を載せた専用uplink
（`AI_PROMPT_ANSWER`）へ強化する。

### 8.3 レイテンシ

uplinkは20 msのサブループで、`AI_CLIENT_STATE` の送出は500 msの制御ループで行われている
（`commands.rs`）。このままでは選択肢モードの表示が最大500 ms遅れるため、
`handle_uplink_events()` から即時downlinkを送る経路を用意する。

### 8.4 複数キーボード

通常のslot stateは全対応デバイスへ同報しているが、**選択肢モードは押下した個体へunicast
する**。同報すると他のキーボードの4画面が、押していないユーザーの操作でメニュー化される。
既存の同報規則からの明示的な逸脱点として記録する。

---

## 9. Host状態機械

`AiDisplaySlots` の隣に単一インスタンスの選択肢モード状態を置く。

- **開く条件**: 押下slotのセッションが `WAITING_APPROVAL` / `WAITING_INPUT` であり、
  対象ウィンドウのHWNDを解決できること。解決できない場合は開かず
  `ai_prompt_not_answerable` としてログし、表示を変えない
- **排他**: 選択肢モードは同時に1つだけ。開いている間に別セッションが待機状態になっても
  割り込ませない。解除後の再描画で点滅が現れる
- **復帰**: 解除時は全slotの `AI_CLIENT_STATE` をfull sendで再送する。既存のselection epoch
  と同じ手法（`ai-session-display-switching.md` §4）を流用する

---

## 10. 安全性と設定

- 既定は無効。`[ai_client.prompt_answer] enabled = false` とし、Settingsで明示的な警告つき
  opt-inとする。キーボードにコマンド実行の承認権限を与える機能である
- 監査ログを残す。セッション、対象種別、押下位置、送出可否、検証失敗理由を記録する
- 失敗時は常に「ターミナルで訊く」へ縮退させ、自動許可へ倒さない
- 送出するキーは選択肢の位置に対応する数字のみとし、任意文字列の送出経路は作らない

---

## 11. 未確定・要実測

| # | 項目 | 影響 |
|---:|---|---|
| 1 | 既存 `launch` アクションをScreenKeyから実行して前面化が成立するか | §6.4。成立しなければ前面化実装が分岐する。最優先 |
| 2 | `wt -w <name>` の名前付きウィンドウ起動の実動作 | §6.2の前提 |
| 3 | Codex TUIが数字キーで選択肢を選べるか | §5.3。不可なら代替キー列へ |
| 4 | Hostが選択肢の個数を確信できないケースの扱い | §7.2。4つ全部点灯か、確信できる数だけ点灯か |
| 5 | `AskUserQuestion` の回答を `updatedInput.answers` で返せるか | in-band方式へ進む場合のみ必要 |
| 6 | Broker注入時のTUI重複応答と表示ズレの挙動 | in-band方式へ進む場合のみ必要 |

---

## 12. 段階実装

1. §11-1 と §11-2 の実測
2. 新規ウィンドウ起動への変更とHWND解決（前面化のみ、選択肢モードなし）
3. 選択肢モードのdownlinkとHost状態機械、Firmware描画
4. キー入力送出と§5.4の検証、監査ログ
5. 実機E2E: 4セッション、待機→前面化→選択肢→回答→復帰、外部解決とのレース、TTL満了、
   フォーカスを外した状態での中止

---

## 13. 非対象

- 選択肢の文言やコマンド本文のScreenKeyへの表示
- 5個以上の選択肢（初期実装）
- テキスト入力を伴う回答（Codexの「拒否して指示を伝える」、`isOther` の自由記述など）
- Keylink Studio外から起動されたセッションへの回答
- in-band方式（Broker注入／decision hook）の実装
- hook受信口を観測専用から変更すること

---

## 14. 参照

- 既存Broker実装: `crates/rawhid-host-core/src/codex_broker.rs`
- 既存Codex activity実装: `crates/rawhid-host-core/src/codex_activity.rs`
- 既存Claude hook登録: `crates/rawhid-host-core/src/claude_hooks.rs`
- 既存Claude activity実装: `crates/rawhid-host-core/src/claude_activity.rs`
- 既存Windows Terminal起動実装: `crates/rawhid-host-tauri/src/claude_launcher.rs`、
  `crates/rawhid-host-tauri/src/codex_launcher.rs`
- 既存前面化実装: `crates/rawhid-host-tauri/src/app_launch.rs`、
  `crates/rawhid-host-tauri/src/explorer.rs`
- 既存foreground監視: `crates/rawhid-host-tauri/src/foreground.rs`
- 既存表示slot実装: `crates/rawhid-host-tauri/src/state.rs`
- Codex App Server schema生成: `codex app-server generate-json-schema --experimental`
- Claude Code公式Hooks reference: <https://code.claude.com/docs/en/hooks>
