# DEC-028 — 学習した反復作業からの承認制Deterministic Workflow

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（反復作業からのWorkflow化、保存・有効化の明示承認、正式手順改訂の明示承認）／PROVISIONAL（変換方式・提案条件・実装詳細・適用版）  
**最終合意日:** 2026-11-05  
**対象:** Nagiがユーザーの反復作業を学習・再利用する将来のWorkflow機能。実装時期・Release割当ては未確定  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-003](DEC-003-memory-classification.md)、[DEC-004](DEC-004-memory-learning-forgetting.md)、[DEC-005](DEC-005-files-albert-memory-lifecycle.md)、[DEC-008](DEC-008-proactive-behavior.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)、[Private 0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[Public 0.2 Platform Foundation仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)

## 目的

ユーザーが繰り返し行う作業からNagiが手順・習慣を学び、再利用できる決定論的Workflowへの変換を提案する。承認されたWorkflowの通常手順は、可能な限りモデル推論に依存しない型付きActionとOS／アプリServiceで実行し、予測可能性・再現性・検証可能性を高める。

## 確定した仕様（思想⑪・⑫、各選択A）

1. **反復作業の学習とWorkflow化の提案:** Nagiは、ユーザーが繰り返し行い採用した手順を、適切なTask・Workspace等の文脈とともに学習し、再利用可能な決定論的Workflowへの変換を提案できる。提案は学習・再利用の便益を示すもので、Workflowの保存・有効化・実行許可を意味しない。
2. **保存・有効化への明示承認:** Workflowとして保存または有効化する前に、ユーザーの明示承認を得る。学習された習慣、過去の実行、Workflowの所有・作成・承認そのものは、現在または将来の実行権限を付与・拡張しない。
3. **実行時の権限と範囲の再評価:** 各実行時に現在のPermission／Capability／Policy、Task Scope、対象・Resourceと必要に応じてRevision、実行時に必要な承認、外部送信等の副作用条件を既存の認可境界で評価する。保存時の承認は将来の状態・権限を固定せず、承認範囲外や現行Policyで許可されない操作は実行しない。既存DECが許す承認済みTask内の操作は、そのTask Scopeと実行時検証の内側で扱う。
4. **決定論的な通常経路:** 確定した通常手順は、可能な限りLLMなしで、型付きPublic Action／OS・アプリService APIから実行する。WorkflowであることはPublic Action、Provider、Permission、Policy、Taskの既存境界を迂回しない。正規Action APIに未対応のアプリを扱う場合はDEC-027の明示許可・限定Scope等の条件に従う。
5. **未定義条件と例外:** Workflowに定義されていない入力・状態・判断が必要な場合や、失敗・競合・結果不明等の例外が生じた場合は、安全に停止・保留し、状況に応じてLLMによる解釈・提案へ戻ることができる。LLMの提案は実行権限ではなく、Actionとして検証し、既存Permission／Policy／Approvalを通さずに実行しない。範囲を確認できない場合は実行せず、ユーザーの判断または手動操作へ戻す。
6. **管理・透明性:** ユーザーは保存済みWorkflowを確認し、内容を変更・無効化・削除できる。実行履歴と結果はActivity／Taskに正確に記録し、成功・部分成功・失敗・未検証、副作用、取り消しや復元の限界を実態どおりに表示する。実行結果の報告、検証可能性・復元可能性はDEC-001／011／014／026に従う。
7. **Workflow改善案と正式な手順改訂（思想⑫・選択A）:** 学習・ユーザー承認済みの決定論的Workflowについて、Nagiは改善候補を新しいバージョン候補として生成・検証し、変更差分と想定影響をユーザーへ提示できる。候補の生成・検証・提示だけでは、Workflowの正式な実行手順への反映、有効化、現行版の置換は行わない。正式な手順の反映または新バージョンへの置換には、対象のWorkflowと変更内容を特定したユーザーの明示承認が必要である。過去の承認は後続の改訂を承認しない。承認済みWorkflowの通常パラメータを、その承認済みTask Scope内で代入・実行することは、手順自体を変更しない限り手順改訂とは区別する。
8. **バージョン・検証記録・復旧:** 旧版および各候補の検証記録と根拠を保持する。変更履歴、差分、検証状態、承認状態、実行結果、復元可否を区別して示し、旧版へ戻ることを可能な範囲で支援する。復元操作にも現在のPermission／Policy／Capability、Task Scope、必要なAction別承認、競合・Revisionを再評価する。失敗・未検証・競合・回復不能な状態を成功または復元可能と表示しない。保持期間や具体的な保存形式は未確定とする。
9. **Memoryとの整合:** Workflow、Procedure／Habit Memory、Profile等は特定LLMの重みに依存しない利用者管理データとして扱う。元データの削除・権限失効・明示的忘却の際は、DEC-003〜005の由来、機密分類、派生Memory、削除・忘却範囲を適用する。元データが削除された場合にWorkflowの実行依存Resourceが無効になれば実行を保留し、別の根拠から再学習したように黙って振る舞わない。ユーザーがWorkflowを無効化・削除した指示を、過去履歴から自動的に覆さない。

## 習慣学習を主軸とする優先順位（2026-11-16／CONFIRMED）

[DEC-058](DEC-058-habit-first-learning-and-optional-teach-mode.md)により、Nagiが利用者の作業方法へ適応するための主経路は、許可された日常利用と反復・利用者による採用・修正に基づく**自然な習慣学習**であることを明確にした。本DECの習慣からのWorkflow化提案を主軸として維持し、Teach Mode（仮称）の作業実演は**ユーザーが必要なときに選べる補助的な草案作成経路**とする。Teachを使わなくてもHabit学習・Workflow提案は利用可能でなければならない。

Teachで得た草案も、自然言語／GUI入力や習慣から生まれた草案と同じ正規の検証・承認・権限・Activity境界に従い、実演だけでWorkflowが正式保存・自動実行されたとは扱わない。記録の具体方式と提供版はPROVISIONALのままとする。

## 例外・禁止・安全境界

- 反復頻度や習慣の推定だけでWorkflowを自動保存・有効化したり、ユーザーが承認していないTaskを開始したりしない。
- Workflowの実行に必要な権限を記憶に含めたり、Workflow保存・有効化・手順改訂の承認を実行権限または恒久的な包括許可と解釈したりしない。変更案の作成だけで承認・権限は発生しない。各実行時に現在のPermission／Policy／Capability、Task Scope、必要なAction別承認を再評価する。
- 決定論的実行を理由に、必要なAction検証、Resource／Revision確認、承認、Activity記録、リスク別保護、取消・停止を省略しない。
- 例外後のLLM提案を未検証のまま実行しない。失敗、部分的副作用、結果不明を成功として表示しない。
- 本決定は0.1／0.2／0.3／0.4+の既存Gate、Acceptance、workstream ownership、Public／Privateの公開境界を変更しない。製品方針の記録は実装開始・有効化・Release activationを許可しない。未公開仕様をPublic repositoryへ転載しない。

## 未確定事項（PROVISIONAL）

- 操作記録／学習からWorkflowを構成するDSL、Recorder、Action列・条件分岐等の表現方式と検証器。
- 習慣とみなす反復・採用の基準、Workflow化の提案閾値・頻度・提示方法。
- Workflowの保存・有効化承認UI、編集・差分確認、無効化・削除、共有・複製の詳細。
- 依存データ・Resource／Revision、バージョン割当、互換性、再検証・更新・再承認の条件。
- 通常手順の決定論的実行、例外検出、LLMへの復帰、テスト・監査・性能要件の詳細。
- Workflow実行時のJob、Retry、Checkpoint、Cancel、重複実行抑止等の契約。具体的な常時監視・Event購読条件。
- 改善案を生成する具体的なトリガー、候補・正式版のテスト方法と合格基準、UI、旧版・検証証拠の保持期間、バージョン体系、保存形式、実装方式。
- Release割当、対応アプリ、実装担当、受入条件。これらは将来のImplementation SpecとActivation Gateに従って決める。

## Roadmapとの関連

- **0.2:** Public Action、Permission／Capability、Jobs、Activity等の既存契約を基盤として扱う。DEC-028は0.2のScope・Gateを追加・変更しない。
- **0.3改訂案:** Procedure／Habit Memoryの方向に接続するが、Workflow化の具体方式・提案閾値・実装割当てはこのRoadmapへの記録だけでは確定しない。
- **0.4+:** 将来の複数Agent等へ展開する場合も、各Agentが現在のPermission／PolicyとTask Scopeに従う。具体的な展開時期は未確定。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | 思想⑪の選択Aを確定。学習した反復作業をユーザー承認のもと決定論的Workflowへ変換できる方針とし、実行権限は別途評価、未定義条件や例外ではLLMへ戻る境界を記録。保存・有効化には明示承認を要し、具体方式は暫定とした。 |
| 2026-11-05 | 思想⑫の選択Aを追加。NagiはWorkflow改善候補を生成・検証し差分・影響を提示できるが、正式な手順改訂には個別の明示承認を要する。旧版・検証記録・根拠を保持し、可能な範囲で旧版復帰を支援する。候補生成は承認・実行権限を付与せず、具体方式は暫定とした。 |
| 2026-11-16 | DEC-058に基づき、習慣学習由来のWorkflow提案を中心に据え、Teach Modeを任意の補助経路とする優先順位を追加。保存・有効化・実行・改訂の承認境界は維持。 |
