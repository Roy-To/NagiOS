# DEC-122 — 端末能力適応型Supervisor Councilと独立したOSセキュリティ

**状態:** CONFIRMED（Councilの主目的を重要Action・疑わしい指示へのセキュリティ審査とする長期方向、通常の深掘り目的で常時合議しない方針、端末Resource適応、独立したOS認証・認可）／PROVISIONAL（審査トリガー・票数・人数、低スペック時の省略／直列3回推論、Cloud深掘りの具体ルーティング、性能・実装・適用版）  
**最終合意日:** 2026-11-22  
**適用候補:** 将来の階層型Multi-Agent Organization（0.3／初期0.4の追加要件ではない）  
**関連:** [DEC-074](DEC-074-human-led-agent-organization.md)、[DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-002](DEC-002-cloud-ai-data-sharing.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)

## 目的とユーザー体験

将来のWorkerの作業をSupervisorがレビューし、Nagiを通じてユーザーに報告する組織運用を検討する。悪意ある外部情報・プロンプトインジェクション・偽のユーザー指示などにSupervisorの1体が影響されても、危険な指示が多数のWorkerへ連鎖しにくいよう、複数Supervisorの合議（Council）による追加的な安全確認を目指す。具体的なレビュー・報告Workflowは未確定。

## 確定した仕様（長期の高水準方針）

- **Councilの主目的はセキュリティ:** NagiのローカルLLMネイティブな強みとしてSupervisor同士の合議・相互レビューを将来の階層型Agent組織に取り入れる。**特に危険なAction、権限逸脱の兆候、偽のユーザー指示やPrompt Injectionなどの安全審査を主目的**とし、普段の会話・回答品質向上・多角的な深掘りのためにCouncilを常時起動しない。
- **通常のWorkerレビューとは区別:** Supervisorが配下Workerの成果物を確認し、ユーザーに報告する通常業務は維持する。ただし、そのすべてをCouncilの複数Supervisorによる合議にかけることは要求しない。
- **計算資源の用途を分ける:** 普段のNagi操作はローカルの単独推論で成立させる。深い調査・高度な推論には、必要に応じて許可された外部Cloud LLMを活用する方向を選択肢として維持し、Councilのローカル計算資源は安全性が必要な場面へ重点配分する。外部LLMは必須ではなく、利用可能なLocal ModelやOffline利用を排除しない。
- **独立した中核セキュリティ:** 認証・Permission・Capability・Policy・Public Action・委譲制限とユーザーの必要な最終承認は、Councilとは独立してOS／Providerが強制する。Councilで賛成・全会一致が出ても権限や本人承認は生まれない。未信頼の文章・Agent発言をユーザーの正規の指示と同一視しない。
- **端末能力に応じて柔軟化:** 利用可能なCPU・メモリ等のResourceに応じ、Supervisorの人数、組織規模、Councilの運用方式を将来柔軟に設計する。Supervisor数や推論の同時実行を固定の最低動作要件とはしない。
- **低スペック端末でも中核安全性は不変:** Councilを利用できない構成でもOSの認可・禁止・Task Scope・本人承認は維持し、CouncilがあることもないこともPrivilegeを生まない。
- **既存方針を維持:** DEC-074のAI組み込みOS体験優先／階層型組織後順位、0.3の単一Personal Agent、初期0.4の単一Nagi `builder` Role、既存のRelease／Activation／Acceptance Gateを変更しない。

## 例外・禁止・安全境界

- Supervisorが「ユーザーから指示された」と主張しても、OSが認証・許可した指示の証明にはならない。
- 合議結果は、未承認の外部送信、破壊的Action、権限拡大、再委譲、停止Taskの再開などを承認しない。各Action時点で現行Permission／Policyを検証する。
- Councilも既存のProfile／Task／Agent間のデータ境界、Stop／Cancel、Activityと結果の真偽を維持する。
- **Cloud利用はDEC-002に従う:** 外部LLMへの高度な推論依頼も、許可されたProvider・データ分類・Purpose・Resource／Task Scopeに限定する。自動的なCloud送信や秘密情報の無断転送を新たに許可せず、Cloudを使えない場合のローカル経路を維持する。

## 未確定・暫定案（PROVISIONAL）

- Councilを招集する具体的なRisk基準・対象Action・異常検知方法、通常のWorkerレビューからCouncil審査へ移る境界、審査による効果を測定する条件。
- 普段の高度調査で外部Cloud LLMを使うかどうかの個別Task判断、ローカルとの使い分け・Model選択・費用・応答・UI。Cloudに依存しない基本利用とDEC-002の同意境界は維持する。
- **重大操作の合議:** 複数Supervisorで可否を審査し、特に致命的な変更には全会一致を求める構想。合議の必要条件、重大・致命的の分類、参加人数、賛否・棄権・不明の扱いは未確定。
- **低スペック構成:** Supervisorが1体となる環境ではCouncilを無理に使わない案と、重大案件に限り直列3回の推論を行って確認する案の双方を検討する。採否・回数・条件は未確定。3回の推論が独立した3体の投票になるとはみなさない。
- Supervisorを2体以上設けられる環境での人数・Role・モデル・Contextの独立性、審査分担、負荷変動時の参加数調整、Reviewerの選定。
- Councilが判断不能／不一致／Resource不足の場合の安全な保留・本人への確認・追加検証、Task中断時の扱い。
- Prompt Injectionへの耐性評価、共通情報に汚染された複数AIの判断の相関、偽の投票・Agent ID等の防止策、ログ・説明UI・性能・消費電力。
- 実装Schema、Risk taxonomy、投票閾値、どのReleaseで設計・実装・検証するか。

## REVIEW NOTE（設計論点・ユーザー決定ではない）

同一モデルに同じ根拠を渡して3回推論する場合、判断の相関が強くなる可能性がある。計画・安全性・反証といった異なる観点の逐次レビューは検証候補だが、3体の独立審査と同じ保証を与えない。Councilの不成立を理由にOSが既定で実行を許可することは避ける。

## Roadmapとの関連

- **0.2:** Identity／Permission／Public Action／Model Runtime／Resourceなど、AIの合議に依存しない基盤。
- **0.3:** 単一Personal Agentの既存Acceptanceを維持する。
- **初期0.4:** 単一Nagi `builder` Roleの既存方式・Entry Gateを維持する。
- **後続0.4+／将来:** 階層型Agent組織でのCouncil導入候補。提供Version・実装開始・必須Acceptanceは未決定。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-22 | ユーザーが、CouncilをローカルLLMネイティブな追加審査層として活用する高水準方向、中核OSセキュリティの独立、端末能力に応じたSupervisor規模の柔軟性を確認。低スペック時の省略・重大案件の直列3回推論や致命的操作の全会一致の詳細はPROVISIONALとして保持。 |
| 2026-11-22 | Councilの主目的をセキュリティ審査とし、日常の回答深掘り・常時合議を主用途にしない方向を追加確認。高度な調査では外部Cloud LLM利用を有力な選択肢とするが、利用可否・具体ルーティングはPROVISIONAL、DEC-002の送信同意とOffline-firstを維持。 |
