# DEC-037 — Human Stepの差し戻し・再提出・期限管理

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（思想㉑A：差し戻し・修正・再提出をWorkflowの正式な状態／経路として扱い、承認済みScope内の安全な修正を許し、権限・手順変更はユーザー確認。思想㉒A：期限、控えめな自動リマインド、明示許可先へのエスカレーション）／PROVISIONAL（状態Schema、期限・頻度、通知経路、UI、保持詳細、適用版）  
**最終合意日:** 2026-11-05  
**対象:** DEC-034のHuman-in-the-loop Workflowにおける差し戻し、修正、再提出、期限、リマインド、エスカレーション。実装版は未確定  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-003](DEC-003-memory-classification.md)、[DEC-008](DEC-008-proactive-behavior.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-022](DEC-022-builder-interrupted-task-recovery.md)、[DEC-028](DEC-028-deterministic-workflows-from-habits.md)、[DEC-029](DEC-029-scheduled-event-triggered-workflows.md)、[DEC-032](DEC-032-workflow-safe-retry-and-recovery.md)、[DEC-034](DEC-034-human-in-the-loop-workflow-steps.md)、[Public 0.2 Platform Foundation仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)

## 目的

Human Stepで担当者が修正を求めたり、提出内容を直して再提出したりできる、追跡可能で安全なWorkflow経路を定める。期限と通知は人の対応を支援するために使い、期限経過や通知の送信をStep完了・権限変更と取り違えない。

## 確定した仕様（思想㉑・選択A）

1. **差し戻し・修正・再提出は正式なWorkflow経路:** Human Stepは、対応待ち、作業中、差し戻し／修正待ち、再提出済み、確認待ち、完了、保留、失敗等の状態を持つWorkflow上の正式な経路として扱える。差し戻し理由、必要な修正、提出された入力・成果、再確認の状態をTask／Workflow上で対応付ける。具体的な状態名・SchemaはPROVISIONAL。
2. **担当者の差し戻しコメントは権限にならない:** 担当者からのコメント、依頼、差し戻し、再提出要求だけでは、Workflowの承認範囲、Action、Resource、共有先、担当権限、手順を追加・変更しない。元の承認済みHuman Step／Task内で、許可済みのResourceとActionを用いた安全な修正・再提出だけを続けられる。許可範囲が不明、対象やRevisionが競合、Permission／Policyが拒否または失効した場合は保留し、推測で進めない。
3. **Workflow定義や権限の変更は別の明示承認:** 正式なWorkflow手順、条件、共有対象、担当範囲、Permission、Capability、Policy、Action Scopeを変更・拡張する場合は、担当者のコメントや再提出を承認とみなさず、必要な変更内容をユーザーへ示して明示的な承認を得る。承認が得られるまで変更後の経路・権限に依存する処理を行わない。OS Policyによる拒否はユーザー承認で覆せない。
4. **再提出後に現行状態を再検証:** 再提出や確認完了を受け取った後、次のActionへ進む前に、実行主体の本人確認、現在のUser／Session／Profile、Permission／Capability／Policy、Task／Workflowの有効・停止・取消状態、承認済みScope、対象ResourceとRevision、提出物の由来と必要な入力条件を再確認する。すべて有効な場合のみ、既に承認された後続Stepへ進む。Revision不一致、結果不明、本人確認不能、権限失効、取消・停止は保留し、古いSnapshotで人の変更を上書きしない。
5. **業務上の差し戻しとAction承認を区別:** 差し戻し、修正、再提出、レビュー完了はHuman Stepの状態を示す。これらはOS Permission／Capability／Policyの変更や、高リスクAction・外部送信・不可逆操作に必要な独立した承認を代替しない（DEC-001／011／034）。
6. **履歴と結果を正確に記録:** 差し戻し・再提出の主体と時点、状態遷移、共有を許された対象、確認・検証結果、後続Actionの結果を、既存Privacy／Retention／Permissionに従いTask／Activityから追跡できるようにする。必要最小限の参照・要約を記録し、不要な機密本文、認証情報、他者の無関係なProfile／Task情報は複製・開示しない。未送信・未達、部分成功、未検証、結果不明を成功と表示しない。

## 確定した仕様（思想㉒・選択A）

1. **Human Stepに期限を設定可能:** WorkflowのHuman Stepには期限を持たせられる。期限は対応の目安と進捗表示に用いる。期限が過ぎてもStepを完了扱いせず、担当者を自動変更せず、Action ScopeやPermissionを拡大せず、未承認の後続Stepを開始しない。
2. **控えめな自動リマインド:** 有効なHuman Stepについて、許可された通知経路と設定の範囲で控えめな自動リマインドを行える。通知は必要最小限の内容を、当該Stepを受け取る許可のある相手にのみ伝える。単なるリマインドはTask／Workflowの期限延長、再開、再割当、承認、完了を意味しない。
3. **明示的に許可された相手へのエスカレーション:** 期限超過等の条件に応じてエスカレーションを行う場合、送信先のIdentity、共有する内容・Resource、目的、範囲、タイミング／条件が明示的に許可されている必要がある。元のWorkflowで許可された共有範囲を超えて情報を広げず、受信者のIdentityと現在のPermission／Policyを確認する。許可先や共有範囲が不明なら送信せず保留する。エスカレーションは通知であり、自動担当変更、代理承認、権限移譲、Scope拡張を行わない。
4. **停止・取消・通知抑制に従う:** ユーザーの停止、取消、無効化、通知抑制、許可撤回を現行状態で確認する。停止済み・取消済みWorkflowやHuman Stepをリマインド、Retry、期限処理だけで再開しない。通知を送信できない、または通知先・権限を確認できない場合は、Stepを完了扱いせず、通知不能または保留の状態を正確に示す。
5. **各通知時に再確認:** 送信時点でStepの有効性、期限・通知条件、送信元と宛先のIdentity、許可された共有内容、現行Permission／Policy、Task Scope、停止・取消・通知抑制を確認する。期限やイベント発火のみで通知権限が生じることはない。
6. **履歴:** 期限状態、リマインド／エスカレーションを試みた事実、実際の送信・失敗・未達を、Privacy／Retentionに従う必要な範囲でTask／Activityに記録する。送信結果が確認できない場合に送信済みと主張しない。

## 既存決定・実装Gateとの整合

- **DEC-034:** 本DECはHuman Stepの差し戻し・再提出・期限・リマインド・エスカレーションを具体化する。DEC-034の思想⑲／⑳、他ユーザーの範囲限定割当、共有元／受取側双方のIdentityとPermission、最小限共有、完了後再検証、Action承認との分離を維持する。担当者の依頼が新規権限や正式なWorkflow改訂になることはない。
- **DEC-001／003／008／011:** 現行OS Permission／Policy、機密分類・所有範囲、控えめな通知、Task状態とActivityの正確な表示に従う。通知・リマインドのために全MemoryやTaskを共有しない。Role、担当、期限、通知自体は認可ではない。
- **DEC-022／032:** Human Stepの差し戻し・再提出は、クラッシュ／再起動後の復旧や一般ActionのRetry許可ではない。結果不明、不可逆操作、取消・停止等では自動再実行せず、安全に停止して確認する。期限経過を復旧・Retry・強制再開に転用しない。
- **DEC-028／029:** Workflowの正式保存・改訂には明示承認を要する。Scheduled／Event-triggered実行の許可はHuman Stepの完了、担当変更、通知先追加を許可しない。
- **Public 0.2基盤:** Jobs、Notifications、Identity、Capability／Permission、Activityの既存契約とGateを尊重する。通知Actionは実行時に再検証し、背景JobをPermission回避に使わず、期限・取消・重複防止・状態記録を既存契約に従って扱う。
- **Gateと公開境界:** 本決定は0.1 M30、0.2 Jobs／Notifications／Permission／Activity／Wayback Gate、0.3／0.4+のImplementation／Activation、AGENTS.md、Workstream owner／branch／path、Acceptanceを変更せず、実装開始・実装済みを意味しない。未公開のDEC本文や内部RoadmapをPublic repositoryへ転載しない。

## 未確定事項（PROVISIONAL）

- 状態機械、差し戻し回数、担当者コメント／成果物の形式、再提出後の検証方法、競合・Revision差分のUI。
- 期限の入力・計算、タイムゾーン、期限延長の扱い、期限前後の表示、例外処理。
- 「控えめ」なリマインドの具体頻度・間隔・上限・時間帯、重複排除、抑制設定、通知経路・再試行方法。
- エスカレーション条件、対象者の登録・Identity確認UI、各通知に含める最小情報、許可の付与・取消・満了UI。
- Offline／再起動時の処理、Job永続化、Activity／Task Logの具体Schema・Retention・redaction。
- 実装時期、版割当て、各GateのAcceptance基準。これらは個別のImplementation SpecとActivation Gateで決める。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | 思想㉑の選択Aを確定。差し戻し・修正・再提出を正式なWorkflow経路とし、承認済みScope内の安全な作業に限定。担当者コメントは権限を追加せず、正式な手順・権限変更にはユーザーの明示承認を要し、現行条件とRevisionを再検証して履歴を残す。 |
| 2026-11-05 | 思想㉒の選択Aを確定。Human Stepに期限、控えめな自動リマインド、明示的に許可された相手へのエスカレーションを認める。期限は完了扱い・自動担当変更・権限拡大を生じず、送信先／共有内容／条件の許可、停止・取消・通知抑制、履歴を尊重する。数値・頻度・経路等はPROVISIONAL。 |
