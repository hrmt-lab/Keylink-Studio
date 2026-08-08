# Codex／Claude Code共通 ScreenKeyセッション切替仕様

## 1. 目的

1つのScreenKeyで、Keylink Studioが現在観測しているCodex／Claude Codeのセッションを、キー押下ごとに順送りして表示する。
クライアント種別は切替条件にせず、同じ表示候補として扱う。

## 2. 用語

- 表示候補: ScreenKeyへ表示できる有効なAIセッション
- 選択中候補: 現在ScreenKeyへ送信する1件
- 共通セレクタ: CodexとClaude Codeの表示候補をまとめ、選択を保持するHost側コンポーネント
- selection epoch: 選択変更を検出してfull state送信を強制するHost内部値。Host Link packetには載せない

## 3. 表示候補

候補順は次のとおりとする。

1. 現在のCodexセッション（有効な場合）
2. Claude Codeセッション（Keylink Studioへ登録された順）

Claude Codeは`(launch_id, session_id)`を識別子とする。同じ`session_id`でも別起動なら別候補である。

現行Codex Brokerは、表示用には現在追跡中のthreadを1件だけ保持する。このため複数のCodex threadを同時に候補へ残すことは本仕様の対象外であり、Codex候補は最大1件である。Claude Codeは複数起動・複数sessionを候補にできる。

終了済み、retired、`session_active = false`のsessionは候補に含めない。

## 4. 選択規則

- 候補が初めて現れた場合は先頭を選択する
- キーを1回押すごとに候補順の次へ進み、末尾の次は先頭へ戻る
- 候補が1件なら選択は変わらない
- 非選択候補の状態変化だけでは選択を変更しない
- 新しい候補が追加されても、現在の選択が有効なら維持する
- 選択中候補が終了したら、終了前の位置に続く有効候補を選ぶ
- 候補が0件になったらセッションなしを送る

選択変更時は、activity、revision、client typeが直前と同じでも、選択後のsnapshotをfull stateとして即時送信する。
5秒heartbeatとUSB再接続後の再送も、選択中候補の最新snapshotを使う。

## 5. 状態更新

CodexとClaude Codeのeventは、どちらが選択中でも常に取り込む。非選択候補のeventは内部snapshotだけ更新し、Host Link出力へは流さない。
その候補へ切り替えた時点で最新snapshotを送る。

`COMPLETED`はクライアント種別や現在の選択に関係なく、完了eventから30秒後にHost内部で`AVAILABLE`へ遷移する。
したがって、完了表示中に別sessionへ切り替え、30秒を超えてから戻しても、期限切れの緑枠を再表示しない。
この期限は新しいTurn開始、session終了、wrapper終了で解除する。

ScreenKeyへ送る`client_type`は選択候補に従う。

- Codex: `0x01 CODEX`
- Claude Code: `0x02 CLAUDE_CODE`

Firmwareは受信した`client_type`でロゴを選び、activity stateは既存の共通表示規約を使う。会話本文、session ID、選択順、selection epochは送らない。

## 6. キー入力

既存`HOST_ACTION`へ組み込みaction `cycle_ai_session`を追加する。Actions画面で任意のaction IDへ割り当て、Firmware keymapでは同じIDを`&host_action <ID> 0`へ設定する。

Host Link packet形式とFirmware Coreの変更は不要である。アクション許可リストと「監視中のみ実行」の既存制約をそのまま適用する。

候補がない場合は`no_active_ai_sessions`としてログへ記録し、表示状態を変更しない。

## 7. Claude Code複数起動

SettingsからClaude Codeを追加起動できる。起動ごとにReceiver、token、plugin directory、`launch_id`を分離する。
停止操作はKeylink Studioが起動したClaude Code連携をまとめてshutdownし、各plugin directoryをcleanupする。

## 8. 非対象

- 1つのAIセッションを複数ScreenKeyへ個別割当する機能
- 複数ScreenKeyへ異なる選択を保持する機能
- 複数Codex threadを同時に候補として保持する機能
- ScreenKeyからapproval／inputへ回答する機能
- Firmwareへsession IDや選択状態を保存する機能

## 9. 受け入れ条件

- Codex 1件とClaude Code 1件以上を候補として循環できる
- Claude Code同士も登録順に循環できる
- 非選択候補のeventで表示が奪われない
- 選択中候補の終了時に次の候補へ移る
- 候補追加時に現在の選択を維持する
- 選択変更時にclient typeを含むfull stateが送られる
- 候補0件／1件で安全に動作する
- Host core、Tauri、UI buildの既存回帰がない
