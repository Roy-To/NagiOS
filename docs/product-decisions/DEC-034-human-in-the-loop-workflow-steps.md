# DEC-034 — 人間の作業工程を組み込むHuman-in-the-loop Workflow

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（思想⑲A：人間Stepと安全な待機／条件再検証、思想⑳A：権限確認済み他ユーザーへの範囲限定Step割当）／PROVISIONAL（Step Schema、通知・受諾・代行・待機／期限・実装／適用版）  
**最終合意日:** 2026-11-05  
**対象:** Nagi Workflowの実行途中に人間の確認・入力・実作業を挟む協調型Workflow。実装版は未確定  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-008](DEC-008-proactive-behavior.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-022](DEC-022-builder-interrupted-task-recovery.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-028](DEC-028-deterministic-workflows-from-habits.md)、[DEC-029](DEC-029-scheduled-event-triggered-workflows.md)、[DEC-032](DEC-032-workflow-safe-retry-and-recovery.md)、[DEC-033](DEC-033-natural-language-workflow-authoring.md)、[DEC-037](DEC-037-human-step-revision-deadlines-reminders.md)、[Private 0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[Public 0.2 Platform基盤仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)

## 目的とユーザー体験

Nagiの自動処理だけでは完結しない業務も、単一のWorkflowとして扱えるようにする。例えば「データ集計→人間が資料内容を確認・修正→確認済みの結果を利用して残りの処理を進める」という流れでは、人間の工程を正式なStepとして記述し、その間はWorkflowを安全に待機させる。

## 確定した仕様（思想⑲・選択A）

1. **人間の工程も正式なStep:** Workflowは、人間の確認、情報入力、実作業、判断等を自動処理Stepと並ぶ正式なStepとして含められる。Nagiは人間に必要な作業・入力・期待する成果を提示し、明示された工程の完了状況をWorkflowのTask状態と結び付けられる。
2. **Human Stepで安全に待機:** 人間による工程が必要になったら、Workflowは該当する確認・入力・作業の完了を待つ。待機状態を未完了のまま記録・表示し、未承認の後続Stepを勝手に実行したり、期限経過やAIの推測だけで人間の工程を完了済みとみなしたりしない。人間待機を単なるResource待機、明示的Pause／Cancel、権限・承認待ち、クラッシュ後復旧待ちと識別できる設計とする。
3. **完了後に現行条件と権限を再確認して続行:** 人間が指定された作業を完了したことを適切な正規UI／契約上の手段で示し、必要な入力・成果を受け取った後、NagiはWorkflowの承認済み範囲とStep条件、ユーザー／Session／Profile、Permission／Capability／Policy、対象Resource／Revision、Task／Workflowの有効・取消・停止状態等を再検証する。すべてが成立する場合に限り、承認済みの次のStepへ進める。状態や結果が不明なら保留・確認し、旧Revisionに基づいて新しいユーザー編集を上書きしない。
4. **業務上の完了とAction承認は別:** 人間が「レビュー完了」「入力終了」等を示す行為は、そのHuman Stepの業務完了を意味し得るが、OSのPermission／Capability／Policyを変更せず、外部送信や大量削除等の高リスクActionの別途承認を自動的に代替しない。既存DEC-001／011の承認済みTask内の自律性と、重要操作の個別最終確認の条件を維持する。
5. **人間による修正・入力を尊重:** Human Stepで利用者が変更したデータを、旧いAI計画・Snapshot・Revisionで黙って上書きしない。Stepの入力や成果物を次の処理で参照するときは、許可されたProvider／Public Actionから必要な情報だけを取得し、適切な最新性・来歴とPermissionを確認する。Human Step完了自体がデータ取得や共有の追加権限になるわけではない。
6. **進捗と通知は控えめかつ正確:** Nagi Home／Desktopなど既存のTask進捗・ハイブリッド承認UIの方針に沿って、人間の作業待ち、未着手、確認済み、条件不一致、保留、完了など実態を表示する。通知・督促はDEC-008の控えめな提示と既存Privacy／Permission規則に従い、Human Stepがあるだけで常時監視や音声割込みを許可しない。
7. **安全な継続・中断の扱い:** Human Step完了後の通常の継続は、明示的に許可された既存Taskの範囲に限る。ユーザーのStop／Cancel、権限失効、期限切れ、競合、実行結果不明があれば勝手に再開しない。予期しないクラッシュ／再起動からのTask復旧や再送について、DEC-022のbuilder専用のユーザー選択ルールを緩和せず、DEC-032の安全な有限Retryを人間Stepの自動完了・強制再開へ転用しない。
8. **検証可能な記録:** Human Stepの要求内容、確認・入力があった事実、完了・未完了、後続Actionの実行結果をTask／Activityへ追跡可能な範囲で記録する。成功・部分成功・保留・未検証・失敗を正確に区別し、不要な機密本文や認証情報を記録しない。不可逆な副作用までUndoできると装わない。

## 確定した仕様（思想⑳・選択A）

1. **許可された他ユーザーへのHuman Step割当:** WorkflowのHuman Stepは、Workflow所有者本人に限定せず、必要な権限を持つ他のユーザーを担当者として指定できる。割当ては、対象Step、担当する本人／Identity、依頼内容、共有するResource／情報、担当範囲・有効なScopeを明示し、依頼元と受取側の現在の許可に従って行う。対象が曖昧であれば共有も割当ても保留する。
2. **共有元・受取側双方の権限境界:** 共有元にはその情報を対象者へ提供・割当てる正規の権限が必要で、受取側には自身のIdentityに紐づく当該Step・ResourceへのPermission／Capability／Policyの確認を要する。情報を読む・修正する・他者へ共有する・外部へ送信する権限は別々に評価する。本人のIdentityが確認できない、受取側に必要なPermissionがない、組織Policyが禁じる場合はStepを待機／保留し、別の権限を推測して付与しない。
3. **最小限の情報だけを共有:** 割当てにより、当該Stepに必要で許可された情報と作業対象のみを提示する。元のユーザーの個人Memory、Private Profile、Task全体、Workspace、Secret、Token、その他不要なResourceを自動共有したり、Team／Organization共有へ昇格したりしない。機密・由来・所有者・PrivacyはDEC-003と現行Permissionに従い、分類・共有条件が不明な情報は開示しない。
4. **担当の割当てはAction権限の委譲ではない:** Human Stepの依頼・完了報告だけで、受取側またはNagiが元のユーザーのTask実行権限・Capability・Approvalを取得したことにはならない。受取側の処理は受取側自身の認可範囲内に限定し、後続の自動Action、重要操作、外部送信、無人起動は既存の各承認を独立に確認する。人間の業務上の確認とOS側の高リスクAction承認も引き続き区別する（DEC-001／011／029）。
5. **担当者による完了後にのみ安全に続行:** 割当てられたStepを、現在のIdentityと許された入力・成果に基づいて正規に完了したと確認できた場合、Nagiは元のWorkflow／Taskの現在のPermission／Policy／Scope、Resource／Revision、実行条件、取消・停止状態を再確認して、許された後続Stepへ進める。未受領、未完了、本人・結果不明、権限失効、競合の場合は確認待ちとし、AIの推測や単なる通知送信だけで完了扱いしない。
6. **状態と来歴の正確な記録:** どのStepを誰に割り当てたか、共有が許された範囲、待機・完了・失敗、必要な確認の結果、後続の実行結果を、現行のPrivacy／Retentionに従いTask／Activityへ追跡可能な範囲で記録する。機密本文の複製を避け、他者の詳細なTask／Profile情報を権限外に開示しない。
7. **実装範囲の限定:** これは複数人の協調Workflowを可能にする製品方針であり、すべての遠隔Nagi端末・オンラインアカウント・組織間での送受信機能が実装済み、あるいは特定版で実装されるという保証ではない。対応するIdentity、通信・共有・Permission基盤とGateを満たした環境に限り利用できる。受諾・拒否、差戻し、代理担当、エスカレーションや具体的なUI／通信は別途決定する。

## 既存仕様との整合と禁止

- **DEC-028／033:** 反復作業の学習、自然言語からの草案生成、GUIでの作成は、同一のWorkflowに人間のStepを組み込む入口として使える方針とする。新規作成・保存・正式改訂はユーザー承認が必要であり、生成したHuman Stepを自動的に有効化・実行しない。
- **DEC-029:** Schedule／Eventによる許可済みWorkflowの起動と、途中にあるHuman Stepの完了待ちは区別する。起動許可だけでHuman Stepを省略したり、未承認の後続Actionを実行したりしない。
- **DEC-020／026／032:** 非特権LLMは計画や説明を支援しても、実行権限と完了判定を保証しない。通常Service／型付きPublic Action／Provider、Revisionの再確認、リスクに応じた保護、有限再試行、安全な停止を維持する。
- **所有・担当の境界:** 他ユーザーへのHuman Step割当は思想⑳の確定条件に限って認める。ただし、他人の確認・業務完了を現在のユーザーのAction実行承認に変換せず、各Identity／Profile／Permission境界を維持する。担当者の受諾／拒否、代行・エスカレーションの実装詳細は未確定とする。
- **実装ゲート:** 本決定は0.1 M30、0.2 Jobs／Public Action／Permission／Activity・WaybackのGate、0.3／0.4+ Implementation／Activation、AGENTS.md、Workstream ownership、既存Acceptanceを変更せず、実装開始や機能完成を意味しない。未公開内容をPublicへ転載しない。

## 未確定事項（PROVISIONAL）

- Human StepのSchema、担当者の識別・本人確認方式とIdentityの結び付き、完了・差戻し・否認・差分の状態遷移、入力形式と成果物検証。
- 待機状態の持続／再開条件、Timeout、Reminder／Notification、停止・取消、Offline／Restart時の復旧規則、Queue／Job永続化の詳細。
- Human Stepの画面・音声UI、業務確認と高リスクAction承認の分離表示、具体的な進捗表示、可視化範囲とアクセシビリティ。
- **他ユーザーへの割当を認める製品方針はCONFIRMED。** 具体的な担当者選択・受諾／拒否、引継ぎ、代行／エスカレーション、本人確認／受信確認、通知・共有経路、Team／Organizationへの展開、UI・通信・版割当・実装ゲートはPROVISIONAL。Privacyと実行権限の境界は本決定の確定条件に従う。
- Workflow全体のVersion／Revision、Human Stepでのユーザー編集の競合処理、Task／Activityの証跡保持・削除、検証・Acceptance、実装時期・対象アプリ。

## Roadmapとの関連

- **0.2:** Jobの状態管理、Permission／Capability、Public Action、Activity／Transaction／Waybackの基盤を利用できる構成を目指す。現在の0.2 scopeやGateへ新規要件を付加しない。
- **0.3改訂案:** Personal Agent、Identity、Profile、Context／Memoryの権限境界、通常GUI／Task UIの思想と整合する。Human-in-the-loop Workflowの実装版は未確定。
- **将来版:** Multi-User／Team連携を導入する際も、明示共有・本人の権限とStep完了・Workflow実行許可の区別を維持し、Implementation SpecとActivation Gateで検証する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | 思想⑲の選択Aを確定。人間の確認・入力・実作業を正式なWorkflow Stepとし、安全に待機。適切な完了通知後、現在のPermissionと実行条件を再検証して承認済みの後続処理を進める。業務の完了とAction承認を分離し、実装詳細は暫定とする。 |
| 2026-11-05 | 思想⑳の選択Aを確定。必要な共有権限と担当者本人のIdentity／Permissionを確認した範囲で他ユーザーへHuman Stepを割当可能とし、必要最小限の情報共有、他人の権限の不継承、完了後の再検証を記録。通知・受諾・代行・通信等の詳細はPROVISIONALとする。 |
| 2026-11-05 | 思想㉑・㉒の差し戻し／再提出、期限・リマインド・エスカレーションの確定仕様は[DEC-037](DEC-037-human-step-revision-deadlines-reminders.md)に記録。 |
