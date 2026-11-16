# DEC-058 — 習慣学習を主軸とし、Teach Modeを補助機能とするPersonal Agentの成長経路

**状態:** CONFIRMED（普段の利用に基づく習慣学習を主経路にする、Teachは任意の補助機能、学習とWorkflow実行の分離）／PROVISIONAL（Teachの具体的な操作記録方法・UI・利用条件・実装方式と適用版）  
**決定日:** 2026-11-16  
**対象:** Nagi Personal Agent OSのLearning／Habit Memory／Workflow作成体験。実装・Release／Activation Gateは変更しない。  
**関連:** [DEC-003](DEC-003-memory-classification.md)、[DEC-004](DEC-004-memory-learning-forgetting.md)、[DEC-005](DEC-005-files-albert-memory-lifecycle.md)、[DEC-008](DEC-008-proactive-behavior.md)、[DEC-013](DEC-013-lightweight-memory-engine-benchmark.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-028](DEC-028-deterministic-workflows-from-habits.md)、[DEC-033](DEC-033-natural-language-workflow-authoring.md)、[0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[0.5–1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

## 目的

Nagiがユーザーとともに成長するPersonal Agent OSとなるうえで、**日常的な利用から自然に習慣や仕事の進め方を学び、提案を改善する経路を主軸**とする。ユーザーが毎回「覚えて」と言ったり、事前に作業を教えたりしなくても、既存の許可された入力とMemory／Learning契約の範囲で継続的な個人適応を目指す。

一方、ユーザーが特定の仕事のやり方を明示的に教えたい場合には、作業の実演などによる**Teach Mode（仮称）**を任意の補助機能として用意する方向とする。Teachを通常の習慣学習の前提条件や唯一の導入経路にしない。

## 確定した製品方針（CONFIRMED）

1. **習慣学習を既定・主経路とする。** Nagiの個人適応とWorkflow提案の中心は、日常の繰り返し行動、ユーザーが採用した作業方法、好みの明示・訂正などから、Task／Workspace／用途の文脈に応じて学ぶこと。既存[DEC-004](DEC-004-memory-learning-forgetting.md)の自動Memory・習慣学習を主軸とし、許可された情報の観察・保持・利用の条件を緩和しない。
2. **Teach Modeは任意の補助機能とする。** ユーザーが明示的に教えたい作業を、実際に操作して見せる等の体験を提供する方向とする。Teachを利用しなくても通常の習慣学習とNagiの利用は成立する。Teachを使うかどうかはユーザーが選び、対象の作業範囲を限定できることを目指す。
3. **学習結果は提案と理解に反映する。** 習慣学習は提案内容、作業文脈の選択、再開支援、作業手順の改善候補に反映できる。ただし、頻度だけを根拠に利用者の望みを断定しない。ユーザーによる訂正や明示的な希望を推定より優先する。
4. **Teach・自然言語・GUIは補助的な別入口。** 学習済みの反復手順からWorkflow化を提案する[DEC-028](DEC-028-deterministic-workflows-from-habits.md)を中心に据えつつ、[DEC-033](DEC-033-natural-language-workflow-authoring.md)の自然言語・GUI作成を引き続き利用できるようにする。TeachからWorkflow草案を作る場合も、既存の構造化Workflow・検証・版管理契約へ接続する方向とし、重複する実行権限体系を設けない。
5. **候補作成と保存・実行を分ける。** 習慣を発見したこと、Teachで操作を見せたこと、草案が生成できたことだけでは、正式Workflowの保存・有効化・自動実行やAction権限は発生しない。正式Workflow化・改訂には既存DEC-028／033のユーザー確認・明示承認、実行には現行のPermission／Capability／PolicyとTask Scope等を適用する。
6. **自然に、控えめに、訂正可能に学ぶ。** 習慣学習を主軸とすることは、無制限な画面録画、全アプリの操作監視、無差別なログの恒久保存、Cloud送信、ユーザーに気付かれない権限拡大を認めない。学習の由来・所有範囲・機密分類・訂正・自然忘却・明示忘却はDEC-003〜005に従い、提案はDEC-008の控えめな働きかけを維持する。
7. **モデル非依存の継続性を保つ。** 習慣・好み・作業の文脈は、特定LLMの重みや一時キャッシュにのみ閉じ込めない。Memoryの所有・移行・忘却、OSの正常なGUI操作、AIが利用できない場合の通常機能は既存決定とロードマップに従う。

## ユーザー体験の例（実装済みを意味しない）

- **主経路 — 習慣学習:** 利用者が毎月行うSheetsの集計とSlidesへの反映を、Nagiが許可された操作・文脈から繰り返し把握する。採用が確認できた作業方法について「今月も同じ手順を使いますか」「Workflowの草案として保存しますか」と提案する。ユーザーは手順や対象、必要な権限を確認して選ぶ。
- **補助経路 — Teach Mode:** 利用者が「この操作を教える」と明示的に開始し、対応アプリで手順を示す。Nagiは対応できたAction／不明な条件を分けてWorkflow草案を提示し、疑問点を確認する。承認までは保存済み・実行可能なWorkflowと扱わない。
- **別経路 — 自然言語・GUI:** 実演も反復履歴もない作業について、ユーザーが言葉で目的を伝えたりGUIで構造化Workflowを作ったりできる（DEC-033）。

## 未決定事項（PROVISIONAL）

- Teach Modeの正式名称、起動・停止・記録範囲選択・削除のUI、説明・同意の粒度、正規イベント／Action／アクセシビリティ等の取得方式。画面録画・キーストローク監視を採用決定しない。
- 実演データから目的・条件・ループ・例外を推定する方式、複数実演の統合、曖昧な意図の質問方式、対応純正／第三者アプリ、対象フォーマット、再現性の検証基準。
- 学習データの保持・集約・Context検索の詳細、提案するタイミング／頻度、習慣と認める重みづけと回数、教示と習慣が競合した場合のUI。DEC-004の明示指示優先と暫定の数値は維持する。
- 実装版、Workstream、Acceptance、性能・電力・推論負荷・信頼性の数値。0.3のPersonal Agent／Memory設計方針と接続するが、Teachの提供版を新たに確約しない。

## 既存決定とロードマップとの関係

- **DEC-004:** 自然なMemory・習慣学習、ユーザーの訂正、自然／明示忘却とモデル非依存を維持する。
- **DEC-028:** 習慣に基づくWorkflow提案を中心とし、正式保存／有効化／改訂はユーザー承認を必要とする。
- **DEC-033:** 自然言語・GUIの別入口を残す。Teachはさらに加わる補助的な作成経路であり、これらを置換しない。
- **0.3改訂ロードマップ:** 個人Profile、Memory、Learning／Weightingの基本方針に沿う。実装の順序、Release・Activation／Acceptance Gateを追加・変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーが習慣学習を主経路とし、Teach Modeは一つの任意機能として設ける方針を明確化。自然な学習・訂正・Memoryの継続性と、承認済みWorkflowの既存境界を維持。Teachの詳細・提供時期は未確定。 |
