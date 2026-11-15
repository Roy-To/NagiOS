# DEC-033 — 自然言語とGUIによるWorkflowの新規作成

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（思想⑱・選択A：自然言語からの構造化Workflow草案、ユーザーによる確認・編集・承認後の保存、GUIからの直接作成）／PROVISIONAL（Schema・編集UI・検証・実装方式・適用版）  
**最終合意日:** 2026-11-05  
**対象:** Nagiにおける新しいWorkflowの作成・編集・保存体験。実装時期とRelease割当ては未確定  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-008](DEC-008-proactive-behavior.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)、[DEC-028](DEC-028-deterministic-workflows-from-habits.md)、[DEC-029](DEC-029-scheduled-event-triggered-workflows.md)、[DEC-030](DEC-030-workflow-template-portability-and-sharing.md)、[DEC-031](DEC-031-external-workflow-validation-and-version-updates.md)、[DEC-032](DEC-032-workflow-safe-retry-and-recovery.md)、[Private 0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[Public 0.2 Platform基盤仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)

## 目的とユーザー体験

ユーザーは、作業を事前に何度もNagiへ見せたり、最初から手順をプログラミングしたりしなくても、達成したい目的を自然言語で説明してWorkflowの草案を作れる。生成した草案は実行手順・条件・必要な権限を構造的に確認・編集・承認できる。会話による作成だけに限定せず、GUIから直接組み立てる方法も利用可能にする。

## 確定した仕様（思想⑱・選択A）

1. **自然言語から新規Workflow草案を生成:** Nagiはユーザーの目的、望む入力・出力、処理の条件などの自然言語による説明をもとに、新しいWorkflowの草案を生成できる。既存の反復作業や学習済みHabitがない場合でも作成を支援する。これはDEC-028の反復作業からのWorkflow化に追加する別の作成経路であり、既存方式を置き換えない。
2. **構造化された内容を確認・編集:** 草案の手順、順序・分岐・実行条件、利用するアプリ／公開Action・依存関係、入出力と対象Resource、必要なPermission／Capability／Task Scopeを、適用できる範囲で人間が読んで確認・修正できる形で提示する。自然言語による追加指示と構造化編集の双方から草案を調整できる。
3. **ユーザーの承認後に正式保存:** Nagiが生成した草案はそれ自体を承認済みWorkflowとみなさない。ユーザーが作成内容と必要権限を確認・編集し、対象の草案を明示的に承認した後に正式なWorkflowとして保存する。曖昧な条件、未解決の依存、未対応Action、検証できない点は明示し、実行できると偽らない。検査・確認が完了していない案を勝手に有効な自動実行へ移行しない。
4. **GUIからの直接作成も可能にする:** 自然言語による草案生成とは別に、ユーザーがGUI上で処理・条件等を自ら構成してWorkflowを作成・編集する経路を提供する。AI生成を唯一の入口や必須条件とせず、LLMが利用できない場合でも、対応するGUI機能による手動作成を妨げない。
5. **作成・保存と実行の許可は別:** Workflow草案の生成、構造化編集、保存への承認は、実際のTask実行、外部送信、変更、Scheduled／Event-triggeredの自動起動、権限付与を意味しない。実行時には現在のPermission／Capability／Policy、目的・対象・承認済みTask Scope、Resource／Revision、必要なAction別承認を正規Provider／Public Action経路で評価する。高リスク・不可逆操作に要求される確認を省略しない（DEC-001／011／026／029）。
6. **同一のWorkflow契約を利用:** 自然言語で作ったもの、GUIで作ったもの、反復作業から学んだものは、可能な範囲で同じ構造化Workflowの公開契約とVersion／検証・Activityの規則に従う。承認済みWorkflowの正式な手順改訂はDEC-028の明示承認・旧版／検証記録の保持規則を適用する。未統合アプリへのGUI fallbackはDEC-027の限定的な条件に従う。
7. **推論・実行の分離:** LLMが目的の解釈と草案作成を助けても、Authorityを判断したり任意のOS内部処理を実行したりしない。草案中のActionと条件は、既存の型付きPublic Action／Service API、Provider、Permission／Policy、実行検証の境界に従う。決定論的に定義できた通常手順は、DEC-020／028の方針に従って可能な範囲でLLM推論を使わず実行できる形を目指す。

## 例外・禁止・安全境界

- 自然言語で「毎月末」「送って」等の指示があっても、未確定の送信先、対象Resource、権限、起動範囲をAIが推測で広げて自動実行させない。不明な事項は草案内で未確定として示し、ユーザーに確認できるようにする。
- 承認済みTaskのAction権限、Workflowの保存権限、Workflowの自動実行許可を混同しない。既存のDEC-029の許可された時刻／イベント条件と実行時の再検証を維持する。
- GUI上での作成操作は、未公開のアプリ内部データや特権APIへのアクセスを認めない。Permission・情報分類・Privacyを尊重する。
- 本決定は0.1 M30、0.2のPublic Action／Jobs／SDK、0.3 Implementation／Activation、0.4+ Gate、AGENTS.md、Workstream ownership、既存Acceptanceを変更せず、実装開始や機能完成を意味しない。Private仕様をPublicへ転載しない。

## 未確定事項（PROVISIONAL）

- Workflow草案の型付きSchema、条件分岐／変数／入出力表現、検証器、静的解析、未定義条件の警告形式。
- 自然言語からの草案生成・ユーザーへの確認質問・Context取得・LLM利用条件とプロンプト等の具体的な内部設計。
- GUI編集の具体的な形態（ビジュアルブロック、ステップ一覧等）、会話とGUIの同期、プレビュー、差分表示、Accessibility、承認UIの詳細。
- 草案を正式保存できる条件、テストやdry-runの適用条件、依存アプリ／Actionの検出、権限の提示・別承認の形式。
- Workflow手順のVersioning、保存・共有・ImportとのSchema互換、Editor／Package／SDKの連携、実装担当・受入条件・適用Release。

## Roadmapとの関連

- **0.2:** 既存Public Action、Permission、SDK、Jobs、Resource／Revision、Activity基盤の契約を尊重する。0.2への新規Release blockerを追加しない。
- **0.3改訂案:** Personal Agent、許可されたContext、Procedure／Habit Memoryと決定論的処理の方向に接続する。自然言語作成機能の導入版は未確定とする。
- **将来版:** 適切なImplementation SpecとActivation GateのもとでEditor／Workflow runtimeを具体化する。将来の第三者アプリも同じ公開API契約とPermission境界に従う。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | 思想⑱の選択Aを確定。自然言語からWorkflow草案を生成し、構造化された手順・条件・権限をユーザーが確認・編集・承認して保存する。GUIによる直接作成も認め、作成・保存と実行権限は分離する。 |
