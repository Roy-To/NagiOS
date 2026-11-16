# DEC-090 — 共通Persistence契約とデータ正本の分散保持

**状態:** CONFIRMED（Foundation #14 B案＝Nagi共通の永続化／移行／復旧契約を定め、User／Profile／Memory／Workspace／Task／App／Providerの正本を用途ごとに保持できる。保存完了・部分成功・結果不明の区別、版管理／安全なSchema Migration、所有・削除／忘却、Canonical Memoryと再生成可能な派生物の分離）／PROVISIONAL（保存先・DB／形式・transaction／journal／flush保証・schema migration手順・性能・Linux target受入／適用版）
**最終合意日:** 2026-11-16
**適用範囲:** Nagi OS Foundation Checklist #14「Data Persistence」（検討束B3）。Linuxカーネル＋Nagi専用ユーザーランドにおける永続データの論理正本・保存／移行／復旧の共通製品契約
**関連:** [DEC-003 Memory分類と権限](DEC-003-memory-classification.md)、[DEC-004 Memoryの学習・忘却](DEC-004-memory-learning-forgetting.md)、[DEC-013 Model-independent Canonical Memory](DEC-013-lightweight-memory-engine-benchmark.md)、[DEC-014 Workspace状態・Checkpoint](DEC-014-workspace-state-checkpoints.md)、[DEC-022 中断Taskの復旧](DEC-022-builder-interrupted-task-recovery.md)、[DEC-023 作業・データ中心](DEC-023-work-data-centric-os.md)、[DEC-025 分散正本・共通参照](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-079 System Image／User Data分離](DEC-079-system-image-readonly-ab-data-separation.md)、[DEC-088 Linux Filesystem＋Nagi Storage API](DEC-088-linux-filesystem-nagi-storage-api.md)、[DEC-089 Volume Manager](DEC-089-nagi-volume-manager-linux-mount-hotplug.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[0.2 Platform Foundation仕様 §23–24・45–46](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[純正アプリ実装仕様 §87–92](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[Roadmaps](roadmaps/README.md)

## 目的

再起動・サービス再起動・電源断・OS／アプリ更新や将来の端末移行で、Nagiの利用者データ・学習Memory・Workspace・Task／Activityなどを安全かつ正確に継続できるよう、共通の保存・復旧ルールを定める。一方、元アプリ・Provider・利用者の保存先が持つデータの正本は維持し、あらゆるデータをNagi共通DBに強制集約しない。

## 確定した製品方針（CONFIRMED）

1. **B案：共通Persistence契約＋用途別・分散保存。** Nagiが永続化の所有／Identity・Version／Revision・保存結果・Schema Migration・復旧・削除／忘却等に関する共通の製品契約を提供する。User／Profile設定、Canonical Memory、Workspace、Task／Activity、App／Providerの文書等はそれぞれ適切な正本を保持でき、単一共通DB・単一ファイル形式に強制統合しない。
2. **「保存完了」と実際の耐久性を区別する。** メモリ上の更新・一時キュー受付・アプリ内操作完了は、永続媒体への安全な保存が確認されたことと同一視しない。保存済み・未保存・部分成功・失敗・結果不明を実際の契約と確認可能な証拠に応じて区別し、ユーザーに虚偽の成功を表示しない。厳密なcommit／flush／fsync等の保証範囲は後続技術設計で定める。
3. **保存SchemaをVersion管理し安全に移行する。** 変更される永続データのSchemaはVersionと互換／Migrationの契約を持たせ、更新でUser Dataを無断破棄・黙って解釈変更しない。Migration不能・破損・競合等は失敗／保留として明示し、回復できる範囲を正確に示す。
4. **User／Profile／App／Resourceの所有・現行権限・Revisionを維持する。** Nagiの論理Identity・Permission／Capability／PolicyとResource／Document／Object／Revision／Provenanceを保存・取得・復旧でも評価する。Task／Workspaceとの関連付けや保存レコードの存在によって、元Providerや別Profileへのアクセス権限が自動的に増えるわけではない。
5. **Canonical Memoryと再生成可能な派生データを分離する。** モデルから独立したユーザー所有のCanonical Memory／Profile・学習状態、由来、保持／忘却の状態を正本として扱い、検索索引・キャッシュ・Context Adapterなど必要時に再生成できる派生物と区別する。モデルや端末を変更した際に、正本から安全に再構築できる設計を維持する。自然忘却・明示的忘却の既存規則を上書きしない。
6. **Workspace復元・本文バックアップ・Task再実行を区別する。** Workspaceの自動保存／Checkpointと、ファイル本文やアプリの正本・外部サービスへの副作用は同一ではない。保存や復旧で中断Task／副作用Actionを無断再実行せず、結果不明を照合し、現在のUser／Profile／Permission／Resource Revision等を再評価する。
7. **削除・忘却・Retentionの既存ルールを維持する。** 明示的に忘れるよう指示されたMemoryを再び索引・キャッシュや過去の履歴から無断で復活させない。通常の自動保存履歴の整理と、保護されたWorkspace／名前付きCheckpoint／正本の削除は区別する。Backup／Wayback／Cloudなど別管理範囲の完全消去を、実行・検証なしに保証しない。

## 既存正本との境界

- **DEC-023／025:** 元アプリ・Provider・利用者のデータ正本が分散する方針と、Nagiの共通参照／正規APIを維持する。共通Persistence契約の導入は「Nagiが全アプリの保存ファイル本文を所有する」「全アプリが共通DBに書く」ことを意味しない。
- **DEC-013／003／004:** Canonical MemoryはLLMや索引から独立してUser／Profile・Provenance・Permission・忘却ルールを保持する。自然忘却の暫定日数や閾値を今回確定しない。
- **DEC-014:** Workspaceの自動保存・名前付きCheckpoint・復元前の保護／検索等は既存決定通り。Workspace履歴とファイル本文の完全Backup／全App完全復元を同一視しない。
- **DEC-022:** クラッシュ後のTask再開／やり直し／中止はユーザー選択と現行権限の再検証、二重実行の抑制が必要。保存形式を定めただけで中断Taskが安全に再実行できるとしない。
- **DEC-079／088／089:** 通常時read-only System／User Data分離、用途別Linux FS、Nagi Storage／Files API、Volume Manager／安全な取り外しの高水準方針を維持。FSの選択・snapshot・mount・flushはNagi Persistenceの論理契約とは別途検証する。
- **0.2仕様 §23–24・45–46／純正仕様 §87–92:** Resource・Revision等の安定ID、データSchemaのversion／migrationと互換性試験、Activity／Transaction／Waybackの非偽装・非一律ACID、クラッシュ／破損・不明結果・競合の安全な扱いを継承する。
- **Linux移行・Release:** 旧kernel・host-onlyの保存／検索／アプリテストをLinux imageのcrash durabilityやmigrationのAcceptance PASSへ読み替えない。既存Workstream／Release・Activation／Acceptance Gate、機種・版・提供時期を変更しない。

## 未確定・暫定事項（PROVISIONAL／OPEN）

- 正本の実際の所在と所有責務、用途別DB／永続化ファイル形式、SQLite等の具体製品、データ暗号化／鍵・アクセス境界、共通Persistence Serviceのプロセス配置／API。
- 「durable commit」の技術的定義、fsync／flush／journal／atomic rename・Transaction境界、複数Provider間でのpartial commit・再送や重複防止、結果不明からの照合手順。
- Schema VersionとMigration互換matrix、失敗時Rollback／Backupの具体手順、旧版と新版間の二方向互換範囲・データ破損時の扱い。
- 正本／派生Index・Cache／Activity／Checkpointの分担、Retention・容量上限・Storage不足時の処理、削除／忘却を復旧・Backupへ反映する具体境界。
- Linux target／実媒体でのpower loss／crash耐久性、複数AppやProviderを横断する保存・復旧・権限拒否試験、性能・Release割当て。

## 技術設計への引渡し（REVIEW NOTE）

用途別の「保存正本表」を作成してUser／Profile／App／Canonical Memory／Workspace／Task・Activity／Search Index／External Providerごとに所有者、Storage、Version、Revision、Retention／Forget、Migration、Failure／Recoveryを明確化する。Linux targetで書込み中の電源断・容量不足・壊れたSchema／Checkpoint・移行失敗、保存先不在、異なるProfileからのアクセス、Memory忘却後の再構築とTaskの二重実行拒否を個別に検証する。保証できないatomicity・復旧を成功として扱わない。

## Roadmapとの関連

B3 #14の高水準方針として、0.2 Resource／Activity／Wayback・Storage／App保存契約と、0.3 Profile／Canonical Memoryの将来継続を安全に接続する。具体的なStorage・DB採用・Runtime実装の完了や、0.3 Memory Engineの新Gate・新しい版割当てを意味しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #14「Data Persistence」のB案を承認。Nagiが共通Persistenceの所有・保存結果・Version／Schema Migration・復旧ルールを定め、正本は用途・アプリ・Provider別に保持できる。Canonical Memoryと派生Indexの分離、Workspace／本文／Taskの区別、データ所有・削除／忘却と不明な保存結果の正しい扱いを維持。具体Storage／DB・commit保証・migration技術・Linux受入・適用版は未決定。 |
