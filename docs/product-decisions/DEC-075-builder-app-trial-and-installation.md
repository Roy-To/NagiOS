# DEC-075 — `builder`が作成した新規アプリの試用・正式導入

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（方式C：安全な環境で試用し、採用判断後に別途承認を経て正式導入）／PROVISIONAL（試用・検証・Package導入の実装方式、具体的な適用版）
**最終合意日:** 2026-11-05
**適用候補:** Nagi OS 0.4+ の`builder`による新規アプリ作成・導入ライフサイクル（版別実装時期は未確定）
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-015](DEC-015-builder-role-review.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-035](DEC-035-builder-created-app-maintenance-updates.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md), [DEC-041（アンインストール・関連データ削除）](DEC-041-builder-app-uninstall-and-data-deletion.md)

## 目的とユーザー体験

`builder`が作成した新しいアプリを、正式にインストール・登録する前に安全に試せるようにする。Nagiは機能、確認できた検証証拠と未検証事項、アプリが要求する権限を説明する。ユーザーが採用を選んだ場合に限り、正規Package検証、OSのPermission／Policy確認、正式導入への別途承認を経てインストール・永続登録する。導入後の保守・更新はDEC-035の別ライフサイクルとして扱う。

## 確定した仕様 — 選択C

1. **試用と正式導入を分ける。** builderが作ったアプリは、正式インストール・永続登録と区別できる候補として扱う。Build／Testの成功やbuilder Taskの完了だけでは正式導入しない。
2. **安全に対応できる環境で試す。** 試用は、対象アプリを安全に実行でき、現行Permission／Policyと承認済みScopeを強制できる環境でのみ行う。適切な環境がない、または安全性・動作を確認できない場合は試用を実施済みと扱わず、その制約をユーザーへ伝える。試用環境の具体的な隔離機構は本決定で指定しない。
3. **採用判断に必要な情報を示す。** Nagiはユーザーの採用判断より前に、アプリの機能、試用した範囲、確認できた検証証拠、未検証事項・失敗、要求するPermission／Capabilityを説明する。証拠のない安全性・互換性を推定しない。
4. **ユーザーが採用を選んだ場合だけ次へ進む。** 採用しない、または判断が保留されている間は、正式インストールや永続登録を行わない。採用判断は、インストールや権限付与そのものへの承認ではない。
5. **正式導入は別の承認を要する。** 採用後、正規Package検証と現行OS Permission／Policy確認を行う。導入対象Package、検証結果、要求Permission、永続登録の内容を提示し、正式インストール・永続登録に対する別途の明示承認を得てから、OSの正規導入経路で実行する。既存Policyがさらに厳しい承認を要求する場合はそれにも従う。権限が拒否されたり、Package検証・Policy確認・承認のいずれかが満たされない場合は導入しない。
6. **結果を正確に報告する。** Nagiは試用、Package検証、Permission／Policy判定、ユーザーの採用判断、正式導入の各状態を区別して伝える。試用結果をPackageの正規検証結果に読み替えず、失敗・未確認・部分成功を成功として表示しない。

## 例外・禁止・安全境界

- 採用判断、開発Taskの事前承認、builderの自律実行Scope、コード変更の反映承認、Role切替、Build／Test成功は、正式インストール、永続登録、Permission／Capability付与を許可しない。
- 既存のOS Permission／Capability／Policy、正規Package導入経路、Actionごとの認可を迂回しない。builderまたはNagiの判断でPermissionを発行・拡大しない。
- 試用できない場合に、通常環境での実行を試用として代用したり、未検証のまま安全と説明したりしない。
- 試用成果、Package、Taskデータの保管・削除はDEC-019等の既存データライフサイクル規則に従う。試用の実施を理由にユーザーデータを無断削除しない。 導入後の通常アンインストールと完全削除は[DEC-041](DEC-041-builder-app-uninstall-and-data-deletion.md)に従う。
- 本決定は機能の実装済みを意味せず、0.3 Acceptance、0.4 Entry Gate、Integration Owner checkpoint、Activation、Workstream所有権・実装許可を変更しない。Publicリポジトリには未公開仕様を転載しない。

## 未確定・暫定案

- 試用可能な実行環境・隔離方式、対象アプリ／Resourceに対する実行時制限、未対応環境でのユーザー表示。
- Build／Test／試用時の証拠形式、正規Package検証の項目・基準、署名やPackage provenanceの扱い。
- Permission要求の表示・差分確認、採用判断と正式導入承認のUI、拒否後の候補状態・再試行方法。
- 試用Package・実行データの保持期間、容量上限、削除条件、導入後の更新・削除・失効フロー。
- 本仕様を受け入れる具体的なリリース版、Workstream、Target Runtime Acceptance条件。

## Roadmapとの関連

- **0.4+:** builderが作ったアプリを試用候補から正式導入へ進める際の製品境界として扱う。具体的なリリース割当てとAcceptanceは未確定であり、0.4のGateを開くものではない。
- **DEC-017:** 第三者アプリの開発者環境・Package方針を定める。builder生成アプリの試用・ユーザー採用・正式導入の段階は本DECで定める。
- **DEC-015／016／018:** Role実行、開発Taskの完了判定、コード差分の反映を定める。これらの承認・自動完了は正式なアプリ導入を含まない。
- **DEC-035:** 正式導入済みアプリの保守・更新を定める。本DECは未導入アプリの試用・初回導入を定める。
- **DEC-026:** リスク別の一般的な検証・保護方針に対し、本DECの明示的な採用判断と正式導入への別途承認を維持する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | ユーザーが方式Cを確定。安全に対応できる環境で試用し、Nagiが機能・検証証拠・要求権限を説明する。採用判断後に限り、正規Package検証、OS Permission／Policy確認と正式導入への別途承認を経てインストール・永続登録する。実装方式・適用版・0.4 Gateは未確定／維持。 |

## 番号整理履歴（2026-11-16）

旧DEC-036をDEC-075へ改番。異なる仕様が共有していた番号を分離し、元の合意日・CONFIRMED／PROVISIONAL・製品決定・履歴を保持した。旧番号との対応は[PR統合記録](PR-107-129-INTEGRATION-2026-11-16.md)を参照。
