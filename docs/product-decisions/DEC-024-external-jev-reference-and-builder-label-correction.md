# DEC-024 — TypeSafe AI Jev参照の誤用訂正と開発担当の仮名称

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（誤用訂正・`builder` Role名・初期Nagi Role切替C）／PROVISIONAL（将来の独立Agent配置・実装詳細）  
**最終訂正日:** 2026-11-05  
**関連:** [DEC-015](DEC-015-builder-role-review.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-019](DEC-019-builder-work-data-cleanup.md)、[DEC-022](DEC-022-builder-interrupted-task-recovery.md)、[ADR-0018](../decisions/0018-decision-and-generative-model-architecture.md)、[Decision / Generative Architecture](../architecture/decision-and-generative-ai-architecture.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)

## 訂正の理由

ユーザーが参照した**TypeSafe AIのJev**は、状況と限定的な質問を受けて選択・スコア・確率等の型付き判断を返す外部のSystem Oneモデルであり、**Nagi OSの開発・コーディング担当Agentの固有名ではない**。以前の相談における「Jevのような役割」という例示を、アシスタントがNagi OS内の専門Agentの名称と機能に誤って転用した。ユーザーがその混同と位置づけの誤りを指摘し、名称変更を要求したため訂正する。

## 確定事項

1. **Jevの名称流用を停止する。** Nagi OSの開発・実行担当を「Jev」と呼ばない。TypeSafe AIの製品名Jevは、その外部製品を正確に指す場合のみに使用する。
2. **モデル／判断機能とAgent／Roleを混同しない。** TypeSafe AIのJevを参考にするのは高速で型付きの判断能力（分類、Choice／Score、ルーティング、候補絞り込み等）であり、Nagiに新しい独立Agentを追加すること、コーディングを担当させること、TypeSafeのJevそのものを導入すること、同じモデル構造を実装することを意味しない。
3. **既存の正しいArchitectureを保持する。** 2026-09-19のADR-0018およびDecision / Generative AI Architectureでは、Deterministic Fast Path、DecisionProvider、Generative/Reasoning Providerを区別し、DecisionProviderを将来交換可能とする。専用判断モデルやJevの導入は必須でなく、既存ローカルモデルを用いたLlmDecisionAdapterでも動作できる。権限・Action実行は決定論的なValidator／Policy／Permission／Executorが担う。
4. **混同していた開発担当の仮ラベル:** 既存DECで「Jev（Build / Execute）」と記載されたコード修正・テスト担当は、当面**Builder（仮称）**と呼び、TypeSafe AIのJevと区別する。これは正式名称の採用や独立Agentとしての配置の再確定ではない。
5. **位置づけは再整理する。** 元の例示だけから、Builderを必ず初回0.4で独立したAgentにする、最初の専門Agentとして固定する、または専用LLMを持たせるという結論を導かない。DEC-015における名前・概念上の前提は**REVIEW REQUIRED**として扱い、Nagi、Role、Agent、DecisionProviderの分離を踏まえて再検討する。
6. **従前の機能・安全方針の扱い:** 既存DEC-016／018／019／022でユーザーが選択した、承認Scope内作業・検証・条件付き反映・整理・中断後の復旧といった動作要件は履歴として保持する。ただし、これらを「TypeSafe AIのJevに相当するモデルの仕様」と扱わず、どのAgent／Roleに適用するかは再確認対象とする。無許可のGit commit／push・権限昇格・自動再実行は認めない。

### 2026-11-05 追記：Role名の正式決定（CONFIRMED）

ユーザーはプログラミング担当のRole名を **`builder`（小文字）** と確定した。以下の「Builder（仮称）」といった過去の記述は、命名確定前の履歴である。`builder` が単独Agentとして動くか、Nagi等のAgentがRoleを切り替えるか、0.4の初回構成は引き続き未確定とする。

### 2026-11-05 追記：Jev相当の限定判断機能の採用方針（CONFIRMED）

ユーザーはTypeSafe AIのJevのような高速・限定的判断機能について、**新しいAgent／Roleを作らず、既存のDecisionProviderを利用する**方針を確定した。2026-09-19の[ADR-0018](../decisions/0018-decision-and-generative-model-architecture.md)と[Decision / Generative AI Architecture](../architecture/decision-and-generative-ai-architecture.md)の既存境界を維持する。

- **実用化の優先順位:** まず既存の型付き`DecisionProvider`契約と判断ルーティング、既存ローカル`GenerativeProvider`を使う`LlmDecisionAdapter`代替経路を完成・検証する。新たな専用判断モデルの採用を、基盤完成の前提条件にしない。
- **専用小型判断モデルは追加候補に限定:** 速度、メモリ、消費電力だけでなく判断精度・失敗率も既存経路と比較し、実測で有効性が確認された場合に`DecisionProvider`の差し替え可能なローカルProviderとして採用を検討する。ベンチマーク結果なしに採用、常駐、性能向上を確約しない。
- **独立Agent／Roleを追加しない:** 限定判断はNagi OSのCapability／Provider経路で扱う。`builder`はプログラミング担当Roleであり、この機能と混同しない。
- **外部Jevの直接統合は不要:** Jev SDK、TypeSafe AI製品、専用モデルやCloud接続を必須としない。将来のProvider差し替え可能性は閉じず、外部通信には既存のPrivacy／Permission規則を適用する。
- **既存の安全境界:** 判断値・Score・Confidenceは実行権限ではない。型付き結果からのActionは既存のValidator／Policy／Permission／Executor、Activity／Undo／Waybackの条件に従う。通常OS操作のAI非依存とOffline-firstを維持する。
- **Gateを変更しない:** 本決定は既存0.1〜0.4のAcceptance、Workstream所有権、Integration Owner checkpoint、Release／Activationを変更せず、実装済みを意味しない。

**PROVISIONAL:** 専用判断モデルの候補、採用判断の数値閾値、比較Task／Benchmark、導入時期・対象Version、具体的なRuntime／Provider構成は未確定とする。

### 2026-11-05 追記：`builder`実行方式Cの確定（CONFIRMED）

プログラミングRole `builder` は、**初期にはNagi自身が必要時にRoleを切り替えて実行する**。作業Task、作業領域、許可Scope／権限評価、検証証拠、履歴をTask単位で区別し、独立した専門Agentや専用LLMを初期必須としない。将来は同じRoleを独立専門Agentへ担当させられる構成とする。旧「Nagi＋別開発Agent二体」を初回0.4の必須構成とする案、および本書の初回構成未確定という以前の記述は、この新しい決定により置き換える。Role変更はPermissionを付与せず、正式なOS Policy／Provider検査、Public Action、Activity／復旧条件、0.3／0.4の既存Gateを維持する。詳細は[DEC-015](DEC-015-builder-role-review.md)を正本とする。独立Agent移行時期、Model／Runtime、数値Resource Budget、Acceptance詳細は未確定。

## 未確定事項

- `builder` のRole名と初期Nagi Role切替方式Cは確定済み。将来の独立専門Agent導入条件、Runtime／Model選択、実装詳細は未確定。
- DecisionProviderの専用モデルは実測で効果が確認できた場合のみ追加採用を検討する（上記CONFIRMED）。候補、採用閾値、技術／品質／Resource予算、対象Versionは未確定。既存ADRのFallback／Security制約は維持する。
- 以前の「最初の専門Agent」選定について、誤解に基づく部分の正式な再決定。

## 実装／公開境界

この訂正はPrivateの製品仕様に限る。Public実装仕様・AGENTS.md・RoadmapのRelease Gate・Activation／Acceptanceを変更せず、0.4の実装開始を許可しない。

## 履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | ユーザーがTypeSafe AIのJevとの混同と0.4の位置づけを指摘し、少なくとも名前の変更を要求。Jevの流用を撤回し、開発担当をBuilder（暫定）と表記。Agent／Role配置の再検討を明記 |
| 2026-11-05 | Jev相当の限定判断機能は既存DecisionProviderとローカルLlmDecisionAdapterを優先し、専用モデルは実測で有利な場合に追加検討。新規Agent／Role・Jev直接統合は不要と確定 |
