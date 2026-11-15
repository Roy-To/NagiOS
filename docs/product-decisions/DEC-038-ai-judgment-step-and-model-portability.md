# DEC-038 — WorkflowのAI判断Stepとモデル非依存の入出力契約

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（思想㉓A：明示されたAI判断Step、型付き入力・出力・分岐・検証、判断不能時のHuman Step等への保留／思想㉔A：判断能力と入出力契約に基づくモデル非依存性、切替時の品質・互換性・安全性検証）／PROVISIONAL（判断品質の閾値、モデル選択・検証方式・実装版）  
**最終合意日:** 2026-11-05  
**対象:** Nagi Workflowの通常経路に含めるAI判断Step、モデルの選択・切替。実装版は未確定  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-002](DEC-002-cloud-ai-data-sharing.md)、[DEC-003](DEC-003-memory-classification.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-028](DEC-028-deterministic-workflows-from-habits.md)、[DEC-029](DEC-029-scheduled-event-triggered-workflows.md)、[DEC-032](DEC-032-workflow-safe-retry-and-recovery.md)、[DEC-033](DEC-033-natural-language-workflow-authoring.md)、[DEC-034](DEC-034-human-in-the-loop-workflow-steps.md)、[DEC-037](DEC-037-human-step-revision-deadlines-reminders.md)、[ADR-0018](../decisions/0018-decision-and-generative-model-architecture.md)、[Decision / Generative AI architecture](../architecture/decision-and-generative-ai-architecture.md)、[Private 0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[Public 0.2基盤仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)

## 目的

Workflowの手順、権限、Actionの通常処理は決定論的なOS／アプリServiceを基本とし、文章分類・要約・判断など推論を必要とする箇所だけ、明示的に定義したAI判断Stepを利用できるようにする。判断Stepの契約を特定モデル名から分離して、ユーザーのモデル切替後も安全に利用を検証できる設計を目指す。

## 確定した仕様（思想㉓・選択A）

1. **推論が必要な工程のみAI Step:** 既存の決定論的なPublic Action／Service／Job処理を基本とし、Workflowの定義に明示した工程に限ってAI判断Stepを含められる。LLMを全手順の実行者としたり、AI判断をすべてのStepに必須としたりしない。
2. **入力・出力・分岐の契約:** AI Stepが使用する目的・入力範囲、期待する構造化出力、許される候補・分岐、後続Stepへの接続を定義できるようにする。Providerの応答は契約上の型・値・必要条件に従って検証し、未定義の行先や任意のActionをAI応答から追加しない。
3. **認可されたContextだけを利用:** 入力取得時にはUser／Session／Profile、Task Scope、Resource／Revision、Permission／Policy、情報分類・由来を確認する。AI Stepを理由に他ユーザーのMemoryや秘密情報を追加取得しない。Cloud推論へ入力する場合はDEC-002のService・Purpose・情報クラス・送信先等の許可を別途評価する。
4. **推論結果は権限ではない:** AIが「この分岐が適切」と回答しても、Workflowの実行許可、Action承認、外部送信・変更権限にはならない。OS側の型付きValidator、Public Action／Provider、現在のPermission／Capability／Policy、Task Scope、必要なAction別承認を経て後続処理を実行する。
5. **不確実なら安全に保留:** 出力不適合、判断不能、十分に検証できない結果、依存するModel／Providerが利用不能、対象Resource競合などでは、結果を成功や確定判断と装わず、後続の危険なActionを保留して人間の判断・Human Stepに戻せる。Human Stepと他ユーザー割当の条件はDEC-034／037に従う。
6. **監査と取消:** AI判断の対象Step、利用可能な入力・モデル／Provider参照、検証状態、選択された分岐、後続の実行結果・失敗をTask／Activityに記録する。秘密情報の無用な複製や停止済みWorkflowの無断再開を行わない。

## 確定した仕様（思想㉔・選択A）

1. **モデル非依存の定義:** WorkflowのAI Stepは特定モデルの重み・名前を本質的な契約とせず、必要な判断能力と構造化された入力・出力の契約を基準として定義する。既存のDecisionProvider／GenerativeProvider、Model Manager／Router等の型付き境界との整合を目指し、モデル内部構造には依存しない。
2. **切替時の品質・互換性・安全性検査:** 使用モデルを変更する場合、対象Stepの必要能力、出力Schema・分岐条件との互換性、利用可能な品質・安全性の検証結果、入力の情報分類・Privacy、現行Permission／Capability／組織Policy、承認済みTask Scope、Provider・送信先・目的に関するデータ共有許可を再検証する。モデルごとに結果が異なり得ることを隠さず、重大な判断傾向・分岐・結果の変化や承認Scopeの変更が必要な場合はユーザーへ確認する。
3. **不適合時は保留、権限は継承しない:** 必要な能力を満たせない、評価結果が不明・不足、Providerの利用許可がない場合、別モデルを使えると決めつけずAI Stepを保留し、ユーザーの判断へ戻す。モデル変更によってCloud送信、Resource、Actionの権限が新たに得られることはない。
4. **ユーザー選択と正式改訂を尊重:** ユーザーがモデルを固定・制限した設定、Cloud利用制限、明示的な停止を尊重する。固定設定を最適化目的で勝手に上書きしない。モデル切替それ自体と、保存済みWorkflowの正式な手順変更は区別し、手順を変更する場合はDEC-028の明示承認・Version履歴を適用する。
5. **判断の等価性を保証しない:** 同じSchemaを満たしても、異なるモデルの判断内容・正確性・性能が一致することや、自動で安全な移行が常に可能なことは保証しない。判定の根拠と限界を可能な範囲で示す。

## 未確定事項（PROVISIONAL）

- 判断StepのSchema／DSL、DecisionKindやGenerativeProviderの使い分け、モデル能力登録、選択・固定・切替・Fallbackの具体的方式。
- 品質評価のデータ、閾値、信頼度の意味・校正、出力検証手法、モデル変更でユーザー確認を要する基準、検証不能時の画面と操作。
- AI StepのContext budget、モデル利用コスト、実行・Batch・Queue、Cancel、監査情報のRetention、受入・障害注入テスト、対応モデル・Release割当。

## 実装・公開境界

本決定は0.1 M30、0.2のPublic Action／Jobs／Permission基盤とGate、0.3のModel／Context／AgentのImplementation／Activation Gate、0.4+のWorkstream／Activation Gate、AGENTS.md、Workstream ownership、既存Acceptanceを変更しない。DecisionProviderが存在することを「すべての判断が正しく実行できる」と解釈しない。本DECは実装開始・実装済みの証拠ではない。未公開仕様をPublic repositoryへ転載しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | 思想㉓Aを確定。通常の決定論的Workflowに明示的AI判断Stepを認め、構造化された入出力・分岐・実行時検証を要求し、不確実な判断はHuman Step等へ戻す。 |
| 2026-11-05 | 思想㉔Aを確定。判断能力・型付き契約を基本としてモデル非依存にし、切替時の品質・互換性・安全性を評価。不適合・不確実なら保留し、既存ユーザー設定とPermissionを維持する。 |
