# DEC-032 — Workflowの安全な有限再試行と失敗時の復旧

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（思想⑰・選択A：承認範囲内の安全な有限Retry、危険・不確実な状態で停止しユーザー確認）／PROVISIONAL（Retry条件・回数・実装方式・適用版）  
**最終合意日:** 2026-11-05  
**対象:** Nagi Workflowの実行中の一時的失敗、再試行、部分成功、復旧判断。実装時期は未確定  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-008](DEC-008-proactive-behavior.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-022](DEC-022-builder-interrupted-task-recovery.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-028](DEC-028-deterministic-workflows-from-habits.md)、[DEC-029](DEC-029-scheduled-event-triggered-workflows.md)、[DEC-030](DEC-030-workflow-template-portability-and-sharing.md)、[DEC-031](DEC-031-external-workflow-validation-and-version-updates.md)、[Private 0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[Public 0.2 Platform実装仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)

## 目的

承認済みWorkflowで一時的なエラーが起きても、安全性を確認できる範囲では毎回人間の手を止めずに回復できるようにする。他方、実行済みか不明な副作用、無効なPermission、競合、不可逆操作を推測で再実行せず、元データとユーザーの意思を守る。

## 確定した仕様（思想⑰・選択A）

1. **安全な処理だけ有限回の自動Retry:** 既に明示的に許可されたWorkflow／Task Scopeの内側で、失敗原因や対象状態を確認し、安全な再試行ができる処理に限り、回数・時間・資源等の有限Budgetの範囲で自動再試行・復旧してよい。対象Actionが冪等である、実行前に失敗したことが確認済みである、または正規Provider契約で重複副作用を防止できる等、再試行の安全性を確認できることを前提とする。
2. **毎回の現行権限・状態を確認:** 再試行でも現在のUser／Profile／Provider、Permission／Capability／Policy、承認済みTask Scope、期限・取消・停止状態、対象Resource／Revision、Workflow版／依存状態を正規経路で再評価する。元の承認を延長したり、新たなResource・Action・送信先へ範囲を拡張したりしない。
3. **危険・不明なときは停止して確認:** 実行結果が不明、権限失効・拒否、Revision不一致・競合、不可逆な操作、重複副作用の危険、未承認の範囲変更、停止・取消、または現在状態を確証できない場合、該当Actionを自動再送・続行せず停止／保留する。実施状況と残る危険をユーザーへ示し、必要な確認・別承認を求める。ユーザー確認はPermission／Policy拒否を覆す手段ではない。
4. **早期停止と有限Budget:** Retryの上限まで必ず繰り返す必要はない。同じ失敗が続く、改善の見込みがない、状態確認や保護が失敗する場合は早期停止する。RetryによりCPU／RAM／I/O負荷や時間を際限なく増やさず、前景ユーザー操作の応答性、Jobs／Schedulerの既存制約を尊重する。
5. **副作用と部分成功を追跡:** Task／Attempt／Actionの識別情報、利用可能な冪等性・重複排除の契約、実行結果、Revision、Activity等に基づいて成功済み・未実行・失敗・結果不明を区別する。既に成功した副作用を盲目的に繰り返さない。複数アプリ間の部分成功を全体成功や完全Atomicityとして表示せず、可能な復旧経路と未解決の影響を明示する。
6. **結果検証と可逆性:** 自動復旧後も、元のWorkflowの受入条件、対象状態と可能な検証証拠を確認し、実際に完了した範囲だけを完了として報告する。Checkpoint／Wayback／保護機構を利用可能な範囲で用いるが、外部への送信や不可逆な副作用までUndoできると装わない（DEC-026）。
7. **LLMへの相談は権限付与ではない:** 定義済みの回復手順で解決できない場合、Nagiは状況を整理し、必要に応じてLLMによる原因分析・対応案へ戻れる。ただしLLMの提案や自動判断自体は新しいWorkflow、再実行、変更、送信の承認を生じさせない。判断不能ならユーザーに戻す。
8. **明示的なStop／Cancelを尊重:** ユーザーが停止・取消・無効化したWorkflowやScheduled／Event-triggered起動設定を、Retry／再送Event／失敗回復だけで自動再開しない。取消前の副作用や部分成功は実態どおり報告する（DEC-029）。

## 既存の復旧決定との関係

- **通常の実行中エラーと、クラッシュ／電源断後の再起動は区別する。** 本DECは承認済みで実行中のWorkflowの安全なRetry方針である。予期せぬ停止・再起動後に保存済みTaskや結果不明の副作用を勝手に再実行してよいことを意味しない。再起動時のRecovery方針・具体条件は別途既存のTask／Job契約と整合させる。特にDEC-022のbuilder Roleの中断後ユーザー選択・無断再開禁止を変更しない。
- **Builderの個別ルールを維持する。** DEC-016のbuilder修正・テストRetry方式Cと有限Budgetを尊重し、具体的な初期Retry回数はそのDECでもPROVISIONALである。DEC-018の隔離編集・反映前保護と条件付き適用を本DECで緩和しない。
- **既存Jobs基盤を使う。** Public 0.2のJobsのJobId、所有者・Profile Scope、状態機械、deduplication／idempotency、Retry／Backoff、deadline、cancel、bounded persistence、capability check等の契約を維持する。Job SchedulerをPermission回避経路にしない。
- **既存Activation境界を維持する。** 0.1 M30、0.2のJob／Public Action／Permission／Activity／Wayback Gate、0.3以降のImplementation／Activation、AGENTS.md、Workstream owner、Acceptanceを変更せず、実装済み・実装開始の証拠としない。Private仕様をPublic repositoryへ転載しない。

## 未確定事項（PROVISIONAL）

- Retry可能な失敗／Action種別の分類、Providerが保証する冪等性や成功判定の形式、安全判定とRecoveryの具体的なアルゴリズム。
- Retry回数、Backoff、時間・Resource Budget、早期停止の判定基準とユーザー設定。具体値は未決定。
- Task／Attempt／Action ID、Idempotency Key、Checkpoint／Activity記録、跨るProviderの部分成功、状態再構築と重複排除の実装方式。
- 再起動後のWorkflow一般の復旧モード、ユーザー確認UI、結果不明の追跡、保護／復元の制約、受入テスト／障害注入テスト。
- 対応アプリ／Action、優先導入版、実装担当、性能目標とAcceptance。

## ロードマップとの整合

- **0.2:** 既存Job／Permission／Task state／Activity／Wayback基盤を利用するがRelease scope・Gateを変更しない。
- **0.3改訂案:** 習慣・決定論的Workflowおよび権限評価されたAgent構想に接続する。具体的な実装版は未確定。
- **将来版:** 自動Retry／復旧の実装は対応するImplementation SpecとActivation Gateの承認を別途要する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | 思想⑰の選択Aを確定。承認済み範囲内の安全な処理は有限回自動再試行できる。結果不明・権限失効・競合・不可逆操作・取消時は自動再実行せず停止／保留してユーザー確認へ戻す。既存のBuilder中断後復旧・Jobs契約との区別を維持。 |
