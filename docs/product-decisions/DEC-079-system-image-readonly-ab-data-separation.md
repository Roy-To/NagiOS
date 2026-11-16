# DEC-079 — System Imageの読み取り専用化・データ分離・A/B更新／復旧方針

**状態:** CONFIRMED（通常時read-only System・User Data分離・A/B更新と復旧・AI Model論理分離）／PROVISIONAL（具体ディスク構成・更新／復旧の実装・適用版）
**最終合意日:** 2026-11-16
**適用範囲:** Nagi OS Foundation Checklist #3「System Image」（検討束B1）。Linux採用後の製品方針。対象機器・リリース版・切替時期は未確定
**関連:** [DEC-042](DEC-042-linux-kernel-transition.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-069](DEC-069-foundation-checklist-management.md)、[DEC-077](DEC-077-linux-kernel-lts-stable-policy.md)、[DEC-078](DEC-078-boot-platform-entry-and-common-nagi-startup.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[Roadmaps](roadmaps/README.md)、[旧0.1 Boot Trust設計](../architecture/boot-trust-and-system-updates.md)

## 目的

LinuxカーネルとNagi専用ユーザーランドからなるOS本体を、利用者のデータと分離して更新・復旧可能な構成にする。既存の0.1向けA/B・Recovery設計を参照しつつ、旧独自カーネル向けのディスク形式や実装をLinuxでそのまま採用したと扱わない。

## 確定した製品方針（CONFIRMED）

1. **OS本体を通常時は読み取り専用とする。** Nagi OSのシステム本体は、通常稼働中に書き換えないread-onlyを基本とし、システム更新・保守を通常のユーザーデータ書き込みと区別する。
2. **User DataをOS本体から分離する。** 利用者のファイル、設定、Memory、Workspace等のデータはシステム本体から分離し、OSの通常更新後も保持することを基本とする。
3. **A/Bシステム更新と復旧を基本構成とする。** システム更新はA/B方式による切替と、更新失敗時などに備えた復旧の仕組みを基本とする。実装の受入・復旧が成功する範囲は別途検証する。
4. **AIモデルをOS本体と論理的に分離する。** ローカルLLM等のモデルデータは、OS本体の更新領域とは論理的に区別して管理する。

これらは論理的な設計方針であり、独立した物理パーティション数、特定のファイルシステム、ディスク上の配置、モデルの格納方式を今回固定したものではない。

> **2026-11-16 システム信頼基盤の後続決定:** [DEC-087](DEC-087-tiered-system-trust-and-signature-verification.md)で、OS／更新／アプリの正規署名・完全性確認、対応機器でのハードウェア信頼機構活用／非対応機器でのSoftware署名検証というB案をCONFIRMED。read-only System／A/B更新・Recovery／User Data分離の方針は維持し、起動署名検証・rollback防止・不正更新拒否の具体機構やLinux実機の成功は別に検証する。

> **2026-11-16 Storage／Filesystem後続決定:** [DEC-088](DEC-088-linux-filesystem-nagi-storage-api.md)で、Linux標準FilesystemをSystem／User Data／Model Store／Temp等の用途別に選択活用し、Nagi独自Storage／Files APIを維持するB案をCONFIRMED。本DECのread-only System、User Data更新後保持、A/B更新・Recovery、AI Model論理分離は不変。具体FS・partition／mount／暗号化・durabilityやLinux target受入は未確定。旧独自kernelのext2／FAT32をLinux製品構成に昇格しない。

> **2026-11-16 Volume／Mount後続決定:** [DEC-089](DEC-089-nagi-volume-manager-linux-mount-hotplug.md)で、Linuxのblock／mount／hotplugを活用しNagi Volume ManagerがVolume検出・認可・安全な取り外し・突然の切断状態を統括するB案をCONFIRMED。System read-only、User Data分離、A/B更新・RecoveryとModel Storeの論理分離は不変。外部媒体の自動mount・任意のSystem書込み許可・具体Linux mount構成／受入を今回追加確定しない。

## 既存決定と安全・復旧境界

- [DEC-043](DEC-043-nagi-userland-independent-os.md)に従い、Nagi専用ユーザーランドとrootfsを構成し、既存Linuxディストリビューションの完成済みrootfs／配布体系を製品基盤として採用しない。Linux LTS標準と端末別Boot方針（DEC-077／078）も維持する。
- [DEC-014](DEC-014-workspace-state-checkpoints.md)等で確定済みのWorkspace・Memory・User／Profile／Permission・データ所有・復元前保護等の仕様を、単なるシステム更新を理由に解除しない。
- システムのA/Bロールバックは、ユーザーデータ全体のバックアップ／過去版への復元を意味しない。更新前後のデータschema互換性・移行・復旧範囲は別途設計・検証する。
- 旧0.1向けのGPT／ESP／System A／System B／User Data／Recovery／Model Storeの6領域構成と署名・Boot readiness・回復設計は既存資産として維持する。Linux版で同一partition数・FAT32・署名方式・Loaderをそのまま採用することや、旧guestの検証成功をLinux targetのPASSにすることは承認しない。
- 既存Release／Activation／Acceptance Gate、担当と作業権限、配布開始や対応実機の判断は変更しない。

## 未確定事項（PROVISIONAL／OPEN）

- 物理partition構成・数、rootfsの具体的内容・構築技術、filesystem選定、read-onlyの実装方式、書き換えが必要なシステム状態・例外領域。
- A/Bの実装・更新単位・slotの検証と正常判定・boot control／切替・rollback条件、復旧領域の内容と保護、署名・Secure Boot等の具体技術。
- システム更新とUser Dataのschema変更が衝突した際の互換性・Migration・Rollback・バックアップ／復元の実行可能範囲。ユーザーデータが障害・故障時にも必ず保存できるとの保証は未検証。
- Model Storeの実際の格納位置（専用partitionか共通データ領域か等）・容量・バージョン管理・モデル入替時の取り扱い。
- Raspberry Piやタブレットなど容量やBoot方式の異なる端末での適用方法、必要な例外、配布形式、更新／復旧の検証条件と結果、対応Release・導入時期。
- Linux imageの生成入力・再現性、実機起動・更新後再起動・データ保持・故障注入試験の受入方法と証拠。

## 技術設計への引渡し（REVIEW NOTE）

最初のLinux開発targetについて、OSとユーザーデータの書込境界、更新の状態遷移、正常／失敗時のboot選択、データschemaの前方・後方互換性、モデル格納の論理契約をADRとAcceptanceで具体化することが望ましい。候補構成が成立したかは実際のLinux imageとデータ保持／復旧検証で確認し、旧0.1の成功を移行先へ流用しない。

## Roadmapとの関連

本決定はSystem Imageの基本方針のみを確定する。0.2・0.3・将来版のRelease割当て、既存Acceptance／Activation Gate、実装開始の許可や正式製品版への切替を変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #3で「通常時read-onlyのOS本体」「更新後も保持する分離User Data」「A/B更新と復旧を基本」「AIモデルの論理的分離」を確認。ディスク構成・filesystem・具体Boot／Recovery・適用Releaseは未確定とした。 |
