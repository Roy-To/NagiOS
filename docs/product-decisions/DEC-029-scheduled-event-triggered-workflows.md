# DEC-029 — 明示許可に基づくScheduled / Event-triggered Workflow実行

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（特定のスケジュール／イベント条件による無人起動と実行権限の分離、再検証、停止制御）／PROVISIONAL（DSL・発火閾値・Job永続化・Retry・Idempotency・UI・Scope詳細・Version）  
**最終合意日:** 2026-11-05  
**対象:** ユーザーが明示許可したスケジュールまたはイベント条件で起動する将来のWorkflow機能。実装時期・Release割当ては未確定  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-004](DEC-004-memory-learning-forgetting.md)、[DEC-008](DEC-008-proactive-behavior.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-028](DEC-028-deterministic-workflows-from-habits.md)、[Private 0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[Public 0.2 Platform Foundation仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)

## 目的

ユーザーが承認したWorkflowを、許可された特定のスケジュールまたはイベント条件で自動起動できるようにする。Workflowの学習・保存・改訂への承認と、無人起動の実行許可は別の判断として扱う。

## 確定した仕様（思想⑬・選択A）

1. **保存・改訂承認と実行許可の分離:** Workflowを学習・作成・保存・有効化し、または正式改訂する承認は、それだけでScheduled / Event-triggeredの無人起動を許可しない。自動実行には、対象Workflowについてユーザーが特定の起動条件、対象Task、Scopeを明示して許可する必要がある。
2. **明示された条件だけで起動:** 許可されたScheduleまたはEvent条件に合致した場合に限りWorkflowを起動できる。Event検知それ自体は、新たなTask実行、Task Scopeの拡大、推論・通知・Actionの許可にならない。未許可のEventからTaskを作成・開始せず、既存の権限を拡大しない。
3. **起動時と副作用ごとの再検証:** 各起動時、および副作用を伴う各Actionの実行時に、現在のUser / Provider Permission、Capability、Policy、承認済みTask Scope、対象ResourceとRevision、Workflowの有効状態、許可の取消・期限・状態を再検証する。状態を確認できない、Scope外、権限失効、競合、またはPolicyで許可されない場合はfail closedとし、停止・保留して必要に応じユーザーへ戻す。保存時・起動時の許可は将来のPermissionやProvider状態を固定しない。
4. **ユーザーによる管理と停止:** ユーザーは自動実行設定を表示し、停止・無効化・変更できる。停止または無効化された設定は、ユーザーが明示的に再許可・再開しない限り自動で再開しない。ユーザー操作による停止は、可能な範囲で実行中Jobと待機Actionへ伝播し、既に発生した副作用は実態どおり報告する。
5. **記録と結果の透明性:** 重複Event／起動、失敗、部分成功、結果不明、実行不能、取消、検証状態をTask／Activityへ正確に記録し、通知は既存のPermission・Policy・DEC-008の通知境界に従う。不可逆操作と外部送信は、自動起動許可とは別の既存権限・Action別承認を維持し、実行されたか不確かな副作用を盲目的に再実行しない。
6. **決定論的な通常経路:** OSはLLMが利用不可でも、定義済み条件を満たすWorkflowを決定論的手順で通常経路から実行できる設計を目指す。未定義条件や例外のLLM fallbackはDEC-028に従い、既存のPermission、Policy、Approval、Task Scope内に限定する。LLMが利用できないことも、失敗や曖昧状態を成功扱いしたり権限境界を緩めたりする理由にならない。
7. **停止・再確認:** 条件解決、対象Resource／Revision、Task Scope、必要なPermissionまたは実行状態が曖昧な場合は、自動的に推測・拡張せず、安全に停止または保留してユーザーの判断へ戻す。イベントの再送や遅延だけで停止済み設定を再開しない。

## 既存決定との関係・安全境界

- **DEC-028:** Workflowの学習、保存・有効化、正式改訂の承認と、本DECの特定条件での無人起動許可は別々に取得する。Workflow所有、版の承認、過去の実行は、後続の自動実行許可やAction権限を与えない。
- **DEC-001／025／026:** 習慣やEventは権限を付与しない。承認済みTask内の自律実行、アプリ横断Action、検証・復元、部分成功や結果不明の扱いは各決定を維持する。外部送信・不可逆操作に必要な個別の許可を本DECで包括化しない。
- **DEC-008／020:** Event-driven処理・通知は控えめな既定方針と現行Permissionに従う。Event検知単独でLLM推論、通知、Actionを認可しない。
- **Public 0.2基盤:** Job Scheduler、Permission／Capability、Task state、Notificationは既存Public契約とGateの範囲で利用する。本DECは0.2の実装Scope、Gate、Acceptance、Workstreamを変更せず、Public仕様へ未公開内容を転記しない。
- 本DECの記録は、実装開始・有効化・Release activationの許可ではない。

## 未確定事項（PROVISIONAL）

- 発火条件・イベント照合・Schedule表現のDSL、対象選択とScopeの具体形式。
- Schedule／Eventの登録、重複排除・発火条件、Job永続化、時計・遅延・欠落Eventの扱い。
- Retry上限・Backoff・Idempotency Key・結果不明時の回復契約。
- 実行中の停止・取消伝播、Resource／Revision再検証、部分成功からの復旧詳細。
- 許可・有効化・表示・停止・無効化・変更・再開のUIと確認方式。
- Workflow／許可／JobのVersioning、互換性、変更時の再確認・再承認、保存・保持・削除形式。
- 通知頻度、重複・失敗表示、対応するEvent／Provider、対応アプリ、負荷制御、実装担当・Release割当て・Acceptance。

## Roadmapとの関連

- **0.2:** Jobs、Capability／Permission、Task・Activity状態、Notifications等の既存契約とGateを尊重する。製品機能の追加実装やGate通過を本DECで先取りしない。
- **0.3改訂案:** Habit／JobsおよびDEC-028の決定論的Workflow構想に接続する。具体方式・Version割当ては未確定。
- **将来版:** 実装仕様と別途のActivation Gateで、条件表現、権限再評価、停止・取消、記録、Retry、重複防止を定義する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | 思想⑬の選択Aを確定。ユーザーが明示許可したSchedule／Event条件でWorkflowの無人起動を認め、各起動時と副作用Action時の現行権限・Scope再検証、表示・停止・無効化、停止済設定の勝手な再開禁止を記録。重複・失敗・部分成功・実行不能の記録と既存の外部送信／不可逆操作権限を維持し、通常経路の決定論的実行を目指す。具体実装詳細はPROVISIONAL。|
