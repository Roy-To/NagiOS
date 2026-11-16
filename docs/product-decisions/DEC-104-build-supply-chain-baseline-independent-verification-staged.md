# DEC-104 — Build / Supply Chain：統合管理Bを先行し、独立検証・強化自動化Cを将来の正式目標とする

**状態:** CONFIRMED（Foundation #35：健全な供給網の統合管理Bを先行導入し、隔離／独立検証・複数Architectureの継続検証・より厳格な改ざん検出とセキュリティ自動化Cを将来の正式製品目標として段階導入）／PROVISIONAL（具体ツール・CI規模・予算・日程・閾値・必須Release版・Linux target実証）  
**最終合意日:** 2026-11-16  
**適用範囲:** Nagi OS Foundation Checklist #35 Build / Supply Chain。製品方針の記録であり実装完了・Release受入ではない  
**関連:** [Foundation Checklist #35](NAGI_OS_FOUNDATION_CHECKLIST.md#f35)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-050](DEC-050-public-specification-repository.md)、[DEC-077](DEC-077-linux-kernel-lts-stable-policy.md)、[DEC-079](DEC-079-system-image-readonly-ab-data-separation.md)、[DEC-081](DEC-081-nagi-owned-userland-selective-oss-components.md)、[0.2実装仕様 §18–19](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[Roadmaps](roadmaps/README.md)

## 目的

Linux kernel、rootfs、Nagi userland、apps等について、何を入力にして何を生成・配布したかを追跡し、再現性・出所・ライセンス確認・成果物の真正性と整合性を一貫して管理する。既存Linuxディストリビューションの完成rootfs／依存・配布体系の製品基盤採用とは区別し、Nagiが責務を持つ専用ユーザーランドとOSSの選択的利用を維持する。

## 確定した製品方針（CONFIRMED）

1. **Bから始める。** ビルド入力・依存の固定、source pin／patch管理、ライセンス／NOTICE／SBOM、Provenance、再現性、署名・改ざん検証、成果物整合性を統合した健全な供給網管理を先行導入する。これらは別々の一覧を出力するだけで完了するものではなく、入力から生成物・配布対象までの対応を確認できるようにする。
2. **入力と変更を追跡する。** Linux kernel／config、rootfs構成、Nagi userland／apps、選択したOSS、toolchain、生成入力と依存等を対象とし、実際のrevision・内容digest・patchとビルド条件を追跡する。固定入力の更新も追跡可能にし、DEC-077の製品LTS標準とStable先行検証を混同しない。具体的なlock形式・採用revision・更新周期は今回固定しない。
3. **ライセンス確認を供給網に組み込む。** component／依存の出所とライセンス根拠、NOTICEとSBOMを結び付け、不明・不足・確認待ちを明示する。SBOMやNOTICE候補の生成だけでライセンス適法性・配布義務充足を自動認定しない。組合せ・改変・配布方法による義務と不明点は個別確認へ残す。
4. **再現性と成果物の対応を検証する。** 同じ入力・条件から生成した成果物の再現性を確認し、Provenance、実際の成果物のdigest、manifest／inventory、署名・配布対象の対応を検証する。メタデータの一致だけで外部成果物の現在のbytesまで検証済みとしない。0.2 §18の決定論的比較と、互換入力不一致時のfail-closedな再利用契約を維持する。
5. **真正性・改ざん検証をBの段階から含める。** 信頼する入力・生成／配布成果物について、署名と改ざん検証を管理に組み込む。未署名、信頼不明、検証失敗、欠落を成功と表示しない。署名成功だけで内容の安全性やライセンス適合を保証した扱いにしない。
6. **Cを将来の正式製品目標とする。** Bの基盤を整えた上で、隔離されたビルド／検証、独立した検証、複数Architectureの継続検証、より厳格な改ざん検出・セキュリティ自動化等を段階導入する。Cは単なる研究候補ではない。一方、Bにも再現性・署名・改ざん検証が含まれており、それらをCまで先送りする決定ではない。
7. **LLMの推測を検証の代替にしない。** AIは調査・説明・異常候補の整理を支援できるが、推測・自己評価を出所の証拠、ライセンス判断の根拠、再現ビルドの一致、署名／digest検証や受入PASSの代替としない。中核の供給網検証はLLM不在でも成立させ、未確認・失敗・部分成功・結果不明を区別する。

## 既存決定・実装との境界

- **DEC-043／081:** Linux kernel＋Nagi専用userland、Nagi固有サービス／API／アプリ環境と選択的OSS活用を継承する。rootfs builder、libc、Package Manager等の具体技術は選定しない。
- **DEC-077:** LTS標準・Stable先行利用は既存の確定方針。正確なkernel revision／config／patchを追跡する供給網管理と、系列・保守運用の採用判断を区別する。
- **DEC-079:** read-only System、User Data分離、A/B更新・復旧、Model Store論理分離を維持する。供給網の検証成功は、実機Boot・A/B切替・データ保持／復旧の成功や更新適用の許可を意味しない。
- **Foundation #31／DEC-100:** System・App・Modelの更新責務と承認を維持し、ビルド成果物の検証と実行中システムへの適用を区別する。参照した[DEC-100の固定記録](https://github.com/Roy-To/NagiOS/blob/b121301932003017d98a8fafb5b9e2fc33a58197/docs/product-decisions/DEC-100-system-update-linux-nagi-coordinated-ab.md)は[PR #139](https://github.com/Roy-To/NagiOS/pull/139)側にあり、本記録時点でmain未マージ。Cの供給網強化は、#31で未採用の差分更新・段階配信・無人自動適用を採用する意味ではない。
- **Foundation #34／DEC-103:** [Testing / Releaseの固定記録](https://github.com/Roy-To/NagiOS/blob/b121301932003017d98a8fafb5b9e2fc33a58197/docs/product-decisions/DEC-103-testing-release-evidence-baseline-ai-assisted-staged.md)も同PR側の未統合記録として参照する。host／旧guest／Linux Image・QEMU／実機の証拠を区別し、#35のビルド検証から全Runtimeの成功を推定しない。
- **0.2 §18–19:** source／patch／toolchain／target／allowlisted environment等のfingerprint、artifact SHA-256 inventory、license不明状態、SBOM／NOTICE候補・手動確認の既存契約を保持する。[host契約資産](../../tools/provenance-sbom-contract/README.md)はLinux全体の実装・受入証拠ではなく、メタデータ比較だけで成果物再利用を認可しない。
- **DEC-050:** 記録先は公開リポジトリ。Provenance／SBOM／ログにも認証情報・個人情報・ユーザー名入りlocal path・チャット識別子を記録しない。公開可能な製品・ビルド情報と秘密を区別する。
- **既存Gate:** Release／Activation／Acceptance Gate、Linux移行の担当・許可範囲を変更しない。全35項目やCの完成を一律の新Release条件としない。

## 未確定事項（PROVISIONAL／OPEN）

- source／dependency lock、rootfs生成、再現性比較、Provenance／SBOM／NOTICE、署名・検証・改ざん検出・脆弱性自動化の具体ツール、形式、連携APIと採用OSS。
- 信頼する供給元、鍵の保管／配布・更新／失効、署名対象、Trust Anchor、検証不能時の具体処置、隔離境界・独立検証者と独立性の評価方法。
- 再現性の対象範囲・環境、非決定的入力の扱い、比較方法・合格閾値、ビルド／配布キャッシュの検証・再利用条件の詳細。
- Architecture／端末／component別の継続検証Matrix、CI／runner／保管規模、予算、担当・保守責任、導入順序・日程・必須Release版。
- 不明ライセンス・依存の確認体制、個別のライセンス適合・配布条件の結論。SBOM生成済みを確認完了へ読み替えない。
- Linux kernel／rootfs／userland／appsを含む再現build、隔離／独立検証、実際の署名・改ざん検出と配布・Linux target受入の結果。今回の文書記録では実証していない。

## Roadmap・検討一巡との関係

B先行とCの正式な将来製品目標を確定し、0.3／0.4+／0.5–1.0等の特定版への提供割当てはしない。既存の版境界・Acceptanceと開発速度優先の方針を維持する。

**REVIEW NOTE:** #35までの方針検討は一巡した（別PRの未統合記録を含む）。これは35項目すべての技術選定・未確定点の解消、mainへの全件統合、実装完了、Linux／実機・Release受入の完了を意味しない。次段階では索引・DEC番号の統合整合、最小Linux縦断経路の技術契約、対象を絞ったBの検証と証拠整備を優先候補として検討する。新たな担当・日程・実装許可・Gateの決定ではない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #35で「Bから始めてCを将来の正式目標にする」と明示確定。統合供給網管理Bを先行し、隔離／独立・複数Architecture継続検証と強化自動化Cを正式な将来目標として記録。技術・規模・予算・日程・閾値・必須Release・Linux実証は未確定。SBOM・AI推測の限界、既存Gateを維持。 |
