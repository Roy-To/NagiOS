# DEC-041 — `builder`製アプリのアンインストールと関連データ削除

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（通常アンインストールと完全削除の分離、対象別の明示選択・承認、安全境界）／PROVISIONAL（保持期間、容量、管理UI、実装方式・適用版）  
**最終合意日:** 2026-11-05  
**対象:** `builder`が作成したアプリのアンインストールと関連データのライフサイクル。実装版は未確定  
**関連:** [DEC-014（Workspace保存・チェックポイント）](DEC-014-workspace-state-checkpoints.md)、[DEC-017（アプリ開発・配布）](DEC-017-third-party-developer-ecosystem.md)、[DEC-019（builder作業データ整理）](DEC-019-builder-work-data-cleanup.md)、[DEC-025（アプリ横断参照）](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-026（変更の検証・復元）](DEC-026-ai-change-verification-and-recovery.md)、[DEC-027（Public Action）](DEC-027-public-action-and-scoped-gui-fallback.md)、[DEC-030（Workflow共有）](DEC-030-workflow-template-portability-and-sharing.md)、[DEC-035（アプリ保守・更新）](DEC-035-builder-created-app-maintenance-updates.md)、[DEC-036（アプリ共有・公開）](DEC-036-builder-app-sharing-and-publication.md)、[DEC-036（試用・正式導入）](DEC-036-builder-app-trial-and-installation.md)、[DEC-040（開発Project長期保存）](DEC-040-builder-app-development-project-retention.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)、[0.5–1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

## 目的

アプリの削除と、そのアプリに関連するデータ・開発成果物の削除を分け、通常のアンインストールでユーザーのデータや将来の復旧・再利用に必要な成果物を失わないようにする。完全削除は、対象と影響を理解したユーザーが、削除対象を個別に選び承認した場合にのみ行う。

## 確定した仕様 — 選択C

1. **通常のアンインストールはアプリ本体を対象とする。** インストール済みアプリ本体を削除する。ユーザーデータ、設定、保護されたソースコード、正式な開発Project、開発履歴は原則として保持する。
2. **データ種別と削除対象を分けて管理する。** アプリ本体、アプリが作成・利用するデータ、設定、保護されたソースコード、開発Projectと履歴、必要な復旧情報を、それぞれ識別し個別の管理・削除対象とする。アプリ本体のアンインストールを、これらすべての削除許可と解釈しない。開発Projectの長期保持と再発見・Scope付き再開の方針は[DEC-040](DEC-040-builder-app-development-project-retention.md)に従う。
3. **完全削除前に対象ごとの確認と承認を得る。** 完全削除を望むユーザーには、Nagiが削除候補ごとに対象、既知の共有参照、影響、回復可否を具体的に示す。ユーザーが削除対象を個別に明示選択・承認した後で、現在のOS Permission／Policyと正規Public Action／Providerの条件を満たす対象だけを削除する。対象や影響を特定できない項目は削除しない。
4. **他者・他作業に関係するデータを巻き込まない。** 他アプリまたはTaskが参照する共有データ、作業中の成果、復旧用データ、他ユーザー所有データ、所有者・参照関係・対象が未確認のデータを、アプリのアンインストールや一括削除に含めない。必要な共有参照の解除・移行・削除は、別途対象と影響を示し、該当する権限と承認を得る。
5. **再インストール・再ビルドの可否を保証しない。** 再インストールや保存ソースからの再ビルド可能性は、実際に保持されたArtifact／Manifest、その完全性・互換性、依存関係、現在のPermissionとPolicyに依存する。アンインストールを理由に削除済み成果物の復元や、古いPermission／Capabilityの自動再付与を約束しない。必要な権限は現在のOS Policyに従って改めて確認・承認する。
6. **安全なキャッシュ整理は別の規則に従う。** 再生成可能で不要な作業キャッシュの整理は[DEC-019](DEC-019-builder-work-data-cleanup.md)に従う。アプリデータ、保護成果物、共有データ、復旧情報を「キャッシュ」と推定して削除対象にしない。

## 例外・禁止・安全境界

- アンインストール、共有解除、アプリ間参照の解除、データ削除、ソース／Project削除、復旧情報の削除、外部送信を別の操作として扱う。一つの承認を他の操作へ流用しない。
- ファイルやアプリの関連が推定されるだけでは、削除・閲覧・権限の根拠にしない。所有者、参照先、Revision、現在のPermission／Policy、削除対象を正規のOS経路で再確認する。
- 削除操作の結果、部分成功、失敗、結果不明と実際の回復可能範囲を正確に報告する。削除可能性や復元可能性を推測で保証しない。
- 本決定はDEC-014のWorkspace／名前付きチェックポイント保護、DEC-019の未完了作業・共有・復旧データ保護、DEC-026の検証・復元方針を弱めない。Workspaceやチェックポイントの削除・復元、Taskの再実行は本決定の対象外であり、各既存規則に従う。
- 試用成果と正式導入済みアプリの区別はDEC-036（試用・導入）に従う。共有・公開の範囲と外部送信承認はDEC-036（共有・公開）に従う。保守更新の承認（DEC-035）はアンインストールやデータ削除を許可しない。
- 本決定は実装済み、0.4 release／workstream activation、Workstream owner変更を意味しない。0.1 M30、0.2 M21、0.3 Acceptance、0.4 Entry Gate／Integration Owner checkpointを変更しない。未公開仕様をPublicリポジトリへ転載しない。

## 未確定・暫定事項

データ分類・所有・参照をどのように登録するか、保護Artifact／Manifestの具体的な範囲、保持期間・容量上限、削除候補と共有参照を示すUI、取り消し可能な削除と完全削除の実装差、バックアップ・復旧方式、再インストール時のPackage検証、対応アプリ・Provider、実装Ownerと適用版は未確定である。ここで数値や実装契約を確定しない。

## Roadmapとの関係

- **0.4+:** DEC-015のRole切替、DEC-019の作業領域整理、DEC-035／036の試用・導入・更新・共有、DEC-040の開発Project長期保存と整合する将来のアプリライフサイクル方針として扱う。0.4のGate、Acceptance、完了要件は変更しない。
- **0.5–1.0:** 開発者・配布基盤およびデータ管理の将来検討で参照する。版別の割当てや完了条件は確定しない。
- 正式な実装には、該当Versionの承認済み仕様、現行Workstream registry／owner、Permission／Policyと既存Gateを別途満たす必要がある。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | ユーザーがbuilder製アプリのアンインストール・関連データ削除方式Cを確定。通常のアンインストールはアプリ本体のみを削除し、ユーザーデータ、設定、保護されたソースコード、開発Project／履歴を原則保持する。完全削除は対象・共有参照・影響・回復可否を説明し、対象ごとの明示選択・承認と正規OS Permission／Policyの下で行う。実装詳細、保持期間、UI、版割当ては暫定とした。 |
