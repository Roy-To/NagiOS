# DEC-082 — ユーザー環境のバックアップとWaybackの分担・連携

**状態:** CONFIRMED（B方式を基本、Waybackとの分担・連携、必要に応じたC方式の追加選択肢）／PROVISIONAL（詳細・技術・適用版）
**最終合意日:** 2026-11-16
**適用候補:** Nagi OS Foundation Checklist #16 Backup / Restore。提供版は未確定
**関連:** [DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-004](DEC-004-memory-learning-forgetting.md)、[DEC-053](DEC-053-ai-task-recovery-preview.md)、[DEC-079](DEC-079-system-image-readonly-ab-data-separation.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[roadmaps](roadmaps/README.md)

## 目的とユーザー体験

日常の変更から戻るためのWaybackと、ユーザー環境を保護・復元するバックアップを組み合わせる。Workspaceの画面・作業状態保存だけで、本文やユーザー環境全体を保護できたと扱わない。

## 確定した仕様（CONFIRMED）

- **B方式を基本とする:** #16の基本方針は「ユーザー環境全体のバックアップ」とする。Workspace移行だけに限定しない。個々のデータ種別の収録・除外・復元契約は下記の未確定事項であり、「全体」は全ディスク・全状態の完全復元保証を意味しない。
- **Waybackと分担・連携する:** ユーザー環境のバックアップ／復元と、Waybackが対応する変更・Checkpoint・実在する逆操作の復元を補完的に扱う。バックアップをWaybackの代替とせず、Waybackだけでユーザー環境全体のバックアップが成立したとも扱わない。連携の具体方式は未確定。
- **必要に応じてC方式も選択可能とする方向:** C方式はB方式に追加する選択肢として扱い、基本方式や初期必須機能に置き換えない。C方式の具体的な内容・対象・採用条件・提供時期は未確定であり、ディスクイメージや特定snapshot技術の採用をこの合意から推定しない。

## 既存決定の継承・安全境界

- **DEC-014:** Workspaceの状態保存・暗号化Export／Import・選択式移行は維持する。Workspace snapshotは本文バックアップではない。B方式の採用によってWorkspaceの保存対象へ本文・アプリ等を自動追加しない。
- **DEC-026／053:** AI Task開始前のWayback復元可能性表示と、実行後の検証を維持する。バックアップの存在だけで復元可能と断言しない。外部送信や遠隔の副作用などに完全Undoを約束せず、部分復元・失敗・不明を正確に示す。
- **DEC-004:** 明示的に忘却したMemoryを古い履歴・バックアップから自動再学習しない。Memoryの削除と、別管理のActivity／Wayback／バックアップの削除範囲を区別する。復元後の忘却状態維持の具体方式は未確定。
- 現行User／Profile／Owner／Permission／Privacy、Source有効性、Revision・競合と既存の機密分類・外部送信許可を尊重する。復元は古い権限の再付与や、中断AI Task・副作用Actionの自動再開／再実行の許可ではない。
- DEC-079のSystem ImageのA/B更新・復旧とUser Data分離を維持する。System rollbackとユーザー環境のバックアップ／復元は別の保護範囲である。

## 未確定事項（PROVISIONAL / OPEN）

- ファイル本文、設定、Profile／Memory、Workspace、アプリ本体・アプリデータ、端末固有情報、認証・鍵関連情報の具体的な収録／除外範囲、整合性と復元単位。
- C方式の具体的内容、対象、必要とする場面、選択条件・UI・実装方式。必要時の追加選択肢という方向以上は確定しない。
- Wayback履歴をバックアップへ含めるか、相関・世代・保持・容量・削除の管理、全体復元と部分復元の導線。
- 保存先・外部媒体・ネットワーク／クラウド経路、手動／定期実行、完全／差分等の方式、filesystem・snapshot技術、暗号化・鍵管理・鍵喪失時の回復。
- 復元前プレビュー、既存データ保護、互換性・検証・中断復旧・競合・忘却状態の維持、復元不能／部分復元の具体的処理。
- 提供版・時期・担当・実装順序・性能値・Acceptance。全ディスク復元、bare-metal restore、全機種間移行の保証は確定しない。

## Roadmapとの関連

0.3ロードマップのProfile単位BackupをOS全体Backupから分離可能とする構造、0.4+のWaybackと不可逆作用の区別、0.5–1.0の実際に回復できる範囲に基づく復旧方針を維持する。本決定はユーザー環境の製品方針を補足するもので、Profile単位Backupを廃止しない。特定の版にB／Cを割り当てず、Release／Activation／Acceptance Gate、担当・着手条件を変更しない。仕様の記録であり、実装・Linux image／実機のバックアップ・復元成功を示さない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーが#16のB方式（ユーザー環境全体のバックアップ）＋Wayback連携＋必要に応じたC方式を承認。基本方針と追加選択肢の方向をCONFIRMED、具体的な対象・技術・Cの詳細・版を未確定として分離した。 |
