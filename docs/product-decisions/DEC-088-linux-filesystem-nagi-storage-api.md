# DEC-088 — 用途別Linux FilesystemとNagi Storage／Files APIの分離

**状態:** CONFIRMED（B案＝Linux標準Filesystemを保存用途別に選択活用、Nagi独自Storage／Files APIとResource／Objectの論理Identityを維持、snapshot／Wayback／Backup／Workspace復元の役割を区別）／PROVISIONAL（具体FS・mount／暗号化・durability／atomicity・Resource binding実装・Linux target受入／適用版）
**最終合意日:** 2026-11-25（8/12 Storage・Security B案を追加）
**適用範囲:** Nagi OS Foundation Checklist #12「Storage / Filesystem」（検討束B3）。Linuxカーネル＋Nagi専用ユーザーランドのファイル保存下層とNagi固有の論理Storage／Files契約
**関連:** [DEC-014 Workspace保存・移行](DEC-014-workspace-state-checkpoints.md)、[DEC-023 作業・データ中心OS](DEC-023-work-data-centric-os.md)、[DEC-025 分散正本・共通参照](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-043 Linuxカーネル＋Nagi userland](DEC-043-nagi-userland-independent-os.md)、[DEC-079 System Image／User Data分離](DEC-079-system-image-readonly-ab-data-separation.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[0.1仕様 §17–18](../Nagi_OS_0.1_Codex_Implementation_Spec.md)、[0.2仕様 §23–24](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[Roadmaps](roadmaps/README.md)

## 目的・利用者体験

Nagi固有のFile／Resource／Workspaceとアプリ横断の参照・権限管理を維持しながら、Linuxカーネルが提供する実績あるFilesystemを用途ごとに選択活用する。Nagi独自Filesystemの新規開発や単一Filesystemへの全面統一を基本条件とせず、データ保存の信頼性・既存資産との親和性・実装速度を重視する。

## 確定した製品方針（CONFIRMED）

1. **B案：Linux標準Filesystemを用途別に活用する。** System Image、User Data、AI Model Store、一時保存など異なる保存用途について、適切なLinux標準Filesystem／保存機構を比較して選択できる構造とする。全領域を同じFilesystemに統一することや、Nagi専用Filesystemの新規開発を初期の必須条件としない。
2. **Nagi独自Storage／Files Service／APIを維持する。** Files・App・AI・Public Action等が使う上位のStorage／Files契約、Nagi Permission／Capability／Policy、User／Profile／Resourceの所有・参照境界をNagiの正規サービスとして維持する。Filesystemや物理媒体の違いをNagiの公開APIへ無条件に漏らさない。
3. **論理Resource／Object IDとLinuxのpath／inodeを区別する。** Resource／Document／Object／Revision／Provenance等のプラットフォーム契約を保持し、pathやinodeを永続的なユーザー向けIdentityと同一視しない。rename／move／再起動／Provider変更等で保持できる同一性の範囲は別途検証し、維持不能な場合に黙って同一と扱わない。
4. **保存用途を既存System Image／データ分離方針に整合させる。** DEC-079の通常時read-onlyのOS本体、User Data分離と更新後保持、A/B更新／Recovery、AI Model Storeの論理分離を維持する。用途区分は物理partitionの固定数、単一SSD上の正確な配置・format・mount先を今回確定するものではない。
5. **Filesystemのsnapshot、Nagi Wayback、Backup、Workspace復元を区別する。** filesystemのsnapshotだけでNagiの意味的なActivity／Checkpoint／Waybackや文書／Workspaceの復元・バックアップが完成したと扱わない。元Provider・アプリ／ユーザーのデータ正本はDEC-023／025の分散正本に従い、無断集約・別データの上書きや中断Taskの再実行を認めない。
6. **データ保全と障害時の正確な状態報告を重視する。** 容量不足・書込み失敗・電源断・データ破損・外部メディア切断等の障害を見据え、保存成功・部分成功・未保存・結果不明を区別する。実際に保証できないdurability／完全性／復旧成功を利用者へ示さず、保存・復旧のための正規API・実装／検証を整備する。

> **2026-11-16 Persistence後続決定:** [DEC-090](DEC-090-common-persistence-contract-distributed-authority.md)で、Nagi共通の永続化／Schema Migration／復旧契約を用いながら、データ正本をUser／Profile／Memory／Workspace／Task／App／Provider別に保持できるB案をCONFIRMED。本DECの用途別Linux FilesystemとNagi Storage／Files APIは不変。具体的なdurable commit／fsync／atomicity・Linux target受入は未決定。Filesystem snapshotがWaybackや全アプリBackupを代替することもない。

## 安全・実装境界

- **DEC-043／Linux移行:** Linux Filesystemの採用は既存distributionのuserland一式をOS基盤として採用する決定ではなく、Nagi独自のService／API／app環境を維持する。
- **0.1 §17–18の旧実装:** 旧独自kernelでのVFS／ext2初期実装、ESPのFAT32、NagiFS後続案、固定partition／mount案は歴史的な実装・設計資産であり、Linux版の製品Filesystemの正式採用・実装結果を意味しない。Nagi独自Filesystemをすぐ新設する必要性も今回確認されていない。
- **0.2 §23–24とDEC-023／025:** Resource／Document／Object／Revision／Provenanceの共通参照、Providerごとのデータ正本、現行Permission／Policy、Activity／Checkpoint／Waybackの意味的契約を維持する。Resource IDを知ること自体は閲覧・操作権限やRevisionの新しさを保証しない。
- **DEC-014:** Workspaceの状態保存・選択式復元／Export／Import・ファイル本文バックアップとの分離・復旧時の権限再評価を維持する。Filesystem snapshotを利用する場合もWorkspace／Task全体の完全復元を保証しない。
- **秘密保護:** データ暗号化・鍵の分離・Access ControlはFoundation #8／#10の製品方針との連携対象。#10のハイブリッドSecrets Serviceの決定は[PR #133](https://github.com/Roy-To/NagiOS/pull/133)のDEC-086に記録中（本DEC作成時点ではmain未マージ）。特定の暗号化Filesystem／暗号方式や鍵保護が本DECから決定したことにはならない。
- **#13との分離（2026-11-16後続決定）:** [DEC-089](DEC-089-nagi-volume-manager-linux-mount-hotplug.md)で、Linux block／mount／hotplug＋Nagi Volume Managerによる発見・権限・安全な取り外し／切断時状態管理のB案をCONFIRMED。#12のLinux用途別Filesystem選定・Nagi Storage／Files APIは維持する。接続検出と自動mount／全形式read-write対応は異なり、Volume ID・mount／eject具体方式・Linux target受入は引き続き未決定。
- **適用版・検証:** 既存Release／Activation／Acceptance Gate、担当・Linux移行開発権限、機種・提供版は変更しない。host-onlyや旧ext2テストをLinux image／実SSDでの耐障害検証PASSと読み替えない。

## 未確定事項（PROVISIONAL／OPEN）

- System／User Data／Model Store／Temp等の用途ごとに採る具体Filesystem。EROFS（read-only System）、ext4／Btrfs（User Data）等は比較候補であり正式採用ではない。AI Model Storeの実配置、容量・snapshot／reflinkの採否も未決定。
- block layer、storage driver／mount、partition／volume配置、対応媒体、暗号化、鍵管理、Quota／容量不足時の処理、fsck／Integrity検証、異常電源断・書込durabilityの契約。
- flush／fsync／atomicity・renameの保証範囲、metadata・Resource IDとLinux path／inodeのmapping、rename／move／backup／restore／storage交換時の生存条件、断線・結果不明の扱い。
- ユーザー向けsnapshot／Backup／Waybackの具体実装分担、データRetention・Restore／Rollback／ForgetのPolicy、対象Release・機器・移行、性能／Linux target受入。

## 技術設計への引渡し（REVIEW NOTE）

同一Linux targetにおいて、候補FSをSystem／User Data／Model Store等の実用途で比較し、read-only SystemとUser Data更新保持、ファイル作成・rename／move／再起動後のNagi Resource参照、User／Profile拒否と許可、容量不足・強制電源断・部分書込み・メタデータ破損の代表ケースを検証する。App／Providerの分散正本を尊重し、FilesystemのatomicityとNagiのTransaction／Wayback／Backupの保証を混同しない。測定した性能・容量・破損／復旧結果と未対応範囲を明示する。

## Roadmapとの関連

0.2のPlatform Resource／Files／Wayback・Storage契約と、0.3以降のProfile／Memory／Project・アプリ横断体験の保存下層を支える方針。具体的なFilesystem・版割当・受入条件を定める実装仕様やLinux target検証は別に実施し、既存Release Gateに新たな全体停止条件を作らない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #12「Storage / Filesystem」に対しB案を明確に承認。Linux標準Filesystemを用途別に活用し、Nagi独自Storage／Files APIと論理Resource IDを維持、FS snapshot・Wayback・Backup・Workspace復元を区別する。具体FS・暗号・耐障害方式・Linux受入・適用版は未決定。 |
| 2026-11-25 | 8/12 Storage・SecurityのB案を承認。用途別Linux FSと既存暗号技術を活用し、Nagiが上位の管理を担う。暗号化・鍵管理・Backup／移行・復旧の既存方針を継承し、評価候補・組合せ・実機検証・提供版はPROVISIONAL。 |

## 関連決定の参照注記（2026-11-21）

- [DEC-117](DEC-117-user-data-encryption-and-lost-device.md) — ユーザーデータの暗号化は既定で有効にし、MemoryとProfileは常時暗号化する。具体的な暗号方式とFilesystemとの連携は未決定。

## Linux 0.1 Files（2026-11-24・確定）

- Files の一覧は選択中のプロファイルに発行された利用者ファイルのルートに限る。Notes の保存領域とノートは Files に表示しない。Notes からコピーを書き出す操作は 0.2 の候補とする。
- ごみ箱から復元するとき、元の親フォルダが存在しなければ利用者ファイルのルートに戻す。同名項目を上書きしない。
- Linux の変更操作では親ディレクトリの fsync、上書きを防ぐ RENAME_NOREPLACE、ごみ箱の孤児回復を実装する。耐久性と再起動後の保持は Linux 受入で検証する。
- Linux 0.1 の Files の名前入力欄では、日本語のファイル名・フォルダ名を入力できない。

## 8/12 Storage・Security B案（2026-11-25）

### 製品方針（CONFIRMED）

- **用途別のLinux Filesystemと既存の暗号技術を活用する。** NagiはStorage／Filesの公開契約・論理Resource ID・所有と認可、[Volume Manager（DEC-089）](DEC-089-nagi-volume-manager-linux-mount-hotplug.md)による媒体管理、[共通Persistence契約と分散正本（DEC-090）](DEC-090-common-persistence-contract-distributed-authority.md)、[Recovery Coordinatorと各サービスによる正本の復旧（DEC-091）](DEC-091-nagi-recovery-coordinator-service-owned-recovery.md)を担う。独自Filesystemや独自暗号の開発を初期の必須条件にしない。
- **System／User Data／Model Storeの境界を維持する。** [DEC-079](DEC-079-system-image-readonly-ab-data-separation.md)の通常時read-only System、A/B更新・System Recovery、User Dataの分離、Model Storeの論理分離を継承する。
- **保存データを二層で保護する。** User Dataの暗号化は既定で有効とし、利用者が解除できる。Memory／ProfileはUser Data側の設定にかかわらず常時暗号化し、解除できない。[DEC-086](DEC-086-nagi-secrets-service-hybrid-key-protection.md)と[DEC-117](DEC-117-user-data-encryption-and-lost-device.md)に従い、User／Profileごとに鍵を分離し、Memoryと暗号化されたUser Dataに適用する。鍵はNagi Secrets Serviceが管理し、対応機器ではTPM／TEE等の保護を優先する。非対応機器ではパスフレーズで保護するローカル保管を用い、回復キーによる回復経路を設ける。回復キーのクラウド自動預託は行わない。
- **OSが認可と隔離を強制する。** [DEC-084](DEC-084-nagi-permission-linux-sandbox-enforcement.md)のNagi Permission／CapabilityとLinux側の隔離を組み合わせ、AIの判断に依存せず、復号後も現行の権限を適用する。起動・更新・アプリの署名検証は[DEC-087](DEC-087-tiered-system-trust-and-signature-verification.md)の段階的な信頼方針に従う。
- **復元と移行は用途に応じて行う。** 日常の変更を戻すWayback、外部媒体／NASへの自動差分Backupと選択式復元、モデル非依存の暗号化Profile Export／Import、起動・更新障害に対するSystem Recoveryは[DEC-105](DEC-105-user-environment-backup-and-wayback.md)の分担と安全境界を維持する。
- **Memory DBは[DEC-013](DEC-013-lightweight-memory-engine-benchmark.md)を維持する。** SQLiteを初期実用経路および比較基準とし、Rust製Memory専用DBを並行研究する。置換の判断には性能・信頼性と安全な移行・復元の検証を要する。

### 技術の評価候補（PROVISIONAL）

| 領域 | 評価候補と比較の観点 |
|---|---|
| System A/B | 読み取り専用イメージにEROFSを優先評価。完全性検証にdm-verityを評価。 |
| User Data／Model Store | ext4を先行比較の基準にし、User DataではBtrfsのsnapshot・圧縮・チェックサム等を比較する。F2FSは将来のフラッシュ媒体向け候補。 |
| User Data暗号化 | LUKS2／dm-cryptを優先評価。 |
| Memory／Profile暗号化 | DB向けのSQLCipherとファイル／ディレクトリ向けの[fscrypt](https://docs.kernel.org/filesystems/fscrypt.html)等を比較。fscryptは対応FSに制約があり、Btrfsとの組合せを前提にしない。ファイル名以外のファイルシステムメタデータは保護対象外。 |
| 鍵保護・導出 | TPM 2.0／TEE、パスフレーズからの鍵導出にArgon2id。 |
| Backupと整合性 | resticを第一評価候補とし、BorgBackupを比較候補とする。稼働中DBの整合したコピーにはSQLite Backup APIを評価する。Btrfs snapshotはローカルでの履歴・復元を支える候補。同一媒体のsnapshotでは媒体故障に備えられないため、外部媒体／NASへの独立したBackupを用いる。 |
| 実行時隔離・起動時の信頼 | Linux namespaces、seccomp、Landlock等のLSM、cgroups v2、対応機器でのSecure Boot。 |

個別技術の正式採用・組合せ・依存バージョン・ライセンス／再配布条件、物理パーティションと容量配分、実機性能と電源断・中断・復元への耐性、UI、提供版・受入の詳細はPROVISIONAL。Profile別暗号化の実効性、最新忘却状態を確認できない場合の隔離などの処理、保存・復元の検証方法は、それぞれDEC-117／DEC-105の未確定事項に従う。既存のRelease／Activation／Acceptance Gate、実装範囲と担当を維持する。
