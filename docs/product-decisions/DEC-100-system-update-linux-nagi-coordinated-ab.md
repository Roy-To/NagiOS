# DEC-100 — System Update：Linux更新機構を活用したNagi統合Update Service（方式B）

**状態:** CONFIRMED（Foundation #31のB方式：Linuxの更新・起動・保存機構を活用し、Nagiが更新確認・真正性／署名・互換検証・A/B切替・失敗時の復旧を統合管理する）／PROVISIONAL（Linux更新コンポーネント、配布／署名鍵運用、更新単位、実装・UI・自動適用条件・対象端末／Release・target Acceptance）\
**最終合意日:** 2026-11-16\
**対象:** Nagi OS Foundation #31 System Update。製品方針の確定であり実装・Release／Activation Gateを変更しない\
**関連:** [Foundation Checklist #31](NAGI_OS_FOUNDATION_CHECKLIST.md#f31)、[DEC-079（read-only System・User Data分離・A/B）](DEC-079-system-image-readonly-ab-data-separation.md)、[DEC-078（Boot）](DEC-078-boot-platform-entry-and-common-nagi-startup.md)、[DEC-035（builder製App更新）](DEC-035-builder-created-app-maintenance-updates.md)、[DEC-017（Nagi Native Apps）](DEC-017-third-party-developer-ecosystem.md)、[DEC-043（Linux・専用ユーザーランド）](DEC-043-nagi-userland-independent-os.md)、[DEC-099（OS Installer）](DEC-099-installer-first-setup-linux-nagi-gui.md)、[0.2実装仕様 §16／M15](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[Roadmaps](roadmaps/README.md)

## 目的・背景

LinuxカーネルとNagi専用ユーザーランドによるOS本体を、ユーザーデータを保護しながら安全に更新・再起動・復旧できるようにする。旧0.1のA/B・Boot Trust設計や0.2のInstall／Update Runtimeを既存資産として尊重しつつ、Linuxに移行した製品の正規更新経路をNagi固有のUpdate Serviceで統合する。

## 確定した製品方針（CONFIRMED：B）

1. **Linuxの既存機構＋Nagi統合Update Serviceを基本とする。** Linux側の更新・ストレージ・Boot切替・再起動等の利用可能な機構を選択的に利用し、Nagiの正規Update Serviceが更新の確認、計画、検証、適用、状態表示、失敗時の復旧判断を一貫して管理する。Linuxディストリビューション全体の採用や、特定のPackage Manager／Update技術の即時採用を意味しない。
2. **更新元と対象を検証する。** OS System Image／更新Artifactは、信頼できる署名・真正性、バージョン／互換性、適用先と現在の状態、利用可能な回復経路などを確認してから適用する。署名方式、Trust Anchor・鍵ローテーション・配布方法はPROVISIONALとし、署名検証だけで完全安全を保証したと扱わない。
3. **A/B System更新と失敗時復旧を統合する。** [DEC-079](DEC-079-system-image-readonly-ab-data-separation.md)がCONFIRMEDとするread-only System、User Data分離、A/B更新・復旧を維持する。Update Serviceは更新準備、切替、Boot後の状態確認、失敗時の戻し方を統合する。具体slot・Boot Control／Atomic Commit／Health Gate・Fallback条件は後続設計で決め、旧Kernel実装の受入実績をLinux targetの証拠に流用しない。
4. **System・App・Modelの更新責務を分離する。** OS本体、Nagi Native App、ローカルLLM Model／Model Storeの更新は別の対象・互換・保存／認可・Rollback境界を持つ。下位の検証／Package／Update primitiveを共用できても、System Image更新の承認や成功を、アプリの自動正式更新・モデル無断切替・Memory変更の許可としない。特にbuilder製アプリの正式更新は[DEC-035](DEC-035-builder-created-app-maintenance-updates.md)のユーザー確認後の承認・検証を維持する。
5. **User Dataと復旧の限界を正確に扱う。** 通常のSystem更新後もUser Data／設定／Memory／Workspaceを保持する方針を維持する。ただしSystem slot rollbackとUser Dataの過去状態への完全復元を同一視しない。非互換／不可逆なData Migration、部分更新、電源断、中断、更新後Boot不良は、実際に復旧可能と検証できた範囲で扱い、成功やデータ無損失を推測で報告しない。
6. **通常の更新判断は決定論的に強制する。** 許可・Policy、署名、互換、対象Image／Revision、適用結果の検証を正規OS Serviceで行う。AIの推測や通知・提案をSystem更新のAuthorityとして扱わない。UIに表示する更新状態・失敗と後続操作は実際の証拠に基づかせる。
7. **方式Cの高度な自動更新を今回の必須目標にしない。** 差分更新、段階配信、電力・通信状況に応じた実施タイミングの自動最適化、無人の自動適用などは有用な後続候補だが、ユーザーが今回採用したのは方式B。Cの機能を正式に採用するか、更新確認／ダウンロード／適用をどこまで自動化するか、ユーザー確認をいつ必要とするかは未決定。

## 既存契約との境界

- **DEC-079:** read-only System／User Data分離、A/B更新・復旧、AI Model論理分離は既存のCONFIRMED事項。partition数・rootfs・署名・slot実装は未決定。
- **DEC-078:** PC UEFI基本・端末別Boot方式を許容し、Linux起動後の共通Nagi処理を維持。Boot chain／署名・fallbackはLinux上で個別確認する。
- **0.2 §16／M15:** Package inventory、manifest compatibility、source／version/provenance、atomic commit、rollback、interruption recovery、Policyを継承。hostの成功結果をLinux System Imageの実機更新成功と取り違えない。active inventoryとdurableな実体が一致するまで成功と主張しない。
- **DEC-035／017:** Nagi Native AppsのManifest／Package、別承認と更新データ保護を維持。System更新とbuilder製App更新の許可を統合しない。
- **DEC-099:** OS初回Installerと導入後System Updateは別の製品操作・状態機械で、同じLinux Tooling利用でも承認・データ危険性・復旧を個別に検証する。

## 技術／運用上の未決定事項（PROVISIONAL／OPEN）

- Linux採用部品、更新単位（Image／Package）、slot layout・Boot切替と健康確認、読み取り専用領域／不可変システムへの適用方式、端末別の必要例外。
- 配信Channel、更新元の信頼、公開署名鍵・検証方式・鍵更新、offline／USB等経路、バージョン互換・依存性・rollback可能性の判断。
- 更新の自動ダウンロード・適用の既定とUser consent／再起動／延期UI、再起動を伴うJob・Task／アプリ状態の調整と権限。
- System ImageとUser Data schema migrationの前方・後方互換性、バックアップ・復旧、電源断／容量不足／ネットワーク断・再試行のdurable state、rollback不可のケース。
- App Package・Model更新との具体APIの共有範囲、権限・更新Window、Model Storeの格納と容量・更新時停止／切替条件。
- Linux target／実機の更新→再起動→失敗注入→復旧Acceptance、配布インフラ、対応機器・性能値、担当／実装時期・提供Version。
- Cに含まれる差分更新、段階配信、端末状態最適化等の採否・優先度・効果。採用済みと推定しない。

## Roadmapとの関係

方式Bの製品方針のみ確定する。既存0.1／0.2／0.3／0.4以降のRelease・Activation・Acceptance Gate、Linux移行の作業許可や導入のTargetを変更しない。CI／host-only acceptance、旧guestの成功、PRでの仕様記録はLinux image更新と実機復旧の実施・合格証拠ではない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #31で方式Bを明示承認。Linuxの更新機構とNagi統合Update Serviceによる署名／互換検証、A/B切替・失敗時の復旧を製品標準とし、System・App・Model更新を区別。Cの高度な自動更新、具体OSS／署名運用／端末／提供版は未決定。 |

## 関連決定の参照注記（2026-11-21）

- [DEC-118](DEC-118-support-period-and-vulnerability-response.md) — 脆弱性修正は同決定の種類別目標に沿って配布する。修正を急ぐ場合も、署名検証とA/B更新・ロールバックの手順を適用する。
