# Codex 代理承認 Gate (KO-2) 結果

- 状態: 完了
- 実施日: 2026-09-03（保持）、2026-09-05（`race`追加検証）
- 最終判定: **KO-2 成立**（保持・`race`とも成立。製品方針は同時転送＋first-wins）
- 対象 Codex CLI: `codex-cli 0.151.0`（保持）、`codex-cli 0.153.2`（`race`）
- 現行 App Server schema SHA-256: `B06F77062369D481A59CC70720C12B89CB9DD49C385863923262102D3AD6C978`
  （`codex_broker.rs` の現行`SUPPORTED_SCHEMA_SHA256`と一致）
- 実行環境: Windows 11 Pro `10.0.26200.9278`
- 検証コード: `crates/rawhid-host-core/examples/codex_approval_probe.rs`
- 実行: `hold`と`race`の各モード（`race`確定ランは`--delay-seconds 10`）

---

## 1. 背景

`hud-focus-gate-results.md`（KO-1）で HUD がフォーカスを奪わないことが、
`claude-permission-hook-gate-results.md`（KO-3）で Claude Code へ回答を注入できることが確定した。
残る問いが Codex への回答経路である。

Codex の承認要求は **App Server → CLI** 方向の JSON-RPC request
（`item/commandExecution/requestApproval`）であり、通常は CLI が TUI でユーザーに訊いて
response を返す。Keylink Studio の Broker（`crates/rawhid-host-core/src/codex_broker.rs`）は
その間に立つ双方向プロキシである。

検証すべき方式は2つあった。

| 方式 | 内容 | 懸念 |
|---|---|---|
| **代理応答**（`race`） | requestApproval を CLI へ転送したうえで、Broker からも response を送る | **CLI の TUI に出たプロンプトが閉じないのではないか。** 後から届く CLI の response が二重配送にならないか |
| **保持**（`hold`） | requestApproval を CLI へ転送せず、Broker が保持して代わりに応答する | CLI が要求を知らないまま turn が進むのか。App Server が要求にタイムアウトを持たないか |

既存の設計文書は代理応答方式を前提に「Gate 1 で TUI のプロンプトが閉じない場合は
単純な代理応答方式を実装しない」としていた。本Gateでは**両方式を1つのプローブで測れるようにし、
保持方式を先に検証した**。保持方式が成立すれば、TUI のプロンプトが閉じるかという問題そのものが
発生しないためである。

Studio 本体には一切手を入れていない。KO-3 と同じく、
「プロトコルが許すか」と「実装が正しいか」を切り分けるためである。

---

## 2. 判定

**保持方式（`hold`）と代理応答方式（`race`）がともに成立した。**

| 確認点 | 結果 |
|---|---|
| CLI へ転送しない → TUI に承認プロンプトが出ないか | ✅ **出なかった**（実機目視） |
| App Server が CLI 以外からの response を受理するか | ✅ **受理。エラー応答なし** |
| 要求が解決済みとして扱われるか | ✅ **`serverRequest/resolved` が応答の2ms後に発火** |
| turn が正しく完了するか | ✅ **`turn/completed` / `turn.status = completed`** |
| CLI からの遅延・重複 response | ✅ **無し**（CLI は要求を一度も見ていない） |

2026-09-05に`race`を追加検証した。HUD相当回答が先着する経路、CLIの`cancel`が先着する経路、
CLIの`accept`が先着する経路で、先着回答が要求を解決し、遅着responseによる再実行や
JSON-RPCエラーが起きないことを確認した。詳細は§4.1、採用判断は§6。

---

## 3. 検証方法

```text
[Codex CLI]  --WS-->  [プローブ(中継+介入)]  --WS-->  [codex app-server]
 人が別ターミナルで起動        このプログラム         プローブが spawn
```

プローブは Studio の Broker と同じ位置に立ち、`codex_broker.rs` の区間別認証を再現する。

- App Server 起動: `codex app-server --listen ws://127.0.0.1:<port> --ws-auth capability-token --ws-token-file <token>`
- 下流（CLI→プローブ）: `Authorization: Bearer <cli token>` を定数時間比較で検証
- 上流（プローブ→App Server）: 別トークンで接続
- Codex 実行ファイルの解決も Broker と同じ（`where.exe codex` → `com|exe|bat|cmd` の最初の候補）

Codex CLI は TUI であり、プローブは起動しない。起動時にコピペ用の PowerShell ブロックを表示し、
人が別ターミナルで実行する。

### モード

| モード | 動作 |
|---|---|
| `observe` | 素通し（ベースライン） |
| `race` | requestApproval を CLI へ転送し、`--delay-seconds` 後にプローブからも応答する |
| `hold` | requestApproval を CLI へ転送せず、`--delay-seconds` 後にプローブが応答する |
| `hold-forever` | 転送も応答もしない（App Server 側のタイムアウト有無） |

### 終了条件

`--max-approvals`（既定1）件を処理したら自動終了する。「処理した」の判定は次のいずれか。

- **決着を観測**: 対象 id への response を観測し、その後 `turn/completed` を観測し、さらに3秒経過
- **観測窓の満了**: 最初の検出から `--observe-seconds`（既定120秒）が経過

終了理由をログに明示する。これは §8.1 の失敗を受けた設計である。

---

## 4. 確定ランの結果

`codex-approval-probe-20260903-190157.log`

```text
=== SUMMARY: 1 requestApproval occurrence(s) observed ===
id=0 (received at +80747ms)
    note: hold: withheld from CLI + probe response scheduled
    observation outcome: settled (+96657ms)
    probe response: sent=true decision=accept (+90749ms)
    CLI response: seen=false body=-
    duplicate (both probe and CLI answered this id): false
    serverRequest/resolved observed: true (+90751ms)
    turn/completed observed: true turn.status=completed (+93568ms)
    JSON-RPC error response observed for this id: false
=== END SUMMARY ===

shutdown reason: settled (response observed at +90749ms,
                          turn/completed at +93568ms, tail 3s elapsed)
```

要求受信から10秒後（`--delay-seconds 10`）にプローブが `accept` を送信し、
2ms後に `serverRequest/resolved`、約2.8秒後に `turn/completed` が流れた。
その間、**Codex CLI の TUI には承認プロンプトが一切表示されなかった**（実機目視）。

### 4.1 `race`追加検証（2026-09-05）

`codex-cli 0.153.2`と現行schemaを使い、requestApprovalをCLIへ転送したまま、プローブからも
遅延responseを送った。

| 先着回答 | 結果 |
|---|---|
| HUD相当の`accept` | TUIに承認画面が表示された後、プローブ回答で自動的に閉じた。コマンド結果は実機目視で1回だけ表示 |
| CLIの`cancel` | `serverRequest/resolved`後にturnは`interrupted`。遅着したプローブの`accept`で実行へ反転せず、JSON-RPCエラーなし |
| CLIの`accept` | 1ms後に`serverRequest/resolved`、約3.28秒後にturnは`completed`。約8秒後の重複`accept`でもJSON-RPCエラーなし |

CLI先行`accept`の確定ランは次のとおり。

```text
=== SUMMARY: 1 requestApproval occurrence(s) observed ===
id=0 (received at +90980ms)
    note: race: forwarded to CLI + probe response scheduled
    observation outcome: settled (+99534ms)
    probe response: sent=true decision=accept (+100990ms)
    CLI response: seen=true (+92920ms) body={"decision":"accept"}
    duplicate (both probe and CLI answered this id): true
    serverRequest/resolved observed: true (+92921ms)
    turn/completed observed: true turn.status=completed (+96200ms)
    JSON-RPC error response observed for this id: false
=== END SUMMARY ===
```

このプローブは重複耐性を測るため、要求解決後も意図的にresponseを送る。実測上は無害だったが、
**製品実装は重複responseを送らず、先着回答を採用した時点で他方を抑止する。**

---

## 5. requestApproval の実測内容

```json
{
  "id": 0,
  "method": "item/commandExecution/requestApproval",
  "params": {
    "availableDecisions": [
      "accept",
      { "acceptWithExecpolicyAmendment": { "execpolicy_amendment": ["mkdir"] } },
      "cancel"
    ],
    "command": "\"C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -Command 'mkdir ko2-test'",
    "commandActions": [ { "command": "mkdir ko2-test", "type": "unknown" } ],
    "cwd": "C:\\01.keyboards\\OriginalKeyboards\\02.SW\\Keylink-Studio",
    "environmentId": "local",
    "itemId": "exec-882ac982-...",
    "kind": "command",
    "proposedExecpolicyAmendment": ["mkdir"],
    "reason": "ワークスペース内に ko2-test ディレクトリを作成してよいですか？",
    "startedAtMs": 1788429792762,
    "threadId": "01a066b8-5269-71b2-9c8a-d7e64a8302a1",
    "turnId": "01a066b8-e33a-7861-8334-256907f36ccc"
  }
}
```

### 5.1 `availableDecisions` は要求ごとに変わる

この要求では **3要素**であり、しかも**文字列とオブジェクトが混在**していた。

```text
"accept"
{"acceptWithExecpolicyAmendment": {"execpolicy_amendment": ["mkdir"]}}
"cancel"
```

スキーマ上は `accept` / `acceptForSession` / `decline` / `cancel` /
`acceptWithExecpolicyAmendment` / `applyNetworkPolicyAmendment` が定義されているが、
**実際に提示されるのはその一部であり、この要求には `decline` も `acceptForSession` も含まれていない。**

したがって次が確定した。

- **Host が選択肢の集合を固定してはいけない。** 要求ごとに `availableDecisions` を読むこと
- **要素を再構築してはいけない。** オブジェクト variant があるため、不透明値としてそのまま返すこと
- 既存の設計文書が「意味を再構築せず不透明JSONとして保持する」としていた判断は正しい

### 5.2 HUD に出せる情報

| HUDに出す内容 | 取得元 | 備考 |
|---|---|---|
| 何を実行しようとしているか | **`commandActions[].command`** | `mkdir ko2-test`。表示に適する |
| 実際に走るコマンド全文 | `command` | `"C:\Windows\...\powershell.exe" -Command '...'`。長く冗長 |
| 理由 | `reason` | **AIがユーザーの言語で書く**（実測は日本語） |
| 作業ディレクトリ | `cwd` | |
| 選択肢 | `availableDecisions` | 順序付き。そのまま提示する |
| 相関用 | `threadId` / `turnId` / `itemId` | |

**`command` をそのまま HUD に出すべきではない。** `powershell.exe -Command '...'` のラッパが
含まれ、本質が埋もれる。`commandActions[].command` を主表示にし、全文は必要に応じて見せる。

`reason` が日本語で返ってきたのは重要である。**HUD 側で翻訳や要約をする必要がない。**

### 5.3 Codex の「常に許可」は Claude Code より広い

`proposedExecpolicyAmendment: ["mkdir"]` — スキーマの説明は
"allow **similar** commands without prompting"。プログラム単位の緩和である。

KO-3 で判明した Claude Code の `ruleContent`（**完全一致の文字列**）と対照的である。

| | Claude Code | Codex |
|---|---|---|
| ルールの粒度 | 完全一致の文字列 | プログラム単位（`mkdir`） |
| 次に効く見込み | 低い（毎回違うコマンドが生成される） | 高い |
| 永続先 | `.claude/settings.local.json`（恒久） | execpolicy（未調査） |

KO-3 では「キーボードからの常に許可は提供しない」と決めたが、**Codex では実効性があるため
再検討の余地がある**。ただし提供する場合も `Fn` 併用など、単押しにしない方針は維持する。

---

## 6. 製品方式を`race`＋first-winsへ変更する理由

2026-09-03時点では保持方式を採用したが、2026-09-05に実際の利用体験を再検討し、
**HUDとターミナルの両方へ最初から表示するほうがよい**と判断して`race`を検証した。

- 保持方式はHUDが使えないとき、CLIへ要求を見せるまで遅延転送タイマーが必要になる
- `race`ならHUD/TUIのどちらからも直ちに答えられ、普段のCodex操作も失われない
- 実測で、HUD先行時にTUIプロンプトが閉じ、CLI先行時も正常完了することが確認できた
- 重複responseは現行App Serverで無害だったが、製品ではfirst-wins制御により送信自体を抑止できる

保持方式は技術的に成立しているため縮退候補として残すが、通常経路には採用しない。
30秒後のCLI遅延転送も不要になった。

---

## 7. 設計への影響

1. **HUD 方式は Codex・Claude Code の両方で成立する。** 3つのノックアウト要因がすべて解消した
2. **Codex 側は`race`＋first-wins。** Brokerは要求をCLIへ転送しつつHUDにも提示し、
   HUD/CLIの先着回答だけをApp Serverへ送る
3. **`availableDecisions` を読んで HUD に提示する。** 選択肢の集合を Host が固定しない
4. **HUD の主表示は `commandActions[].command` と `reason`。** `command` 全文は副次的に
5. **CLIは最初から表示する。** 未回答時の30秒遅延転送は作らない
6. Codex の「常に許可」（`proposedExecpolicyAmendment`）は Claude Code より実効性がある。
   提供するかは別途判断する

---

## 8. 検証手段側で潰した欠陥

KO-1・KO-3 と同様、**判定に至るまでの障害はすべて計測側にあった**。

### 8.1 固定タイマーによる自動終了は肝心な部分を取り逃す

当初、`--max-approvals` の自動終了タイマーを **requestApproval を「見た瞬間」に開始**し、
`--delay-seconds + 2秒` で終了する作りにしていた。実機の `observe` 実行では
**人が回答する前に7秒で終了した**。

`hold` / `race` で観測したいのは **プローブが応答を送った「後」に流れるフレーム**
（`serverRequest/resolved`、コマンドの実行、`turn/completed`）であり、
固定タイマーでは記録されない。実際、確定ランでは応答から `turn/completed` まで
**2.8秒**かかっており、応答後2秒の窓では取り逃していた。

終了条件を「決着（response → `turn/completed` → 3秒）の観測」または
「観測窓の満了（既定120秒）」に変更し、**どちらで終わったかをログに明示**するようにした。

> KO-1 でも「計測器が肝心なところの手前で止まる」同種の欠陥を踏んでいる。
> **プローブを書くときは、観測窓が「見たい事象の後」まで伸びているかを必ず確認すること。**

### 8.2 Rust の `Command::new` は Windows で `PATHEXT` を見ない

`Command::new("codex")` が `program not found` で失敗した。この環境の `codex` は
npm の `.cmd` シムであり、`codex.exe` は存在しない。Rust の `Command` は裸の名前を
`.exe` としてしか解決しない。

`codex_broker.rs` の `resolve_codex_executable` / `select_codex_executable` に既に正解があり
（`where.exe codex` → 拡張子 `com|exe|bat|cmd` の最初の候補を選ぶ）、これを移植して解決した。
**`where.exe` の出力は複数行になり、拡張子なしの行が先に来る**ため、その行を飛ばす処理が要る。

解決したフルパスを起動時にログへ出すようにした。これがあれば原因特定は一瞬だった。

### 8.3 フレームログの量

確定ランのログは **647,618行**になった。Codex CLI が `command/exec` を高頻度で発行し、
全フレームを全文記録しているためである。調査には有用だが、常用するなら
方向・メソッドでのフィルタが要る。

なお同ランでは `-32600 custom outputBytesCap is not supported with windows sandbox` が
多数観測された。これは Codex 側の挙動であり、本Gateの対象外である。

---

## 9. 非対象

- `hold-forever` による App Server 側タイムアウトの有無
- `item/fileChange/requestApproval` / `item/permissions/requestApproval` /
  `item/tool/requestUserInput` の各要求
- `proposedExecpolicyAmendment` を適用したときの永続範囲
- 複数 thread / 複数 connection が同時に承認待ちになる場合

---

## 10. 3ゲートの総括

| # | 問い | 結果 | 文書 |
|---|---|---|---|
| KO-1 | HUD は作業中のウィンドウからフォーカスを奪わないか | ✅ 成立 | `hud-focus-gate-results.md` |
| KO-3 | Claude Code へ回答を注入できるか | ✅ 成立 | `claude-permission-hook-gate-results.md` |
| KO-2 | Codex へ代理応答できるか | ✅ 成立（保持・`race`。製品は`race`＋first-wins） | 本書 |

**「読むのはモニタの HUD、決めるのはキーボード、ScreenKey は気づくため」という設計の前提は、
すべて実機で裏付けられた。** 実装フェーズへ進める。
