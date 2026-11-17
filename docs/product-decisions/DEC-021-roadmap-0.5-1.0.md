# DEC-021 — 0.5〜1.0ロードマップ境界と段階案

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（従前の製品方針・1.0一般公開／開発者環境・配布基盤の高水準目標、DEC-074の長期方向、PX-13のCloud AI橋渡し／移植補助優先・高度統合制作環境の低優先という高水準順序）／PROVISIONAL（版別詳細・具体対応・完成時期・実装順序／Acceptance）  
**最終確認日:** 2026-11-17（PX-13後期優先の方向を補足。既存1.0・Gate目標を維持）  
**適用候補:** Nagi OS 0.5〜1.0  
**関連:** [DEC-007](DEC-007-roadmap-alignment.md)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-010](DEC-010-intent-first-adaptive-ui.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-012](DEC-012-workspace-naming-organization.md)、[DEC-013](DEC-013-lightweight-memory-engine-benchmark.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-015](DEC-015-builder-role-review.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-019](DEC-019-builder-work-data-cleanup.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-074](DEC-074-human-led-agent-organization.md)、[改訂0.3 Roadmap](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)、[0.5〜1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

## 目的と位置付け

0.4以降についてユーザーが承認した製品の方向を、0.5〜1.0の計画案へ接続する。引き継ぐ製品境界、改訂0.3 Roadmapが示す1.0の高水準目標、DEC-017が示す1.0の開発者・配布目標、版ごとの提案、未決定事項を区別する。

本DECとMaster Roadmapは、実装仕様、Release commitment、Release activation、Workstream開始許可ではない。既存の0.1 / 0.2 / 0.3 / 0.4のGate、Acceptance、Workstream ownerを変更しない。各後続版も、適用される前版のTarget Runtime acceptanceと個別のactivation判断を経る。

## 確認済みの製品方針と境界

### 2026-11-16 確認：当面のAI組み込みOS体験を優先

[DEC-074](DEC-074-human-led-agent-organization.md)の後続確認により、当面はAIがOS標準機能として自然に働くAI組み込み体験型OSの確立を優先する。階層型Agent Organizationは非優先の長期ビジョンとして保持する。以下のPhase・版別配置は候補のままであり、組織の拡大を短期の優先項目や1.0の必須要件へ昇格させない。具体的な提供版、階層数、実装順序はPROVISIONAL。既存0.3／0.4 Gateと初期の単一Nagiによるbuilder Role方式は変更しない。

### 2026-11-17 — PX-13アプリ制作環境の後期優先とCloud AI橋渡し

[DEC-071](DEC-071-nagi-native-app-creation-experience.md)にPX-13のB（対話しながら設計・修正・試用する共同制作）＋将来C（許可されたMemory等を踏まえ後日継続改良する高度共同制作）の**高水準製品方向**を記録した。一方、すでに一般のCloud開発AI／既存IDEで自然言語による開発が可能なことを踏まえ、**Nagi単体で完結する高度な統合開発環境の早期完成は優先しない**。ローカルLLMがNagi公開SDK／API・移植診断の情報をCloud AI／既存IDE・人間へ安全に橋渡しし、他OSアプリをNagi Nativeへ移植する支援を、当面の開発者体験の優先方向とする（[DEC-045](DEC-045-nagi-porting-toolkit.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)）。

**CONFIRMED（順序の高水準方針）:** OS基本動作・日常AI体験を優先し、高度な統合アプリ制作環境は**1.0計画の最終盤を見据える低優先の長期項目**とする。**1.0での具体的な完成・搭載を約束せず、1.0正式公開の新しいGate／必須条件にしない**。一方で、DEC-017の**1.0までの公開SDK／開発者環境・アプリ配布基盤という既存CONFIRMEDの高水準目標は維持**し、PT-01のhost-only診断等の既存許可済み準備を止めない。

**安全・未決:** Cloud AIによるコード／Context生成・交換はDEC-002のProvider／Purpose・Resource・機密分類等の事前許可と現行Policyに従い、ローカルLLMがCloud送信の権限を持つわけではない。外部AI利用の既定設定、Connector／技術・具体の対象モデル、開発支援の段階・性能／対応OS範囲、実装者と提供版・時期はPROVISIONAL。既存の0.1〜0.4 Gate・Workstream／Action Scope・試用／正式導入の別承認を維持する。

### 2026-11-05 確定：初期`builder`実行方式C

初期0.4ではNagi一体が`builder` Roleへ切り替えて開発Taskを実行する（DEC-015）。Task・作業領域・Permission scope・証拠を区別し、旧「Nagi＋Builder独立二体」を初回必須とする案は採用しない。将来は独立専門Agentにも同じRoleを割り当て可能にする。以下の旧二体構成・配置未確定と記載した箇所は、この確定を優先して読む。既存Release Gateは変更しない。

### ReleaseとAgent Team

- **0.3から0.4への境界:** 0.3 Personal AgentのAcceptance後に、0.4を別途有効化する。0.4+ Roadmapは後続のTeam構想と各Phaseの案を示すが、将来Phaseの版割当てや実装開始を確定しない。
- **初回0.4確定方式（DEC-015／DEC-024）:** Nagi一体が`builder` Roleを切り替えて、分離されたTask・作業領域・権限Scopeで開発作業を実行する方式C。別の専門Agentは将来の候補とし、旧二体必須案は置き換える。Research / Review / Verifierも後続Phaseの候補。
- **Nagiが実行する`builder` Roleの作業動作要件（DEC-016）:** 承認されたTask / Workspace / Resource / Action scope内で調査、修正、Build / Test、必要な再修正を行える。Git commit / pushは既定権限に含めない。通常の検証済み作業はNagiが完了判定でき、重要変更、検証失敗・不能、Scope外などはユーザー確認へ回す。risk threshold等の実装詳細は未確定。
- **隔離編集（DEC-018）:** Nagiの`builder` Roleは原本を保護した隔離領域で作業する。Nagiは差分、検証証拠、承認Scope、現在のPermission、対象Resourceの現行revision / 競合状態、原本内容の復旧可能な保護を確認する。通常・Scope内・検証済み・非競合で保護に成功した変更だけを、実行時に再認可されるPublic Action経由で条件付き反映できる。不確実・高リスク・失敗・競合・保護失敗等はユーザー確認へ回す。Git commit / push / production反映は別許可。
- **作業データ整理（DEC-019）:** Task専用の一時データは、安全な反映・検証の後、必要な差分、証拠、Activity、復旧情報が保持されたことを確認した場合に限り整理できる。失敗、部分成功、確認待ち、停止・中断、状態不明、共有中または保護対象のデータは保持する。所有者、使用中か、参照関係、Permission、保持Policyが不明なら整理を保留する。保持期間・容量上限は未確定。
- **Nagiをユーザー窓口とする:** Agentが増えても、対話、承認、進捗、成果報告の一貫した窓口はNagiとする。RoleやModelは権限を与えない。成果はAgentの成功申告だけでなく、再現可能な証拠に照らして判断する。
- **DEC-074 — 人間が管理する階層型AI組織:** 長期的には、ユーザー1人が唯一の人間Managerとなり、複数の統括AIと各統括AI配下の専門Worker Agentが協働する組織を目指す。必要な規模・階層は利用可能な端末資源に応じて将来拡張できる方向を確認する。Nagiを一貫したユーザー窓口とする既存決定および初期0.4の単一Nagi builder Role方式を維持する。NagiとSupervisorの関係、組織構造・実行方式・適用版は未確定であり、本DECは版割当、独立Agent、並列実行、Permissionの変更を決めない。

### 既存製品体験（DEC-009〜014）

- Nagi Homeを標準の対話画面とし、Nagi Desktopで通常のGUIを利用できる。両者やWorkspaceを切り替えても状態を保ち、「前回の続き」や復元だけで中断Taskを再開・再実行しない（DEC-009）。
- Intent-first UIを保つ。新Workspaceには目的に合う複数画面配置を用意できるが、既存Workspace、固定ウィンドウ、手動配置を勝手に大きく変えない（DEC-010）。
- 承認は会話カードと展開確認の組合せを基本とし、重大操作には明示的な画面確認を要する。Risk thresholdと進捗UIの詳細は未確定（DEC-011）。
- Workspaceの命名・順序をユーザーが制御する。Nagiは理由付きの候補と代替候補を示せるが、復元対象はユーザーが選ぶ。古さや利用頻度によるWorkspaceの自動削除、Taskの自動再開を行わない（DEC-012）。
- Workspace状態は重要な変更時と定期補助で保存する。名前付きcheckpoint、通常autosave、復元前保護状態を区別し、許可されたmetadataでWorkspace横断検索と復元previewを行う。既定の復元先は新Workspace。元Workspaceへ戻す場合は先にその状態を保護し、Workspaceごとの直近保護状態と、期間未確定の複数世代を保つ。Workspace本体、名前付きcheckpoint、直近保護状態は自動削除しない。ユーザーが復元先を選び、Permission / Privacyと手動配置を尊重する。ファイル本文の無断検索・巻戻しやTask再実行は行わない（DEC-014）。保存間隔、世代数、期間、容量、UI、実装Versionは未確定。

### Memory、開発者環境、AIとOSの境界

- **Hardware-Adaptive Memory Engine（DEC-013）:** 正式runtime実装は既存の0.3 Implementation Specificationとactivation gate内に置く。0.1 host-only benchmark準備や0.2 Forward CompatibilityはTarget Runtime acceptanceではない。ユーザー所有のmodel-independent Canonical Memoryを維持し、hardware依存の検索index、cache、Context adapter等の派生物だけを再生成できるようにする。端末間の移行でMemoryの再学習を要求しない。profile名、方式、切替条件、閾値、実測性能は未確定であり、0.3実装を0.5へ移さない。
- **開発者環境・配布（DEC-017）:** 0.1 host-side準備、0.2の既存Platform Foundation、0.3のPublic Action統合という各既存仕様・Gateを維持し、公開開発者環境とアプリ配布基盤を1.0の製品目標とする。AI codingに適した開発体験は方針として確認済み。portableなRust / C / C++アプリの移植可能性を理解し、再利用しやすくすることも製品目標だが、個々のアプリが変更なく動く保証ではない。機械可読catalog、CLI、sandbox、verification、toolchain、署名、Storeの役割、公開範囲、Acceptanceは未確定。Storeは唯一の配布経路ではない。
- **AIとOSの操作経路（DEC-020）:** 標準では小さく構造化された初期Contextから始め、必要に応じてResource参照を通じて詳細を段階取得する。取得のたびにPermission / Policy、情報分類・由来、owner、scope、Resource / revisionを再検証し、不明・競合・不一致なら取得しない。参照やLLMからの追加要求は権限を広げない。全ファイル・全ログ・raw screen dump・全Memoryの一括投入を標準にしない一方、DEC-002の許可済みCloud共有（非機密Memoryを含む）と、Scope内で広いContextを必要とする明示承認済みTaskを維持する。契約上公開されたEventを受け、状態変更には型付きPublic Action / Service APIと既存IPC / Providerを用いる。Event-drivenを既定とし、決定論的な仕事は通常Serviceで扱い、LLMは推論が必要な箇所で使う。Event検出だけでLLM推論、通知・画面割込み、Action実行を許可しない。人間向けGUIを維持し、スクリーンショット認識と座標クリックをNagi native操作の基本経路にしない。資源効率は測定する設計目標であり、削減効果や性能は実証・保証されていない。Context / Event schema、budget、取得条件、購読・集約、scheduler、legacy-app fallback、数値評価、Release割当ては未確定。

これらの製品方針は将来Versionを自動的に割り当てず、別のAction権限やRelease gateを与えない。

## 暫定の0.5〜1.0版別案

以下のVersion配置、順序、各版のscopeと0.8の設置は **PROVISIONAL**。改訂0.3 Roadmapがすでに示す「1.0の一般公開可能な品質・安定性・セキュリティ・互換性・開発者体験」という高水準目標を既存のRoadmap基準として保つ。DEC-017で確認済みの公開開発者環境・アプリ配布基盤の目標も維持する。測定可能な品質基準・具体的な実現範囲・Acceptanceは未確定とする。0.4+ Roadmapが示すPhase D–F等を計画上の候補として配置する。

| Version | 暫定案 | 依存・状態 |
|---|---|---|
| **0.5** | Phase Dを置く候補。Research / Review / Verifyを追加し、Worker成果を決定論的証拠と照合する構造化Reportを扱う。成功の自己申告だけではAcceptanceせず、競合時に新しいUser dataを上書きしない。DEC-018の隔離編集・現行Permission / Resource revision確認・原本保護・条件付きPublic Action適用と、DEC-019の完了後整理・必要な差分／証拠／Activity／復旧情報の保持、未完了・不明な作業データの保持を明示的に継承する。Offline workflow、有限Retry、Stop、Provider再起動、対応範囲のWaybackも検証候補とする。 | 0.4 Target Runtime acceptance後の別activationが必要。割当てはPROVISIONAL。 |
| **0.6** | Resource-aware schedulingとAI event pathを評価する。端末能力・現在負荷に応じた処理経路、決定論的Serviceと推論の分担、必要Contextの段階取得、event購読・集約の候補を検討する。並列実行は逐次を基準とし、budget、隔離、取消、前景応答性を満たす範囲に限る。Latency、CPU、RAM、Storage、Power、Token、成果品質を測定し、効率向上を仮定しない。hardware依存index / Context adapterは再生成し、Canonical Memoryは保持する。 | 0.5のWorkflow証拠を受けて判断。profile、scheduler、event contract、fallback、数値目標はOPEN。 |
| **0.7** | Phase F候補。まずlocal single-userのTeam / Organization内で所有、Membership、共有、Retention、Export、Deletion、Audit、OffboardingのPolicyを定義する。DEC-014のWorkspace履歴、横断metadata検索、復元preview、直近保護状態と複数世代、ユーザー選択を実装候補として扱う。Multi-user / Multi-deviceはlocal single-user invariantsがPASSした後に別Gateで判断する。 | 版割当はPROVISIONAL。保管・保持・検索・UIの詳細とMulti-user activationは未確定。 |
| **0.8** | 必要性とユーザー承認があればMulti-user / Multi-device拡張の独立Gateを設ける。承認されない場合はlocal single-userの信頼性を強化する。 | 任意の候補。版の有無、scope、data sharing / syncはOPEN。 |
| **0.9** | 承認済み範囲を統合し、Security、Reliability、Migration、Compatibility、Target Runtime evidenceを整える。重大な新機能を追加するより、受入可能な品質証拠を揃える候補とする。 | 版割当・exit criteria・対応範囲はPROVISIONAL / OPEN。 |
| **1.0** | Security、Stability、Compatibility、Data Integrity、Diagnosability、Developer Experienceの品質基準を定め、公開開発者環境とアプリ配布基盤を目指す。AI-coding向け体験とportable Rust / C / C++アプリの移行支援を製品目標として引き継ぐ。 | 改訂0.3 Roadmapがすでに示す一般公開可能な品質等の高水準目標をRoadmap基準として保ち、DEC-017でCONFIRMEDの公開開発者環境・アプリ配布基盤の目標を引き継ぐ。測定可能な品質基準、具体的な対応対象、API / CLI / catalog / sandbox / verification、公開範囲、Acceptance、日程、Storeの役割はPROVISIONAL / OPEN。Organization規模・Multi-deviceを必須とはしない。 |

版別計画は実装証拠とユーザーの個別判断で変更する。先行Gateや安全境界が未達の場合は、機能を延期・分割し、承認やPermissionを弱めて帳尻を合わせない。

## 未確定事項

- Version番号、PhaseとVersionの対応、各Versionのscope、feature sequencing。
- 0.8を独立Versionとして設けるか、Multi-user / Multi-device / Remote Agent / Syncを1.0以前に扱うか。
- Team / OrganizationのOwner、Admin、Member、Guest、Membership変更、Recovery、Governance、Offboarding。
- 共有可能なdata、用途、Consent、Retention、Export、Deletion、Audit、Backup。
- Agent / Role packageの第三者導入、署名、審査、失効。
- DEC-013の具体的な検索・Memory profile、DB、index、Embedding、hardware signal、切替条件、migration性能、閾値。
- DEC-014の保存間隔、保持期間・容量、保護世代の数、検索index、画面表示、実装Version。
- DEC-016 / DEC-018のRisk taxonomy、必須検証、確認閾値、保護方式、resource budget。
- DEC-020のContext / Event schema、公開イベント、購読権限、優先順位・集約、推論scheduler、legacy-app fallback、metrics baselineと性能目標。
- 対応Model、voice provider、hardware profile、parallelism、fairness、numerical budget。
- DEC-074の組織構造：Supervisor数・役割・階層、各階層のAgent identity / Session、Nagiの位置付け、Manager向けの指示・監督UI、および性能に応じた構成変更の条件。
- Task Log、Activity、Memory、ArtifactのRetention / Redaction / Promotion、API / schema versioning、migration guarantee、互換性support期間。
- Rust / C / C++ migration優先対象、toolchain、diagnostic、公開API mapping、アプリごとの互換性。
- 1.0のpublic launch日程、developer / distributionの詳細、Storeの役割、公開範囲、Acceptance、support policy。

## 例外・禁止・安全境界

- 本DECは0.1 M30、0.2 M21と各既存契約、0.3 Implementation Specification / activation gate、0.4 Entry Gate / Integration Owner checkpointを変更しない。Host-only PASSをTarget Runtime PASSへ読み替えない。
- 本DECやMaster RoadmapをRelease activation、実装開始、公開許可と解釈しない。0.5〜1.0の版別案は確定Release commitmentではない。
- DEC-013の正式Memory Engine実装を0.5以降へ延期しない。MemoryのHardware-Adaptive profile・performanceを測定前に保証しない。
- Agent、Role、Model、event detection、決定論的実行は、それだけでPermission / Capabilityを付与しない。Public Action / Service APIと現行Policyの境界を維持する。
- 個人Memory、Task artifact、Agent / Team / Organization dataを暗黙に共有しない。
- Private decision / roadmapをPublic repository、Public PR、Issue、CI logへ転載しない。

## 関連文書との位置付け

- [DEC-007](DEC-007-roadmap-alignment.md)はREVIEW NOTEのままとし、ユーザー決定を上書きしない。
- [改訂0.3 Roadmap](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)は0.3のArchitecture intentと既存Gateに加え、1.0の一般公開可能な品質・安定性・セキュリティ・互換性・開発者体験という高水準目標を示す。[DEC-013](DEC-013-lightweight-memory-engine-benchmark.md)は0.3 Hardware-Adaptive Memory Engineの方針・評価境界を規定する。
- [0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)は0.4+ Team構想とPhase A–FのVision proposalであり、初回0.4の確定境界は[DEC-015](DEC-015-builder-role-review.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-019](DEC-019-builder-work-data-cleanup.md)を参照する。
- [DEC-009](DEC-009-nagi-first-home-desktop.md)はHome / DesktopとTask再開の境界、[DEC-010](DEC-010-intent-first-adaptive-ui.md)はIntent-first UIと配置保護、[DEC-011](DEC-011-task-approval-progress-ui.md)は承認と進捗、[DEC-012](DEC-012-workspace-naming-organization.md)はWorkspace管理とユーザー選択による復元を定める。[DEC-014](DEC-014-workspace-state-checkpoints.md)は保存履歴・保護・検索・復元の詳細境界を保持する。
- [DEC-017](DEC-017-third-party-developer-ecosystem.md)の1.0開発者環境・配布基盤は製品目標として確定している。移植支援・AI-codingの方向も引き継ぐが、具体策・対応範囲・Acceptanceは未確定。
- [DEC-020](DEC-020-ai-os-interaction-path.md)の構造化Context / Event、typed Public Action、event-driven / deterministic-first、GUI継続の方針を保持する。性能向上は設計目標であり測定結果ではない。
- [Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)の版別範囲は提案であり、実装仕様と別途activationで確定する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-04 | DEC-017〜020を含む最新のProduct Decisionsと、0.3改訂・0.4+ Roadmapに整合する0.5〜1.0の方向案を記録。DEC-017で明示済みの1.0開発者環境・配布基盤目標を区別し、版別範囲・順序、Multi-user / Multi-device、Organization scope、数値・実装詳細は未確定として記録。 |
| 2026-11-16 | DEC-074の人間が管理する階層型AI組織を長期方向として追加。既存の0.4方式とユーザー窓口を保持し、組織構造・技術・版割当は未確定のままとした。 |
| 2026-11-16 | 後続確認：当面のAI組み込み体験型OSを優先し、階層型AI組織は非優先の長期ビジョンと明記。既存Gate・builder Roleと暫定の版別案を維持。 |
| 2026-11-17 | PX-13のB＋将来C共同制作は長期方向として採用し、ローカル完結型の高度統合制作環境を当面は低優先、1.0計画の最終盤を目安とする製品方向に整理。Cloud AIとローカルLLM／Nagi SDK・Porting Toolkitの安全な橋渡しを先に重視する。ただし1.0の特定版／搭載必須はPROVISIONAL、既存の公開開発者環境・配布基盤の1.0目標とRelease／Activation Gateは維持。 |
