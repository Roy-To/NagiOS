# DEC-052 — 人間が管理する階層型AI組織

**状態:** CONFIRMED（長期的な製品方向）／PROVISIONAL（組織構造・技術・版割当）  
**最終合意日:** 2026-11-16  
**適用候補:** Nagi OSの長期構想。特定Releaseへの割当ては未決定。  
**関連:** [DEC-015](DEC-015-builder-role-review.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-021](DEC-021-roadmap-0.5-1.0.md)、[DEC-024](DEC-024-external-jev-reference-and-builder-label-correction.md)、[VISION-001](VISION-001-human-adaptive-os.md)、[0.4+ Multi-Agent Organization Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)、[0.5–1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

## 目的とユーザー体験

利用者がAIを一体の会話相手として使うだけでなく、仕事の目的に応じて専門AIへ組織的に委ね、規模や階層を段階的に広げられるOSを目指す。利用者は組織を管理する人間として、方針や優先順位を示し、必要な判断・承認・停止を行う。

## 確定した仕様

- **人間が唯一のManager:** この個人向け製品構想では利用者1人が唯一の人間Managerであり、AI組織の目的、優先順位、監督を担う。
- **複数のSupervisor AI:** 一人の人間の下に複数の統括AI（Supervisor AI）が存在してよい。各Supervisorの下で、複数の専門Worker Agentが割り当てられた仕事を行う。
- **階層と規模の拡張方向:** 端末の計算能力や利用可能Resourceに応じて、AI組織の規模や階層を将来拡張できることを目指す。Resourceが増えた場合にAgentを自動追加することや、性能向上を保証するものではない。
- **Nagiを一貫したユーザー窓口とする既存方針を維持する。** NagiがSupervisorの一員か、別個の調整役かは本決定では定めない。
- Supervisor、Worker、Roleという組織上の責任は、独立したProcess、Agent identity、Session、Model、端末、同時並列実行を必須としない。これらの構成方法は未確定である。
- これは長期的な製品方向であり、0.1〜1.0の既存Release Gate、Acceptance、Workstream、実装仕様を変更しない。

## 例外・禁止・安全境界

- **初期0.4は既存どおり:** DEC-015の確認に従い、初期の0.4は単一のNagiがbuilder Roleへ切り替えて、範囲を限定した開発Taskを実行する。複数Supervisor、独立Child Agent、並列実行は初期0.4の要件ではない。
- SupervisorやWorkerの役職、Agent数、モデルの判断、成功申告は、Permission、Capability、Action実行権限やユーザー承認を生まない。
- 委譲は親から許可されたTask、Resource、Action、期間、Resource budgetの範囲を超えず、OSは各Action実行時に現行Policy／Permissionを検証する。副作用は型付きPublic Action等の正規経路で扱い、停止・取消と監査証拠を維持する。
- Agent組織の所属だけを理由に個人Profile Memory、Task data、Agent data、Team data、Organization dataを暗黙に共有しない。DEC-002、DEC-003、DEC-020等の既存データ境界を適用する。
- 人間が組織のManagerであるという製品概念は、OSのroot権限、管理者特権、またはAIへの包括的な許可を定義しない。

## 未確定・暫定案

- SupervisorとWorkerの数、職務、追加階層を作る基準、各Taskの分割・集約・レビュー方法。
- NagiがSupervisorか独立したユーザー窓口／調整役か、複数Supervisorとどう協調するか。
- Agent、Role、Process、Session、Modelの具体的な対応関係、ローカル／Cloud配置、並列実行とResource schedulingの方式。
- 端末性能・負荷・電力・応答品質をどう評価し、組織の規模をどのように調整するか。具体的な数値や性能保証はない。
- Manager向けの監督・進捗・委譲・承認・停止UI、AI間の共有可能なContext、Task履歴・Activity・証拠の保持期間。
- どのVersion／Releaseで設計・実装・Acceptanceを行うか。0.5〜1.0の既存案は候補であり、割当てではない。
- 複数人・複数Profile・複数端末を含む組織への拡張。本決定は個人利用の人間Manager一人という長期構想を記録し、Multi-user設計を決めない。

## Roadmapとの関連

- **0.3:** 単一Personal Agentとしての既存境界を維持する。
- **0.4:** 単一Nagiのbuilder Role方式と既存のEntry Gateを維持する。
- **0.5以降:** 0.4+ Multi-Agent Organization Roadmapと0.5–1.0 Master RoadmapのPhase案は段階拡張候補であり、本決定が具体的Versionや開始時期を確定するものではない。
- 各後続Releaseの実装・Activationは、そのReleaseのAcceptance、Security／Permission契約、明示的な判断に従う。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーが、人間を唯一のManager、複数可能な統括AIをSupervisor、その下の専門AgentをWorkerとする長期構想、および端末能力に応じて組織規模・階層を拡張する方向を確認。具体的構造・実装・版割当は未決定として記録。 |
