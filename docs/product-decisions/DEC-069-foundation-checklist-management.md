# DEC-069 — Nagi OS Foundation Checklistの恒久管理

**状態:** CONFIRMED（35項目を体系的に台帳管理すること）／PROVISIONAL（推奨優先度・検討順序）
**最終合意日:** 2026-11-16
**対象:** OS基盤の製品判断・設計・実装・受入をつなぐ索引と進捗管理
**関連:** [Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[運用規則](OPERATING_RULES.md)、[DEC-042](DEC-042-linux-kernel-transition.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-044](DEC-044-linux-migration-development-authorization.md)、[DEC-047](DEC-047-linux-migration-workspaces-and-integration.md)、[DEC-050](DEC-050-public-specification-repository.md)

## 目的と確定した決定（CONFIRMED）

ユーザーは、提案された35項目を「Nagi OS Foundation Checklist」として管理することを明確に決定した。公開リポジトリ `Roy-To/NagiOS` の台帳に、元の番号1–35と名称を省略せず保存し、以後の仕様決めと進捗更新の共通入口にする。

- 一覧は6つの分類群、今後の検討は別軸の7つの検討束として保持する。項目の削除・改番によって未解決の論点を隠さない。
- 各項目で、製品決定、技術設計、実装、対象環境での受入、依存と担当候補、根拠と次の判断を管理する。
- 台帳はDEC・技術ADR・実装仕様・workstreamの代替ではない。製品決定は対応DEC、具体設計はADR／実装仕様、担当と実装証拠は現行workstream等を正本とする。
- 仕様が固まった範囲を、前提・非目標・依存・Acceptance・実機／セキュリティ検証・担当・証拠を含む実装契約へ渡せるようにする。

## 今回確定していないこと

- 各項目のP0/P1/P2はassistantの推奨値であり、REVIEW NOTE / PROVISIONAL。製品の確定優先順位ではない。
- 全35項目それぞれの製品要求詳細、技術採用、数値目標、リリース割当て、全件の実装開始や担当割当てを一括承認したものではない。
- 既存Linux移行開発承認はその範囲で有効。旧カーネル完成を移行開発の一律前提にせず、既存Release／Activation／Acceptance Gateも変更しない。
- Linux kernel + Nagi専用userlandと非特権AI等の確定境界は継承する。Linuxの版、init、libc、filesystem、sandbox、display backend等の未決定技術を本DECで採用しない。

## AI実装との関係

仕様と契約が明確になればAIによる実装分業を加速できる。一方、Linux上の統合、実機・firmware・driver・電源動作、セキュリティ、性能測定と受入は実際の証拠を要する。AIの自己申告や仕様確定だけで完了とはせず、製品の最終判断・受入責任をAIへ自動移譲しない。全35項目の仕様確定待ちを新しい一律の着手Gateにもしていない。

## 維持と変更履歴

台帳の扱い方・証拠区分・更新手順は[台帳本文](NAGI_OS_FOUNDATION_CHECKLIST.md)に置く。既存決定が変わった場合は正本DEC／ADRを先に更新し、台帳から参照する。未マージPRをmainの確定実装として扱わない。

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーの35項目の恒久台帳化を記録。6分類群・7検討束、決定／設計／実装／受入の分離、暫定優先度と実装契約への引渡しを整理。個別技術・版・Gateは変更しない。 |
