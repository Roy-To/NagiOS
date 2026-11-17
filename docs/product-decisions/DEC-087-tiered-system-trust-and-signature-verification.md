# DEC-087 — 段階的なシステム信頼基盤と署名検証

**状態:** CONFIRMED（B案：ハードウェア信頼機構を対応機器で活用し、非対応機器でもOS／更新／アプリの署名検証を行う段階的信頼基盤、Ownerによる限定Developer Mode、製品鍵／開発鍵の分離・更新／失効／安全な復旧を設計対象とする）／PROVISIONAL（具体Trust Anchor／Secure Boot方式・署名Schema・Rollback防止／鍵運用・対応機器／実装版）
**最終合意日:** 2026-11-16
**適用範囲:** Nagi OS Foundation Checklist #11「システム信頼基盤」（検討束B2）。Linuxカーネル＋Nagi専用ユーザーランドのBoot／System Image／更新／App Packageの信頼検証
**関連:** [DEC-017 第三者アプリ／配布](DEC-017-third-party-developer-ecosystem.md)、[DEC-020 AIの操作経路](DEC-020-ai-os-interaction-path.md)、[DEC-043 Linux＋Nagi専用userland](DEC-043-nagi-userland-independent-os.md)、[DEC-078 端末別Boot](DEC-078-boot-platform-entry-and-common-nagi-startup.md)、[DEC-079 System Image／A/B](DEC-079-system-image-readonly-ab-data-separation.md)、[DEC-083 Identity](DEC-083-nagi-identity-linux-credentials-binding.md)、[DEC-084 Permission／Sandbox](DEC-084-nagi-permission-linux-sandbox-enforcement.md)、[DEC-086 Secrets／Key Management](DEC-086-nagi-secrets-service-hybrid-key-protection.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[0.1 Boot Trust設計](../architecture/boot-trust-and-system-updates.md)、[Roadmaps](roadmaps/README.md)

## 目的

Nagiが起動し、更新し、利用するOS／アプリの正当性と改ざんの有無を検証する製品方針を定める。UEFI Secure Boot等のハードウェアによる信頼の起点を持つ機器を優先的に保護しながら、非対応機器も直ちに排除せず、信頼保証の強さの違いを明示する。

## 確定した製品方針（CONFIRMED）

1. **段階的な信頼基盤（B案）を採用する。** ハードウェアにより起動経路を保護できる機器では、その機構を信頼の起点として活用する。対応しない機器では、NagiのBoot／System／Update／App等の信頼検証をソフトウェア署名検証で行う方針とし、Secure Boot対応をNagi OSの全機器必須条件にはしない。各方式の保護保証が同等とは表示しない。
2. **正規の署名・完全性検証を重視する。** 起動・System Image・更新・アプリPackageの検証境界を明確にし、適切な署名者・署名対象・更新の連続性・改ざんの有無を確認する。検証できない・署名が不正なOS／更新は正常な信頼済み起動／更新として黙って採用しない。署名検証だけでファイルの機密性やOS上での操作権限まで保証したと扱わない。
3. **OwnerのDeveloper Modeは限定的な例外として維持する。** 既存0.1仕様の通り、Ownerが明示的にDeveloper Modeを有効化し、警告と必要な認証／認可の下で未署名アプリを導入できる。通常アプリ／AIがDeveloper ModeやOwner overrideを勝手に行うことを認めない。未署名アプリ導入の許容はOS Bootの署名・信頼検証を無条件に無効化する決定ではない。
4. **製品鍵と開発用鍵を区別する。** 製品用／開発・検証用の署名鍵と信頼範囲を分離し、署名鍵の更新・失効・信頼変更や漏洩時対応を設計対象とする。旧0.1の公開テスト署名鍵を製品用Trust Anchorとして扱わない。
5. **更新・ロールバックと安全な復旧を連携させる。** DEC-079のread-only System／A/B更新・復旧と整合し、破損・改ざん・検証失敗した更新を通常成功として有効化しない。古い脆弱なImageへの危険なロールバックを防止する方針を設計対象に置く。ただし復旧可能性、Rollback許可の例外、単調counter等の具体方式は未決定。
6. **既存のIdentity／Permission／AI非特権を維持する。** 起動検証・署名検証の成功は、アプリが自動的にSystem Capability／Owner権限を得ることや、User Dataへの読み書きが認可されたことを意味しない。ハードウェア保護がない環境で保護を同等だと偽らず、現在のLinux Sandbox・Secrets Service・権限を引き続き適用する。

## 例外と既存正本の尊重

- **DEC-078:** UEFI対応PCはUEFI基本、Raspberry Pi／タブレット等では端末のBoot方式を許容。UEFI Secure Boot固有技術を全端末へ強制するわけではない。
- **DEC-079:** System Imageのread-only、A/B更新・Recovery、User Data／AI Model分離を維持する。更新の成功判定・rollback・データ保護の具体実装は別途判断する。
- **DEC-086:** Secrets ServiceのC案（対応機器はTPM／TEE優先、非対応でも暗号化ローカル保管）と連携するが、鍵の秘密保管、BootのTrust Anchor、署名の検証、データ暗号化・ユーザー認可をそれぞれ区別する。TPM非搭載＝署名なしの無条件起動とはならない。
- **DEC-017／020と0.1仕様:** Ownerの明示的なDeveloper Modeと限定的な未署名アプリ導入、純正・第三者の正規Package契約、非特権AI、正規Public Action・現在のPermission／Policyを維持。署名済みアプリも必要なPermissionを別に評価する。
- **旧0.1 Boot Trust:** UEFI loader、署名付きslot manifest、A/B起動／署名検証、開発Preview向け公開テスト署名鍵、Rollback判定等は技術・試験の参照資産。ただし旧独自kernelの`SLOT.MAN`／Ed25519／FAT32／rollback counter設計をLinux版で無条件に採用・PASSと扱わない。公開テスト鍵からproduction署名鍵への切替、鍵rotationと改ざん耐性のあるRollback counterは旧仕様でも未完了。
- **適用と受入:** 既存の0.1／0.2／0.3以降のRelease／Activation／Acceptance Gate、Workstream owner、実装開始権限や機器・バージョンを本決定で変更しない。

## 未確定・暫定案（PROVISIONAL／OPEN）

- Linux向けの具体的な信頼チェーン、UEFI Secure Boot・Bootloader／Kernel／initramfs／rootfs・Application Packageの署名検証地点、適用するTrust Anchor・鍵形式・署名方式。
- ハードウェアに信頼の起点がない端末でのSoftware検証主体自体の改ざんリスク、物理攻撃・盗難時の保証／警告表示・Owner overrideの範囲。
- 署名鍵の正規発行／保管・rotation／revocation、開発鍵から製品鍵への移行・失効情報更新、攻撃・漏洩・中断時のRecoveryと必要なNetwork／Offline条件。
- Anti-rollbackに用いる単調counter等の適用、古い脆弱版の扱い、復旧Image／Recovery／Developer Modeの信頼境界とユーザー体験。
- 対象機器別のSecure Boot対応、パフォーマンス、実機／Nagi Linux image受入、提供版・対応時期。

## 技術設計への引渡し（REVIEW NOTE）

最小Linux開発targetで、信頼済み起動／更新と改ざんImage・不正Packageの拒否、失効済み署名者・不正なrollback／update、Owner許可済み未署名アプリ導入を別ケースで検証する。対応機器のHardware-backed Boot Trustと非対応機器のSoftware verificationを区別して証拠を記録する。署名が通ることだけで機密性、User／Profile認可、Linux Sandbox、AIの制限、データ復旧をPASSとしない。

## Roadmapとの関連

Foundation #11の製品方針として、Boot #2、System Image #3、Secrets #10、Package／Update等の設計と接続する。0.2や0.3の新必須要件・具体Release／Activation／Acceptance Gateの変更を意味せず、Linux移行の成功・実装完了・実機のSecure Boot受入を主張しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #11「システム信頼基盤」でB案を明確に承認。対応機器でHardware root of trust／Secure Boot等を活用し、非対応機器でもソフトウェア署名検証する段階的方式、Ownerによる限定的Developer Mode、製品／開発鍵分離・更新／失効・安全な復旧の設計方針を採用。具体的なTrust Anchor／署名方法／rollback／実機対応・適用版は未決定。 |
