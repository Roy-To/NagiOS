# DEC-040 — `builder`製アプリの開発プロジェクト長期保存

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（ローカル基本の長期保持、アプリ／利用者データ／Canonical Memoryとの分離、条件付き整理、再発見・再開・端末移行の製品目標）／PROVISIONAL（ファイル形式・保持期間・容量上限・Git採用・移行形式・適用版・技術実装）  
**最終合意日:** 2026-11-05  
**適用候補:** Nagi OSの`builder`製アプリと開発Projectのライフサイクル（実装時期は未確定）  
**関連:** [DEC-003](DEC-003-memory-classification.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-015](DEC-015-builder-role-review.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-019](DEC-019-builder-work-data-cleanup.md)、[DEC-022](DEC-022-builder-interrupted-task-recovery.md)、[DEC-035](DEC-035-builder-created-app-maintenance-updates.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md), [DEC-041](DEC-041-builder-app-uninstall-and-data-deletion.md)

## 目的

`builder`が作成したアプリを将来改修・検証できるよう、正式な開発Projectをアプリの実行物やWorkspace復元状態と区別して保護する。ユーザーが「前に作ったアプリを改良して」と依頼した際に、適切なProjectを発見し、現行のPermissionと承認Scopeの範囲で再開できることを目指す。

## 確定した仕様 — 選択C

- **正式な開発Projectをローカルで長期保持:** アプリの将来の改修・検証・再構築に必要なProjectを、Nagi端末内で長期保持することを基本とする。対象にはソースコード、必要な設定、依存関係・再構築情報、Version履歴、将来の作業・検証・復旧に必要な記録を含む。
- **保護対象を整理対象から分離:** 正式な開発Projectと、将来の改修・検証・復旧に必要な記録は、DEC-019のTask一時データ整理やキャッシュ整理の対象に混ぜない。再生成可能なCache、不要と判定できるBuild生成物、一時作業領域は、所有・参照・使用状態・Permission・保持Policy等の既存安全条件を満たす場合にだけ整理できる。不明な場合は保持または保留する。
- **データの意味を分離:** 開発Project、インストールされたアプリ本体／Package、アプリが使用するユーザー実データ、NagiのCanonical Memoryは別の管理対象とする。Projectの保存や検索を、アプリ導入・データ複製・Memoryへの取り込みとみなさない。
- **再発見とScope付き再開:** ユーザーが以前作成したアプリの改良を依頼したとき、Nagiは利用権限のある保護済みProjectを発見して候補を示し、ユーザーの意図する対象を確認する。開発の再開時には現在のUser／Profile／Permission／Policy、対象Project、Resource Revision、Taskの承認Scopeを適用し、Roleや過去の承認だけから権限を引き継がない。Projectが見つからない、必要情報が欠ける、Scopeが不明な場合は推測で別Projectを編集せず確認する。
- **端末移行可能性:** 将来、ユーザーが開発Projectと必要な開発情報を別端末へ移行できることを製品目標とする。移行元の保持、移行対象の選択・事前確認、移行先での現行Permission／Policy再評価を尊重する。
- **アンインストールと分ける:** 通常のアプリ・アンインストールでは開発Projectとその履歴を原則保持する。Projectを完全削除する場合は対象別に内容、共有参照、影響、回復可否を確認し、明示選択・承認を得る（[DEC-041](DEC-041-builder-app-uninstall-and-data-deletion.md)）。
- **整理と復旧:** Project内の保護対象を削除・上書きする場合は、別途の明示的な操作と適用可能な承認・復旧条件を要する。中断・失敗・状態不明のTaskはDEC-022とDEC-019に従って、再開・検証・復旧に必要なProjectと記録を保護する。

## 禁止・境界

- 開発Projectの保存は、Git commit／push、Cloud backup、外部送信、第三者との共有・公開を自動許可しない。それぞれ現行Permission／Policyに従った別の明示的操作・承認を必要とする。
- DEC-014のWorkspace snapshot／checkpointはWorkspace復元用状態であり、ソースコード本文や開発Projectのバックアップではない。Workspace状態、アプリ実行物、開発Project、ユーザー実データの保存範囲を混同しない。
- 開発Projectの保護は、インストール済みアプリの更新・正式導入を許可しない。新規アプリの試用・導入はDEC-036、導入済みアプリの保守・更新はDEC-035、隔離編集と条件付き反映はDEC-018に従う。
- 本決定は0.1／0.2／0.3 Acceptance、0.4 Entry Gate／Integration Owner checkpoint、既存Workstream・AGENTS.md・実装所有権を変更せず、実装開始・Release Activationを認可しない。Private仕様をPublicリポジトリへ転載しない。

## 未確定・暫定事項

Projectの具体的なディレクトリ・Schema・Package形式、Version履歴を実現するGit採用有無、差分・復旧情報の形式、保存期間、容量上限と逼迫時の扱い、削除・保護解除UI、Project発見・検索の方法、移行Package・暗号化・競合解決・検証、対応する端末範囲、技術実装および適用VersionはPROVISIONALとする。測定・Security／Privacy検証と別途の製品判断を経て決め、本決定で数値を補わない。

## 関連決定との役割分担

- [DEC-014](DEC-014-workspace-state-checkpoints.md)はWorkspaceの復元状態を定め、Project本文の保存を定めない。
- [DEC-019](DEC-019-builder-work-data-cleanup.md)はTask一時データ・Cache等の状態別整理を定める。正式Projectは保護対象として除外する。
- [DEC-022](DEC-022-builder-interrupted-task-recovery.md)はTask中断後の状態復旧を定める。Projectの長期保管とは別である。
- [DEC-035](DEC-035-builder-created-app-maintenance-updates.md)は正式導入済みアプリの修正・更新を定める。開発Projectの保持は更新Packageの適用を許可しない。
- [DEC-017](DEC-017-third-party-developer-ecosystem.md)は開発者・配布環境の製品方針を定める。本決定のProject保存方針はその将来の開発体験を支える。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | ユーザーが方式Cを確定。正式なbuilder製アプリ開発Projectをローカル基本で長期保持し、再生成可能なCache／不要なBuild生成物／一時領域のみ既存の安全条件に従って整理する。アプリ本体・ソース・実データ・Canonical Memoryを分離し、Projectの再発見・Scope付き再開と将来の端末移行を製品目標とする。Git commit／push・Cloud backup・外部送信・共有を保存だけで許可しない。ファイル単位の形式・保持期間・容量・Git・移行形式・適用版は未確定とする。|

---

**End of Decision**
