# DEC-035 — `builder`製アプリの保守・検証付き更新

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（保守・更新方式C：Nagiの修正提案→隔離修正・検証→ユーザー確認後に正式更新）／PROVISIONAL（検知方式・更新機構・数値条件・適用版）  
**最終合意日:** 2026-11-05  
**適用候補:** Nagi OS 0.4+ の`builder`体験および将来のアプリ配布・更新環境（実装時期は未確定）  
**関連:** [DEC-011（Task承認と表示）](DEC-011-task-approval-progress-ui.md)、[DEC-015（builder Role）](DEC-015-builder-role-review.md)、[DEC-016（builder自律実行）](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-017（アプリ開発・配布）](DEC-017-third-party-developer-ecosystem.md)、[DEC-018（隔離修正・反映）](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-026（検証・復元）](DEC-026-ai-change-verification-and-recovery.md)、[DEC-036（初回試用・導入）](DEC-036-builder-app-trial-and-installation.md)、[DEC-040（開発Project保存）](DEC-040-builder-app-development-project-retention.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)、[DEC-036（builder製アプリ共有・公開）](DEC-036-builder-app-sharing-and-publication.md), [DEC-041](DEC-041-builder-app-uninstall-and-data-deletion.md)

## 目的とユーザー体験

`builder`が作成し、ユーザーが正式に導入したアプリを、安全かつ分かりやすく保守できるようにする。不具合や改善点が見つかっても、Nagiが無断でアプリ本体を書き換えたり正式版を更新したりせず、修正内容と安全性をユーザーが把握して採用できる体験を目指す。

## 確定した仕様 — 選択C：修正提案・隔離検証・確認後更新

1. **不具合を発見したら修正を提案する。** Nagiはユーザーからの不具合報告、または既存のPermission／Privacy／Task Scopeの範囲で利用できる診断情報に基づき、問題の可能性と対応案を説明・提案できる。発見したことだけを理由に新しい診断権限や修正・更新権限を得ない。無断の常時監視やデータ閲覧を本決定で認可しない。
2. **`builder`は許可された範囲で隔離修正・検証する。** ユーザーが必要な開発Task／対象・Actionの範囲を承認した後、初期はNagi自身の`builder` Roleで調査・修正・Build／Test・必要な再修正を行う。原本や現行インストール済みアプリを保護した隔離作業領域を使用し、DEC-016／018のScope、Revision、Permission、検証証拠、原本保護、有限Retryの要件を維持する。
3. **正式更新前に内容を説明し、ユーザーが採用を決める。** Nagiは修正の目的、主な変更点、既存機能への影響、Build／Test結果、未検証事項、必要な権限、データ／設定への影響、実際に利用可能な復旧手段を分かる形で提示する。**正式版への適用・更新インストール・永続登録は、ユーザーが内容を確認して更新を承認した場合に限る。** `builder`の成功申告やテスト成功を自動更新許可とみなさない。
4. **正規のアプリ更新経路を利用する。** 正式更新時にはアプリpackage／manifest、適用対象、現行Revision／Version、導入先、現行Permission／Policyの検証を行い、既存の正規インストール・更新契約に従う。作業中のソースコードをDEC-018の条件付きで自動反映できる場合でも、**インストール済みアプリの更新・実行・本番反映への許可とは別**に扱う。パッケージ検証機構や更新契約は既存実装仕様とGateを優先する。
5. **アプリデータ・設定を可能な限り保持する。** 更新時は既存ユーザーデータ、個人設定、互換性と必要な移行を確認し、可能な範囲で保持する。破壊的変更・データ移行・新しい権限付与が必要な場合は影響を示して別途明示承認を求める。完全な互換性や無損失を保証せず、不明・検証不能なら正式更新を保留する。
6. **更新失敗時には実際に復元可能な範囲で旧版へ戻す。** 可能であれば更新前の既知の正常版と必要な復旧データを保護する。更新に失敗した場合、事実に即して旧版への復帰またはユーザーによる復旧判断を支援する。アプリ実行ファイルの復帰と変更済みデータ・外部への副作用の完全巻戻しを同一視しない。復旧できなかった範囲、部分成功、結果不明を正確に記録・表示する。
7. **操作と結果の透明性を維持する。** 修正提案、承認Scope、隔離修正、検証結果、正式更新の承認・実行、失敗・部分成功、復旧状態をTask／Activityに紐付けて表示できるようにする。ユーザーは既存のTask停止／承認取消規則を利用できる。

## 例外・禁止・安全境界

- **無断の自動修正・自動更新は標準としない。** Nagiによる問題検知や提案は、コード変更、packageの実行、更新インストール、権限変更を無条件に許すものではない。
- 開発Projectの長期保存（DEC-040）はソース・開発情報の保持であり、インストール済みアプリの正式更新とは別のライフサイクルである。
- 正式更新の承認はアンインストールやユーザーデータ・Project削除を許可しない。通常アンインストールと完全削除は[DEC-041](DEC-041-builder-app-uninstall-and-data-deletion.md)に従う。
- 新規アプリの**試用・採用判断・初回正式登録**と、既存アプリの**保守・更新**を区別する。初回試用から正式導入までの確定仕様は[DEC-036](DEC-036-builder-app-trial-and-installation.md)に従い、本DECは導入前の試用環境や初回インストール権限を追加するものではない。
- ローカル低リスク変更の条件付き自動反映（DEC-018）と、正式に導入したアプリのインストール済みVersion更新を混同しない。
- 追加Permission、外部送信、Git commit／push、公開・Store配布、システム更新、本番環境への反映はそれぞれ独立のScopeと現行Policy／Capability／承認に従う。アプリ共有・公開はDEC-036に従い、保守Taskや正式更新の承認から自動的に許可されない。
- 対象アプリが非対応である、検証できない、署名・manifest・Version整合性を満たさない、原本やデータの保護を実証できない場合、成功・安全・Rollback可能と偽らず適切に保留・確認待ちへ移す。
- 本決定で0.1 M30、0.2 M21、0.3 Target Acceptance、0.4 Entry Gate／Integration Owner checkpoint、既存Workstream／AGENTS.md／実装所有権を変更しない。Target Runtimeの更新・Rollbackが実装・検証済みとみなさない。Private仕様をPublicへ転載しない。

## 未確定・暫定案

- 診断・不具合検知のデータ種類、通知頻度、同意UI、保存期間、Privacy設定、収集／解析方法。
- 隔離試用と正式更新の具体的UI、Task Scope承認方式、変更内容・権限の提示方法、どの更新で追加承認が必要かのRisk taxonomy。
- 更新packageの形式・署名・配布元・Version pinning・自動取得・更新確認・インストール処理・依存関係／互換性検査。
- ユーザーデータのバックアップ・Schema migration・設定継承・旧版への復帰／Forward recovery条件、非可逆変更や部分失敗の扱い。
- 検証ケースと証拠形式、成功・安全・復旧可能性のAcceptance基準、実装Releaseと予算・リソース上限。

## Roadmapとの関連

- **0.2:** DEC-017のapp SDK、manifest、package検証・インストール基盤の既存0.2-M21契約をそのまま尊重する。既存Gateの前倒し・新規Workstream開始許可ではない。
- **0.3:** Personal Agentによる正規Public Actionと承認・Task／Activity管理の既存境界を維持する。
- **0.4+:** Nagi一体の`builder` Role切替（DEC-015）を土台に、アプリ保守提案と隔離検証・ユーザー承認後の正式更新を将来の製品体験として定める。0.4でアプリ更新機構まで完成させる義務を新設しない。
- **1.0に向けて:** DEC-017の開発者・配布環境の製品目標と整合させる。Store専用配布・自動更新を前提としない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | `builder`製アプリの保守・更新方式Cをユーザーが確定。Nagiによる修正提案、許可Scope内での隔離修正・検証、変更内容と権限・影響を確認したユーザーの承認後に正式更新し、可能な範囲でデータ／設定を維持・旧版復旧する |
