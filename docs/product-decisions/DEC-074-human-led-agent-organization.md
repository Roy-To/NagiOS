# DEC-074 — 人間が管理する階層型AI組織

**状態:** CONFIRMED（当面のAI組み込み体験優先・階層型Agent組織は非優先の長期ビジョン、PX-15の実用1.0 B＋将来の総合OS Cとの高水準整合）／PROVISIONAL（組織構造・技術・版割当・実装順序、1.0の具体的試験・公開範囲）

**最終合意日:** 2026-11-22（DEC-122のCouncil高水準方向を追記。階層型組織の後順位は不変）  
**適用候補:** Nagi OSの製品優先順位と長期構想。特定Releaseへの割当ては未決定。

**関連:** [DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-010](DEC-010-intent-first-adaptive-ui.md)、[DEC-015](DEC-015-builder-role-review.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-021](DEC-021-roadmap-0.5-1.0.md)、[DEC-023](DEC-023-work-data-centric-os.md)、[DEC-024](DEC-024-external-jev-reference-and-builder-label-correction.md)、[DEC-122](DEC-122-adaptive-supervisor-council-security.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[VISION-001](VISION-001-human-adaptive-os.md)、[0.4+ Multi-Agent Organization Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)、[0.5–1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

## 目的とユーザー体験

利用者がAIを一体の会話相手として使うだけでなく、仕事の目的に応じて専門AIへ組織的に委ね、規模や階層を段階的に広げられるOSを目指す。利用者は組織を管理する人間として、方針や優先順位を示し、必要な判断・承認・停止を行う。

## 確定した仕様

- **当面の製品優先順位:** AIがOS標準機能として自然に働く、**AI組み込み体験型OSとしての確立を優先する**。Nagi-first／Intent-first、作業・データ中心の体験と、許可されたContextから正規のActionへつながる既存方針を土台にする（DEC-009／010／020／023）。
- **階層型Agent Organizationは非優先の長期ビジョン:** 人間Manager・複数Supervisor・複数Staffという方向は保持するが、当面の製品体験の成立条件や短期の開発優先項目にはしない。以下の組織方向のCONFIRMEDは、その早期提供や特定版への実装を確約しない。
- **人間が唯一のManager:** この個人向け製品構想では利用者1人が唯一の人間Managerであり、AI組織の目的、優先順位、監督を担う。
- **複数のSupervisor AI:** 一人の人間の下に複数の統括AI（Supervisor AI）が存在してよい。各Supervisorの下で、複数の専門Worker Agent（Staff）が割り当てられた仕事を行う。
- **階層と規模の拡張方向:** 端末の計算能力や利用可能Resourceに応じて、AI組織の規模や階層を将来拡張できることを目指す。Resourceが増えた場合にAgentを自動追加することや、性能向上を保証するものではない。
- **Nagiを一貫したユーザー窓口とする既存方針を維持する。** NagiがSupervisorの一員か、別個の調整役かは本決定では定めない。
- Supervisor、Worker、Roleという組織上の責任は、独立したProcess、Agent identity、Session、Model、端末、同時並列実行を必須としない。これらの構成方法は未確定である。
- これは長期的な製品方向であり、0.1〜1.0の既存Release Gate、Acceptance、Workstream、実装仕様を変更しない。

## Supervisor Councilとの接続（2026-11-22）

[DEC-122](DEC-122-adaptive-supervisor-council-security.md)に、ローカルLLMネイティブなCouncilを将来のSupervisor相互レビュー・安全確認に用い、端末Resourceに応じてSupervisor数や運用を柔軟に変える**CONFIRMEDの高水準方向**を追加した。OS本来の認証・Permission／Policy・Action認可はCouncilとは独立し、合議結果はユーザーの本人承認・権限を代替しない。

低スペック端末でSupervisorが1体のときCouncilを省略するか重大案件を直列3回推論させるか、重大／致命的変更の合議・全会一致の具体条件、モデル／審査の独立性、実装・提供版は**PROVISIONAL**。階層型組織の後順位と初期0.4の単一Nagi `builder` Role、既存Gateは不変。

## 当面実証したいAI組み込みOS体験（REVIEW NOTE）

既存の製品方針を短い利用シナリオで確認する候補を示す。新しいRelease Gate、実装順序、提供版、達成済み機能を定めるものではない。

- 「昨日の調査を資料にまとめたい」という目的から、許可された資料と作業Contextを示し、対応アプリの型付きPublic Actionで作業を進められる。
- Home／Desktopや作業を切り替えても会話・Task・Workspaceの連続性を保ち、進捗確認、承認、停止に自然に到達できる。復元だけで中断Taskを再実行しない。
- 対応するローカル構成で応答性・成果品質・資源使用を測り、AIが使えない場合も通常GUIで作業を続けられる。性能向上は実測前に断定しない。

競合の未公表設計や能力の推測は、本決定の根拠や差別化の実証としない。Nagi自身の体験と検証結果で評価する。

## PX-15「実用1.0 B＋将来総合OS C」との整合（2026-11-17）

[DEC-021](DEC-021-roadmap-0.5-1.0.md)のPX-15として、**正式対応するPC／Targetおよび機能の範囲で、日常の代表作業をNagiのAIと通常GUIを利用して安全・安定に完結・継続できる実用OS B**を1.0の高水準到達方針とし、**将来は対象Hardware・アプリ・利用形態を段階的に広げる総合OS C**へ発展する方向がCONFIRMEDになった。

**本DECのCONFIRMEDを維持:** 当面はAIをOSの標準機能として自然に使える体験の確立を優先し、**階層型Agent Organizationは非優先の長期ビジョンであって1.0の必須条件ではない**。初期0.4の単一Nagi `builder` Role、Nagiを一貫した利用者窓口とする既存方針、Scope／Permission／Policy・Task停止／結果の真偽を維持する。すべてのAgent・Supervisor／Workerを1.0の一般公開品質と混同せず、将来の採用方向も取り消さない。

**PROVISIONAL／OPEN:** 1.0に正式対応する代表作業・端末・アプリ・性能／品質の閾値、具体的受入と公開判定、階層型Agent組織の構成／実装／提供版。PX-15は新しいAgent実装開始許可や既存Release／Activation／Acceptance Gateの変更ではない。

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
- AI組み込み体験の具体的な提供版・実装順序・評価シナリオの採用、および長期組織構想を優先項目へ移す時期・条件。
- 複数人・複数Profile・複数端末を含む組織への拡張。本決定は個人利用の人間Manager一人という長期構想を記録し、Multi-user設計を決めない。

## Roadmapとの関連

- **0.3:** 単一Personal Agentとしての既存境界を維持する。
- **0.4:** 単一Nagiのbuilder Role方式と既存のEntry Gateを維持する。
- **0.5以降:** 0.4+ Multi-Agent Organization Roadmapと0.5–1.0 Master RoadmapのPhase案は段階拡張候補であり、本決定が具体的Versionや開始時期を確定するものではない。
- 各後続Releaseの実装・Activationは、そのReleaseのAcceptance、Security／Permission契約、明示的な判断に従う。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-22 | DEC-122に、Councilの将来採用方向、独立したOSセキュリティ、端末Resource適応を記録。低スペック時のCouncil省略／直列3回推論および票数・リスク分類・導入時期はPROVISIONAL。 |
| 2026-11-16 | ユーザーが、人間を唯一のManager、複数可能な統括AIをSupervisor、その下の専門AgentをWorkerとする長期構想、および端末能力に応じて組織規模・階層を拡張する方向を確認。具体的構造・実装・版割当は未決定として記録。 |
| 2026-11-16 | 後続確認：当面はAIがOS標準機能として自然に働くAI組み込み体験型OSの確立を優先し、階層型Agent Organizationは非優先の長期ビジョンとする。既存0.3／0.4 Gate・builder Roleは維持し、具体的な提供版・階層数・実装順序は未確定。 |
| 2026-11-17 | PX-15の1.0実用OS Bと将来総合OS Cという高水準完成方針へ整合。AIがOS標準体験として自然に働く当面の優先順位と階層型Agent Organization後順位を維持。代表体験・アプリ・端末・性能・Release判定・組織の版はPROVISIONALで、既存Gateを変更しない。 |

| 2026-11-16 | PR #113の旧DEC-052をDEC-074へ改番。実機候補DEC-052との番号競合のみを解消し、当面の優先順位と長期構想・状態・履歴を保持。 |
