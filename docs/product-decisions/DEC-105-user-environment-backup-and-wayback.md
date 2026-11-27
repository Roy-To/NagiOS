# DEC-105 — ユーザー環境のバックアップとWaybackの分担・連携

**状態:** CONFIRMED（B方式を基本、4機能の分担、ユーザー環境の自動差分バックアップ・選択式復元、独立Profile移行、保存対象・保存先・復元の安全方針、必要に応じたC方式の追加選択肢）／PROVISIONAL（保存・復元契約、技術・運用・提供版）
**最終合意日:** 2026-11-25
**適用候補:** Nagi OS Foundation Checklist #16 Backup / Restore。提供版は未確定
**関連:** [DEC-004](DEC-004-memory-learning-forgetting.md)、[DEC-013](DEC-013-lightweight-memory-engine-benchmark.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-053](DEC-053-ai-task-recovery-preview.md)、[DEC-079](DEC-079-system-image-readonly-ab-data-separation.md)、[DEC-086](DEC-086-nagi-secrets-service-hybrid-key-protection.md)、[DEC-117](DEC-117-user-data-encryption-and-lost-device.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[ロードマップ索引](roadmaps/README.md)、[0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)

## 目的とユーザー体験

日常の変更から戻るためのWaybackと、ユーザー環境を保護・復元するバックアップを組み合わせる。Workspaceの画面・作業状態保存だけで、本文やユーザー環境全体を保護できたと扱わない。

## 確定した仕様（CONFIRMED）

- **B方式を基本とする:** #16の基本方針は「ユーザー環境全体のバックアップ」とする。「全体」の範囲は下記の保存方針に従い、全ディスク・全状態の完全復元保証を意味しない。
- **Waybackと分担・連携する:** ユーザー環境のバックアップ／復元と、Waybackが対応する変更・Checkpoint・実在する逆操作の復元を補完的に扱う。バックアップをWaybackの代替とせず、Waybackだけでユーザー環境全体のバックアップが成立したとも扱わない。連携の具体方式は未確定。
- **必要に応じてC方式も選択可能とする方向:** C方式はB方式に追加する選択肢として扱い、基本方式や初期必須機能に置き換えない。C方式の具体的な内容・対象・採用条件・提供時期は未確定であり、ディスクイメージや特定snapshot技術の採用をこの合意から推定しない。

### F16の分担と復元体験（2026-11-25／CONFIRMED）

| 機能 | 役割 |
|---|---|
| Wayback | 日常の変更を、利用可能なCheckpointや実際に実行できる逆操作で戻す。送信・アップロードなど外部に生じた作用は、ローカル状態の復元では取り消せない。 |
| Nagi Backup | ユーザー環境を自動で差分バックアップする。故障やPC買い替え時に環境を復元でき、必要なファイル・Profile・アプリデータ等だけを選ぶ部分復元にも対応する。 |
| Nagi Profile Export / Import | Memory・学習済みの習慣・Persona・個人設定を含むProfileを、ユーザー環境全体のBackupやWorkspace移行から独立して別端末へ移す。 |
| System Recovery | 起動不能・更新失敗などからSystem Imageを復旧する。DEC-079のA/B更新・復旧とUser Dataの分離に従い、ユーザー環境の復元にはBackupを用いる。 |

バックアップは外付けストレージなどの外部媒体またはNASへの保存を基本とする。クラウド保存は任意で、別途明示的な許可を要する。世代管理と容量上限を設ける。

### 保存対象の方針（2026-11-25／CONFIRMED）

| データ | 取扱い |
|---|---|
| ユーザーファイル、Nagi Memory・学習済みの習慣、Persona・個人設定、Workspace・Project、アプリ設定・保存データ、自作アプリ・開発ソース | Nagi Backupの標準対象。既存の所有・機密区分・権限境界に従う。 |
| Secret Memory | 本人が明示的に選んだ場合に限り、暗号化してBackupまたはProfile移行に含める。復元後も本人専用の利用制限を適用する。 |
| Wayback履歴、Albertの閲覧履歴、LLMモデル本体 | Backupへの収録を選択できる。対応するモデルは再取得も可能とする。 |
| 検索インデックス、一時キャッシュ | 検索インデックスは原則として復元先で再構築する。一時キャッシュは対象外とする。 |
| 端末固有のドライバー | 原則として対象外とし、移行先の端末に合わせる。 |
| 認証情報・秘密鍵 | 通常のデータ復元から切り離し、個別の安全な移行手続きで扱う。 |

## 既存決定の継承・安全境界

- **DEC-014:** Workspaceの状態保存・暗号化Export／Import・選択式移行は維持する。Workspace snapshotは本文バックアップではない。B方式の採用によってWorkspaceの保存対象へ本文・アプリ等を自動追加しない。
- **DEC-026／053:** AI Task開始前のWayback復元可能性表示と、実行後の検証を維持する。バックアップの存在だけで復元可能と断言しない。外部送信や遠隔の副作用などに完全Undoを約束せず、部分復元・失敗・不明を正確に示す。
- **DEC-004／013:** Backupの復元、ProfileのImport、Waybackからの復元で、明示的に忘却した情報を検索・提案・Context構築・自動再学習に復活させない。忘却指定の状態も保護し、復元時に利用可能な最新の忘却状態を照合する。古いバックアップ単体では、作成後の忘却指定を反映できず、最新の忘却状態の維持を保証できない。Memoryの削除と、別管理のActivity／Wayback／バックアップの削除範囲を区別する。
- 現行User／Profile／Owner／Permission／Privacy、Source有効性、Revision・競合と既存の機密分類・外部送信許可を尊重する。復元やImportで旧権限を再付与せず、Secretの利用制限を緩めず、中断AI Task・副作用Actionを自動再開／再実行しない。
- **DEC-086／117:** 既存の鍵保護とMemory／Profileの常時暗号化を継承する。復号後の利用にも現行の権限と機密区分を適用する。
- DEC-079のSystem ImageのA/B更新・復旧とUser Data分離を維持する。System rollbackとユーザー環境のバックアップ／復元は別の保護範囲である。

### 8/12 Storage・Securityの保存・移行境界（2026-11-25／CONFIRMED）

[DEC-088のB案](DEC-088-linux-filesystem-nagi-storage-api.md#812-storagesecurity-b案2026-11-25)に沿って、Profile Export／Importは暗号化し、モデル非依存のMemory・習慣・Persona・個人設定と、由来・忘却・保護状態を引き継ぐ。モデル本体と派生索引は原則として移行先で再取得・再構築する。Secret Memoryの収録は本人の明示選択を条件とし、認証情報・秘密鍵は個別の安全な移行手続きで扱う。

保存・復元の結果は各正本の保存証拠・整合性を再検証し、成功・部分成功・失敗・結果不明を区別する（[DEC-090](DEC-090-common-persistence-contract-distributed-authority.md)／[DEC-091](DEC-091-nagi-recovery-coordinator-service-owned-recovery.md)）。復元・Import時は利用可能な最新の忘却状態、現行の権限・機密分類・Source有効性とRevision・競合を再確認し、新しいデータを黙って上書きしない。最新の忘却状態が不明な場合は安全側に扱い、隔離・保留の具体手順は下記のPROVISIONALに従う。

## 未確定事項（PROVISIONAL / OPEN）

- データ種別ごとの保存・復元契約。対象の粒度、形式・バージョン・参照関係、アプリ本体の再導入やデータ互換性、整合性検証、移行先での競合・既存データ保護・中断復旧、部分復元／復元不能の扱い。
- C方式の具体的内容、対象、必要とする場面、選択条件・UI・実装方式。必要時の追加選択肢という方向以上は確定しない。
- 初回保存・差分・重複排除の実装、Filesystem・Snapshot技術、WaybackとBackupの連携方式、保存先ごとの接続と認可、世代数・保持期間・容量上限値・整理条件、暗号方式・鍵管理・個別移行・鍵喪失時の回復。
- 忘却状態の保存・保護・照合方法と、状態を喪失した場合や最新性を確認できない場合の安全側の処理。照合できないMemoryを利用可能にする前に保留・隔離する案や、旧世代内の残存データを削除する方法・限界は未確定。
- 設定画面、復元前プレビューと結果表示、手動／自動実行の導線、アイドル時等の実行条件・頻度、Job Scheduler／Automation Modeとの連携。
- 提供版・時期・担当・実装順序・性能値・Acceptance。全ディスク復元、bare-metal restore、全機種間移行の保証は確定しない。

## Roadmapとの関連

0.3ロードマップのProfile単位BackupをOS全体Backupから分離可能とする構造、0.4+のWaybackと不可逆作用の区別、0.5–1.0の実際に回復できる範囲に基づく復旧方針を維持する。本決定はユーザー環境の製品方針を補足するもので、Profile単位Backupを廃止しない。特定の版にB／Cを割り当てず、Release／Activation／Acceptance Gate、担当・着手条件を変更しない。仕様の記録であり、実装・Linux image／実機のバックアップ・復元成功を示さない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーが#16のB方式（ユーザー環境全体のバックアップ）＋Wayback連携＋必要に応じたC方式を承認。基本方針と追加選択肢の方向をCONFIRMED、具体的な対象・技術・Cの詳細・版を未確定として分離した。 |
| 2026-11-25 | F16の4機能の分担、自動差分バックアップ・選択式復元・独立Profile移行、保存対象・保存先・世代管理・容量上限・復元時の安全方針をCONFIRMED。保存・復元契約、暗号・鍵、忘却状態を確認できない場合の処理、UI・頻度・性能・提供版をPROVISIONALとした。 |
| 2026-11-25 | 8/12 Storage・Security B案として、モデル非依存の暗号化Profile移行と、保存・復元結果の再検証を明記。実装・技術候補はDEC-088、最新忘却状態が不明な場合の処理詳細は本DECのPROVISIONALを参照。 |
