  # キーマップ バックアップ/復元機能 統合プラン 改訂版

  ## Context
 
  ZMK Studio RPC での編集はデバイスの settings(NVS) に保存されるだけで、FW ソースの .keymap
  は変わらない。
  
  FW をフルイレース／settings reset 付きで焼き直すとキーマップが失われる。
  本機能は、現在キーボードに書かれているキーマップを JSON ファイルへ書き出し（エクスポート）、
  後で同じ構成のキーボードへ書き戻す（インポート＝復元）手段を提供し、焼き直し後の復旧を可能にする。
  NVS を介した運用バックアップであり、ソース .keymap 反映は範囲外。

  ## Summary

  - ZMK Studio で現在デバイス上にあるキーマップを JSON バックアップとしてエクスポートし、後で同じ構造のキーボードへ復
    元できるようにする。

  - 復元は 即保存しない。既存の編集セッション上の未保存変更として反映し、ユーザーが既存の 保存 / 変更を破棄 で確定・取
    消する。

  - 復元は Behavior::Unknown { behavior_id, param1, param2 } で raw binding を書き戻す。既存 EditBehavior へ変換しな
    い。

  - USB Serial / BLE Studio の両方を対象にする。behavior 名検証は接続種別で決め打ちせず、可能なら問い合わせ、取得不能
    時だけ検証スキップにする。

  ## Core Design

  - Rust core に JSON v1 形式を追加する。
      - 型: KeymapBackup, BackupDevice, BackupLayout, BackupLayer, BackupBinding, RestoreReport, RestoreIssue,
        RestorePlan, RawBindingWrite, KeymapFileError。

      - 推奨拡張子は -keymap.json。dialog filter は実装互換性重視で json のみにし、defaultPath で推奨拡張子を付
        ける。

      - 必須フィールド: schema: "rawhid-host.keymap-backup", schema_version: 1, app_version, exported_at_ms, device,
        layout, behavior_catalog, layers。

      - 復元の真実は position, behavior_id, param1, param2。behavior は検証用、label は表示用。
      - 秘密情報、設定ファイル内容、ユーザー絶対パスは含めない。

  - Core 純関数を追加する。
      - keymap_backup_from_snapshot(snapshot, behavior_catalog, app_version) -> KeymapBackup
      - parse_keymap_backup(text) -> Result<KeymapBackup, KeymapFileError>
      - plan_keymap_restore(current, target_behavior_names, backup) -> RestorePlan
      - serialize_keymap_backup(backup) -> Result<String, KeymapFileError>

  - RestorePlan と RestoreReport は分離する。
      - RestorePlan { report, writes } は backend 内部用。
      - RestoreReport は UI 返却用で、writes は含めない。

  - 復元判定:
      - apply不可: schema/version 不一致、JSON不正、1 MiB超過、レイヤー数不一致、各 layer の binding 数不一致、各
        layer の position 集合不一致。

      - 警告のみ: device name 不一致、connection type 不一致、layer id 不一致、selected physical layout name 不一致。
      - layer name は警告だが強く扱う。名前集合が同じで順序だけ違う場合は layer_order_mismatch、名前自体が違う場合は
        layer_name_mismatch として分ける。

      - selected_physical_layout_name の違いだけでは中断しない。position 集合が一致しない場合だけ中断する。
      - 現在値と raw 値が同じ binding は unchanged_skipped にして書かない。

  - behavior 名検証:
      - backup に出現する behavior_id 群を対象 device へ直接問い合わせる。現在 snapshot の使用済み binding から再構築
        しない。

      - USB/BLE を問わず、preview 時に可能なら get_behavior_details(id) 相当で対象 device の behavior_id -> name を取
        得する。

      - 成功時は behavior_verification = "done" とし、backup 名と対象名を比較する。
      - timeout、unsupported、切断、LayoutOnly などで取得不能な場合は behavior_verification = "skipped" とする。
      - backup の behavior_catalog が空、または backup 側 behavior 名が全体的に "behavior {id}" プレースホルダしかない
        場合は、per-key の unverified で全件潰さず、全体を behavior_verification = "skipped" として raw 復元対象にす
        る。

      - behavior_verification = "done" の場合だけ、対象に behavior_id がない、backup 名が未解決、backup/target 名が一
        致しない binding を missing / unverified / conflicts として書き込み対象から除外する。

      - behavior 名比較は trim、ASCII lowercase、_/-/space の区切り正規化を行う。例: mod-tap, mod_tap, mod tap は同一
        扱い。

  - StudioEditSession に bulk raw 書き込みを追加する。
      - apply_raw_writes(&mut self, writes: &[RawBindingWrite]) -> Result<StudioKeymapSnapshot, StudioError>
      - 各 write は client.set_key_at(layer_id, position, Behavior::Unknown { behavior_id, param1, param2 })。
      - layer id は backup 側ではなく、現在 device の同じ layer index の id を使う。
      - snapshot() は全 write 後に1回だけ呼ぶ。

  - 既存の set_binding, EditBehavior, behavior_to_zmk, 通常編集UI、読み取り経路は変更しない。

  ## Tauri/API/UI

  - Tauri command を追加する。
      - studio_export_keymap(device_id, path) -> Result<(), String>
      - studio_preview_keymap_restore(device_id, path) -> Result<RestoreReport, String>
      - studio_apply_keymap_restore(device_id, path) -> Result<(StudioKeymapSnapshot, RestoreReport), String>

  - command 動作:
      - export は同一 device の edit session があればその snapshot を使う。
      - 他 device の edit session がある場合は port_busy を返す。
      - edit session がない場合だけ read_keymap_for_device で fresh read する。
      - preview はファイルを読み、現在 session の snapshot と比較し、書き込みなしで RestoreReport を返す。
      - apply はファイルを再読込し、preview 相当の検証を再実行してから bulk raw write する。
      - apply 後も保存しない。dirty 状態として既存 EditBar に合流する。

  - ファイルI/O:
      - フロントエンドに tauri-plugin-fs は追加しない。
      - capabilities/default.json は dialog:allow-save だけ追加する。
      - ファイル読書きは Rust 側 std::fs で行う。
      - command 側で metadata による 1 MiB 上限を読み込み前に確認し、parse 側でも text.len() で防衛的に再確認する。
      - 読み込みは通常ファイルのみ許可。書き込みは親ディレクトリ存在を確認する。
      - path と JSON 内容はログに出さない。

  - UI:
      - Keymap Viewer の keymap 操作行に Export / Restore を追加する。icon は lucide Download / Upload。
      - Export は閲覧中・編集中どちらでも可能。ただし読み取り中、pending write 中、保存/破棄/終了中は無効。
      - Restore は選択 device が available/unlocked のとき有効。未編集中なら内部で edit session を開始する。
      - Restore 手順は open dialog -> session確保 -> preview -> confirm -> apply。
      - 既に未保存変更がある場合は「現在の未保存変更を破棄して読み込む」確認を出し、同意時だけ force discard して続行
        する。

      - confirm には backup 元名、日時、書き込み件数、変更なし件数、復元不可件数、警告を表示する。
      - behavior_verification = "skipped" の場合は強警告を出す: 「この接続では behavior 名を検証できないため、別FWへの
        復元では誤った割り当てになる可能性があります。USB接続で検証できる場合があります。」

      - layer_order_mismatch は強警告を出す: 「レイヤーの並びが一致しません。別スロットへ復元され、MO/LT などのレイ
        ヤー参照が意図とずれる可能性があります。」

      - apply 成功後は既存 EditBar を表示し、「保存するとキーボードへ永続化されます」と分かる文言にする。
      - 復元不可キーがある場合は apply 後も Notice tone="warn" で件数と概要を残す。

  - i18n 追加キー:
      - keymap.export, keymap.restore, keymap.export.done, keymap.restore.done
      - keymap.restore.discard_confirm, keymap.restore.summary
      - keymap.restore.verify_skipped, keymap.restore.partial
      - keymap.restore.layer_order_mismatch, keymap.restore.layer_name_mismatch
      - keymap.restore.device_mismatch, keymap.restore.layout_name_mismatch
      - keymap.restore.abort.layer_count, keymap.restore.abort.position_count, keymap.restore.abort.position_set
      - keymap.error.keymap_invalid_file, keymap.error.keymap_unsupported_version, keymap.error.keymap_file_too_large,
        keymap.error.keymap_invalid_path, keymap.error.restore_structure_mismatch

  ## Security

  - この機能により「ユーザーが選択した任意 JSON を読む」という攻撃面は増える。
  - そのリスクを次で制限する。
      - 汎用 fs 権限を追加しない。
      - Rust 側で通常ファイル、サイズ上限、schema/version、serde 型、構造一致を検証する。
      - JSON 内容はコード実行・パス展開・コマンド実行に使わない。
      - 書き込み対象は ZMK Studio の key binding のみ。
      - behavior 名検証が可能な場合は behavior_id の意味不一致を検出し、該当キーを書かない。
      - behavior 名検証ができない場合は強警告を出し、ユーザー確認後だけ raw を書く。
      - path と JSON 内容をログに出さない。

  - tauri.conf.json の csp: null は既存リスクとして別タスクで扱う。本機能の実装範囲では変更しない。

  ## Docs

  - README.md, docs/manual-app-usage.md, docs/spec.md, docs/compatibility.md を更新する。
  - 明記する内容:
      - Studio 保存は firmware の .keymap ソースを変更しない。
      - FW焼き直しや settings reset で戻る場合がある。
      - この機能は NVS 運用のバックアップ/復元であり、.keymap 生成・ソース反映は対象外。
      - Restore 後は未保存変更なので、永続化には 保存 が必要。
      - 構造が合わない場合は書き込まない。
      - behavior 名検証ができない場合は強警告を出し、同一FW/同一構成への復元を前提にする。
      - BLE 由来バックアップも raw 復元対象になるが、behavior 名検証ができない場合は USB より安全確認が弱い。

  ## Test Plan

  - Core unit tests:
      - snapshot から backup JSON を生成し、parse 後も raw binding が保持される。
      - unknown/custom behavior を Behavior::Unknown で復元計画に含められる。
      - BLE由来 backup のように behavior_catalog が空、かつ backup behavior 名がプレースホルダでも、全件 unverified
        no-op にならず behavior_verification = "skipped" で writes を作れる。

      - schema/version 不一致、JSON不正、型不一致、1 MiB超過を拒否する。
      - レイヤー数不一致、position 集合不一致、binding 数不一致で apply不可になる。
      - device/layout/layer name/id 不一致は警告になる。
      - layer name 集合が同じで順序だけ違う場合は layer_order_mismatch になる。
      - behavior missing/conflict/unverified は、検証済み時だけ書き込み対象から除外される。
      - behavior 名比較で mod-tap, mod_tap, mod tap が同一扱いになる。
      - 現在値と同じ binding は unchanged_skipped になる。
      - 1キーだけ違う場合は will_write == 1 になる。
      - RestorePlan.writes が UI向け RestoreReport に露出しない。

  - Tauri/API tests:
      - export が valid JSON を書く。
      - 同一 device の session があれば export は session snapshot を使う。
      - 他 device の session がある場合、export は port_busy を返す。
      - preview は device へ書き込まない。
      - apply は preview を再実行し、構造不一致時は書き込まない。
      - active session の device mismatch、locked、port busy、read/write失敗が既存 error code 体系で返る。
      - 読み込み前 metadata サイズ検証と parse 側サイズ検証が両方効く。

  - UI/manual QA:
      - Export で -keymap.json を作成できる。
      - 無変更 round trip は書き込み0件になる。
      - 数キー変更後に Restore すると該当キーだけ未保存変更として戻る。
      - Restore 後に 変更を破棄 で元へ戻せる。
      - layer order/name、device name、layout name 不一致は警告として表示される。
      - behavior conflict/missing は該当キーを書かず、UIに件数と概要が出る。
      - BLE由来 backup を復元しても全件 no-op にならず、検証スキップ警告が出る。
      - USB Serial と BLE Studio の両方で既存の読み取り、編集、保存、破棄が壊れていない。

  - 既存回帰:
      - cargo test --workspace --no-default-features
      - cmd.exe /C npm --prefix ui run build

  ## Assumptions

  - この機能は .keymap ソース生成ではなく、ZMK Studio/NVS 状態のバックアップ復元。
  - 復元は未保存変更として反映し、即保存しない。
  - 構造一致は layer 数と各 layer の position 集合を必須条件とする。
  - device name、layout name、layer name、layer id は警告扱いで、復元可否の必須条件にはしない。
  - raw binding 復元には Behavior::Unknown を使う。
  - 新機能なのでリリース時は MINOR バージョン更新を検討する。