# DEC-016 — builderの自律実行範囲と成果のリスク別完了判定

**状態:** CONFIRMED（基本方式。閾値・実装詳細は未確定）  ／PROVISIONAL（Nagi内Role切替の実装詳細・将来専門Agent化）
**最終合意日:** 2026-11-04  
**適用候補:** Nagi OS 0.4のBuild／Execute作業要件（初期Nagi Role切替C。後続Agent配置は未確定）
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-007](DEC-007-roadmap-alignment.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-015](DEC-015-builder-role-review.md)、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-075](DEC-075-builder-app-trial-and-installation.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)

> **名称・位置づけ訂正（2026-11-05）:** 以前の「Jev」という内部名称は外部のTypeSafe AI製品との混同だったため撤回した。**`builder`は確定したプログラミングRole名**で、初期はNagi自身がRoleを切り替えて実行する方式Cで確定（DEC-015）。将来の独立Agent対応と具体的なRuntimeは未確定。選択済みの動作・安全条件を記録するが、誤解を根拠にAgent構成やモデル選定を確定しない。詳細は[DEC-024](DEC-024-external-jev-reference-and-builder-label-correction.md)。

## 目的とユーザー体験

初回0.4でNagiが切り替えるプログラミングRole `builder`の動作について、作業のたびにユーザーの操作を要求せずにコード修正・テストを進められるようにしつつ、検証できない変更や重要な変更はユーザーが最終判断できるようにする。ユーザーの窓口・報告者はNagiとする。

## 確定した仕様

### 1. builderの初期自律実行レベル — 選択B

- **builderの初期標準は「修正・テストまで自律実行（B）」**とする。
- **Nagiの計画・初回承認範囲:** Nagiが計画を立て、Workspace、対象Resource、許可Action、Resource権限、期限、目的その他のScopeを明示した最初の承認範囲を提示する。ユーザーが実行前に承認し、builderは承認範囲とOSの現在のPermission / Policyの共通部分だけを使う。Scopeの拡張が必要になった場合はbuilderが停止し、Nagiが追加承認を求める。
- **承認範囲内の反復実行:** builderは承認済み範囲内で**調査 → コード修正 → ビルド／テスト → 必要な再修正**を進められる。承認済みの可逆な下位Actionごとに不必要な重複確認は求めない。
- **証拠と報告:** builderは変更差分、テスト結果、部分失敗、未検証箇所、未解決の残課題、provenance等の確認可能な証拠をNagiへ渡し、Nagiがユーザーへ報告する。
- **Gitのcommit・pushはBの標準権限に含めない。** これらを実行可能にするには、別途の明示的な許可と、その時点のOS Policy／Permission検査が必要である。選択C（Git操作まで既定で自律実行）は採用しない。

### 2. builder成果の最終確認方式 — 選択C

- **最終確認方式は「リスクに応じて切り替える（C）」**とする。
- **通常の修正:** 事前に承認されたTask／Workspace／Action範囲内であり、合意した受入条件とテスト・検証証拠を満たし、重大な未解決リスクがない場合は、Nagiが**作業結果を自動で完了判定**し、変更内容と根拠をユーザーへ報告できる。毎回の最終承認を必須とはしない。
- **確認が必要な修正:** 重要な変更、未承認の範囲拡大、テスト失敗、検証不能・証拠不足、結果不明、その他Policyが確認を要求するケースでは、Nagiは自動で成功・完了と判定せず、適切な**ユーザー確認／追加承認**に回す。
- builderが承認範囲内で再修正・再テストを行うことは上記のBに含まれる。ただし、許可された試行範囲内で受入条件を満たさない場合は、未解決であることを報告する。再試行によって権限・対象範囲を自動拡大しない。
- 完了判定は**Taskの作業結果に関する判定**であり、Git commit・push、リリース、本番反映、外部送信、未承認の変更の自動許可や実行を意味しない。これらは別のAction／Permissionとして扱う。
- builderが新規アプリを作った場合、開発Taskの自動完了判定や試用成功は採用・正式インストール・永続登録を意味しない。採用判断、正規Package検証、現行Permission／Policy、正式導入への別途承認は[DEC-075](DEC-075-builder-app-trial-and-installation.md)に従う。
- Nagiは未完了、部分成功、検証不能、停止・取消、Undo不能な副作用を、全面成功として表示しない。

### 3. builderの自動再試行方式 — C（CONFIRMED）

- builderはエラー原因、改善の見込み、検証証拠、残りの承認済みScopeとBudgetに応じて、コード修正とテストの自動再試行を判断する。
- 同じ失敗が続く、改善が見込めない、安全性に疑義がある場合は上限前でも早期停止し、Nagiが残課題と試行結果を報告する。
- Retry回数、実行時間、Resource利用には有限の上限を設け、承認範囲・Permissionを超えない。権限拒否、期限切れ、取消、未解決の競合、結果不明の非冪等Actionは、無断の再試行や権限拡大で回避しない。
- 試行中のテスト失敗は再修正の契機となり得るが、未解決のまま最終完了扱いにしない。Task／Attempt／Activityを記録し、副作用の重複を防ぐ。

**PROVISIONAL:** 初期Retry上限「3回」は提案値であり確定数値ではない。初回を含む数え方、時間・Resource上限、改善判定基準は0.4 Implementation Specificationで決定する。

### 4. 作業中の追加指示・別件依頼 — 分類方式C（CONFIRMED、2026-11-05）

- Nagiは新しい指示が既存`builder` Taskの小さな変更か独立した開発依頼か、または競合・優先変更・大幅なScope変更かを識別する（[DEC-015](DEC-015-builder-role-review.md)）。
- **承認済みScope内の小さな追加**は、作業計画・確認項目を適切に更新して同じTaskで扱える。変更前に生成したPatch・Build／Test証拠を無条件で再利用せず、更新後の目標と現行Resource Revisionに対して再検証する。
- **別件は別Task**としてContext・Workspace・Action Scope・承認状態を分離し、初期0.4では逐次実行／Queueを基本とする。別件の新規受付は、元Taskの承認を新TaskのAction権限へ流用する根拠にならない。
- **大幅な要求変更・競合・優先変更**では、対象への影響と作業済み成果を示して、必要なユーザーの選択・追加承認を得る。未承認のResource追加やAction種別拡大、失効したPermission、競合するPatchを既存の自律実行Bや完了判定Cで正当化しない。
- 新しい指示の受付だけを理由に元Taskを取消・再実行・完了扱いにしない。進行中の作業が新要求と矛盾するときは安全な境界で待機し、隔離成果・証拠・Activityを保持してから判断する。Git commit／pushやリリース操作は別途明示許可が必要。

**PROVISIONAL:** 分類判定の具体基準、計画Version・Queue順序・Priority UI、停止安全点、再検証範囲などの実装詳細と0.4 Acceptance値は別途決める。既存Release GateとOwnerは変更しない。

## 例外・禁止・安全境界

- 決定論的な検査・テスト等の**再現可能な証拠**で完了を判定する。builderまたはNagiの自己申告だけでは受入条件を満たさない。
- builder RoleのTaskには承認範囲に必要な最小権限だけを適用し、その範囲を越えて行動できない。副作用は型付きPublic ActionとOS／Providerの現行Permission / Policyを通す。Model、Agent名、習慣、過去の承認によって権限が拡大しない。
- ユーザーは必要に応じてPause／Cancel／Stopでき、Stop伝播、Activity／Task Log、Revision確認、Waybackの実際の可逆性の条件は既存仕様に従う。
- 重大操作で新たな最終承認が必要な場合はDEC-011の画面上の明示的な最終承認を尊重する。
- 本決定は**実装済みを示さず**、0.1〜0.4の既存Release Gate、0.3 Acceptance、0.4 Entry Gate／Integration Owner checkpoint、Workstream Activationを変更しない。

## 未確定・暫定案

- 「通常」「重要」のRisk taxonomy、具体的な対象操作・閾値・承認UI、ユーザーが修正案を却下した場合の遷移。
- 完了判定に必要なテスト種別、検証証拠の形式、失敗／部分成功／Review Requiredの状態遷移と表示詳細。
- builderの具体的なRetry回数（3回は暫定候補）、初回の数え方、時間／リソース上限、改善判定、Fallback、ローカルModel／Runtimeの選定。
- commit・pushに別途許可を与える具体的なScopeやUI（本決定では自動許可しない）。

## Roadmapとの関連

- **0.3:** 単一NagiのPublic Action、Permission、Task、Activity等の基盤に依存し、0.3のAcceptanceを変更しない。
- **0.4:** DEC-015で確定した初期Nagi一体の`builder` Role切替C、1ユーザー／1端末・Task単位のScope分離を前提に、builderの自律実行範囲とNagiの最終完了判定方式を追加する。Research／Review等のAgent追加や並列実行は本決定の範囲外。

## 決定履歴

> 2026-11-05: プログラミングRole名を `builder` と確定。初期はNagiのRole切替C、将来の独立Agent化は未確定（DEC-015／DEC-024）。

| 日付 | 内容 |
|---|---|
| 2026-11-04 | ユーザーがbuilderの初期自律実行レベルB（修正・テストまで自律、commit・pushは標準権限外）を確定 |
| 2026-11-04 | ユーザーが成果の最終確認方式C（通常はNagiが自動完了、重要変更・検証失敗等はユーザー確認）を確定 |
| 2026-11-04 | ユーザーがbuilderの自動再試行方式Cを確定。数値3回は暫定候補として記録 |
| 2026-11-04 | Nagiが計画と初回承認範囲を管理すること、Resource権限、部分失敗・残課題の報告を明記 |
| 2026-11-05 | 追加指示・別件開発依頼の扱いを方式Cで確定。承認Scope内の軽微変更はTask内計画更新、別件は独立TaskでQueue、競合・大幅変更・優先切替は確認・必要な追加承認とRevision再検証を行う |
