# DEC-091 — Nagi Recovery Coordinatorとサービス別責務の障害復旧

**状態:** CONFIRMED（Foundation #15 B案：Nagi共通のRecovery Coordinatorが復旧状況・整合性・ユーザー向け状態報告を統括し、各サービスが自身の保存正本・状態の安全な復旧を担当する。結果不明Actionの無断再実行禁止、現行権限／Revision再評価、通常Sleep復帰とCrash／電源断後復旧の区別）／PROVISIONAL（Coordinator構成・Journal／Checkpoint方式・状態機械／復旧順序・再試行条件・Linux target受入・適用版）
**最終合意日:** 2026-11-16
**適用範囲:** Nagi OS Foundation Checklist #15「Crash Recovery」（検討束B3）。クラッシュ／電源断／サービス異常終了・再起動後の安全な復旧
**関連:** [DEC-014 Workspace保存・保護](DEC-014-workspace-state-checkpoints.md)、[DEC-022 Builder中断Task復旧](DEC-022-builder-interrupted-task-recovery.md)、[DEC-068 Nagi Sleep](DEC-068-nagi-sleep-resume-recovery.md)、[DEC-079 System Image／A/B](DEC-079-system-image-readonly-ab-data-separation.md)、[DEC-088 Linux Filesystem＋Nagi Storage](DEC-088-linux-filesystem-nagi-storage-api.md)、[DEC-089 Volume Manager](DEC-089-nagi-volume-manager-linux-mount-hotplug.md)、[DEC-090 共通Persistence契約](DEC-090-common-persistence-contract-distributed-authority.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[0.2仕様 §24／46](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[純正アプリ仕様 §90](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[Roadmaps](roadmaps/README.md)

## 目的

Nagiが突然停止しても、確認済みの保存状態を可能な範囲で安全に復旧し、復旧不能・不明な結果を隠さない。OS全体を機械的に以前の時点へ戻して新しいデータや外部副作用を無視するのではなく、共通の復旧調整と各データ所有サービスによる復旧を組み合わせる。

## 確定した製品方針（CONFIRMED）

1. **B案：Nagi共通のRecovery Coordinatorと各サービスの復旧責務を分離する。** Nagiが障害・再起動後の復旧状態や整合性確認・利用者への報告を横断的に調整し、Files／Storage、Workspace、Memory、Task／Activity、各アプリ／Provider等は自身が所有するデータ・状態の復旧と根拠確認を担う。
2. **保存済みの事実と復旧可能性を確認する。** Journal／Checkpoint／Revision／Activity等の利用可能な証拠と現行の永続化契約に基づいて、実際に検証できた状態だけを「復旧済み」と扱う。正常・部分成功・未保存・破損・結果不明・復旧不能を区別し、無根拠にすべてのアプリ状態やファイル本文を完全復元できると宣言しない。
3. **中断Taskや結果不明の副作用Actionを無断再実行しない。** DEC-022に従い、クラッシュ後のBuilder等の中断Taskは利用者に「続きから再開／最初からやり直す／中止」を提示できるが、ユーザーの選択前に副作用Actionを進めない。送信・書込みなどが成功したか分からない操作を、失敗とみなして自動再送しない。実行済み／未実行／結果不明の照合、二重実行防止を維持する。
4. **復旧・再開時には現行の権限とRevisionを再検証する。** 過去の認証・許可をそのまま引き継がず、User／Profile／Session、Permission／Policy、TaskのScope・取消／期限と対象Resource／Revision・競合を再確認する。復旧のためにAIやアプリへ特別なOwner／System権限を自動付与しない。
5. **復旧する内容ごとの境界を守る。** Workspaceの保存・復元は、本文ファイルのBackup、Applicationの全内部状態、System ImageのA/B rollback、外部サービスへの副作用、AI Taskの再実行と別である。復旧処理中も元ファイルや新しいユーザー変更を無断上書きしない。復元前のWorkspace保護やPermission／Privacyを維持する。
6. **通常のSleep復帰と障害復旧を区別する。** RAM保持からの通常復帰は電源断／クラッシュ後の保存状態の再構築とは異なる。DEC-068の通常復帰・接続／権限再検証を尊重し、障害後復旧を「停止していない継続」だと偽らず、Pause／CancelされたTaskを勝手に解除しない。
7. **Linuxの保存・復旧機能を活用できる構造とする。** DEC-088／089／090のLinux Filesystem・Nagi Storage／Files・Volume／Persistence契約を組み合わせ、用途ごとのJournal／Checkpoint等の適切な仕組みを評価できる設計とする。単一のFilesystem snapshotやOS全体の一括RollbackがこのB案の必要条件ではない。

## 例外・既存決定との整合

- **DEC-014／022:** 名前付きWorkspace Checkpoint、復元前保護、中断Taskの再開／やり直し／中止、現在の認可・対象Revisionの再検証、外部副作用の二重実行防止を変更しない。
- **DEC-068:** 通常スリープ復帰とクラッシュ・電源断後の再構築の区別を維持する。
- **DEC-079:** A/B System rollback／RecoveryはSystem Imageの復旧であり、User Data／Memory／Workspace／外部Serviceの完全な巻戻しではない。
- **DEC-090:** Nagiの共通Persistence契約と用途・App／Provider別の分散正本を維持する。Recovery Coordinatorが全データの唯一の正本・単一DBになることを意味しない。
- **0.2 §24／46:** 復元時のPermission・Stale Revision・元状態保護・部分失敗と、duplicate/replay／crash/restart／corrupted durable stateの検証を尊重する。不可逆の外部副作用をUndoできると偽らない。
- **Release／実装状況:** 旧kernel／host-onlyでの復旧検証はLinux target／実媒体でのクラッシュ・電源断復旧PASSを意味しない。既存Release／Activation／Acceptance Gate、Workstream・担当・機種・提供版を今回変更しない。

## 未確定事項（PROVISIONAL／OPEN）

- Coordinatorのサービス配置・起動順と障害自身からの復旧、共通API・Recovery State Machine、保存証拠の照合・不明時の判断・UI。
- Journal／Checkpoint／Atomic commit／fsync・flush、Revision／Task Attempt／Action IDの実装上の対応、複数App／Provider間の部分成功と回復優先順位。
- 安全な再試行／再開・復旧不能表示の詳細条件、重複実行を避ける検証方法、権限失効・Volume切断・壊れたデータ／schemaの扱い。
- Crash・Power-loss・Service Restart・Sleepの実機／Linux target試験、復旧性能／保存範囲／受入と対象提供版。

## 技術設計への引渡し（REVIEW NOTE）

「保存完了直前／直後」「処理途中で電源断」「Service単独クラッシュ」「結果不明の外部副作用」「Volume切断」「権限失効」「壊れたJournal／Checkpoint」「古いResource Revision」で、保存証拠・復旧結果・User選択と二重実行防止を独立に検証する。Serviceが実際に復旧できた内容とCoordinatorの報告が一致するかを確かめ、Linux target・実媒体・host-onlyを区別して記録する。

## Roadmapとの関連

B3 #15の高水準の製品方針であり、#12〜#14のStorage・Volume・Persistenceと接続する。具体的復旧実装・Linux上の受入、0.2／0.3以降のRelease／Activation Gate変更、新たな全体停止条件の採用を意味しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #15「Crash Recovery」のB案を明確に承認。Nagi Recovery Coordinatorによる共通調整とサービス所有データの個別復旧、安全な状態・不明結果の区別、結果不明Action／中断Taskの無断再実行禁止、現行権限・Revision再評価、通常Sleep復帰との分離を確定。Journal／Checkpoint・State Machine・実装／Linux受入・提供版は未決定。 |
