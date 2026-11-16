# Nagi OS 0.4+ Multi-Agent Organization Roadmap

> **2026-11-15 公開移管:** [DEC-050](../DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

## Linux基盤への適用注記（2026-11-13）

[DEC-042](../DEC-042-linux-kernel-transition.md)のLinux kernel選択、[DEC-043](../DEC-043-nagi-userland-independent-os.md)のNagi専用userland／既存distribution基盤非採用／個別OSSの選択的再利用はCONFIRMED。[DEC-044](../DEC-044-linux-migration-development-authorization.md)と[移行ADR](../../decisions/2026-11-12-linux-migration-development-scope.md)はPrivateでの対象限定の移行設計・実装・検証を承認済みである。これは製品の恒久構成・正式切替やrelease activationと別の承認であり、具体的な実開始・統合成功は対象refと証拠で確認する。

割当済みの移行開発では、本文に残る旧独自kernel完成・旧M30順序を一律の着手前提としない。既存release Gate・Workstream owner/path・安全境界・SSD/Git等の個別制限は維持する。具体的Linux構成・互換方式・package・適用版・切替日を本改訂で確定しない。開発用の可逆的選択はDEC-044の委任内で記録・検証でき、全体設計の完成を新たな一律開始Gateにしない。

[仕様・実装影響表](../KERNEL-TRANSITION-ALIGNMENT-2026-11-13.md)と[Private 0.2補足](../../Nagi_OS_0.2_Linux_Migration_Addendum.md)を併読する。Private内容の公開・配布は別承認を要する。

§2.1 Entry Gate、§12初回Role workflow受入、後続専門Agentの別Gateを維持する。初期単一Nagiのbuilder Role、委譲狭窄、Stop、監査・証拠境界は変えない。移行開発の並列作業体制は製品Multi-Agent runtimeのactivationではない。


**Project:** NAGI — Native Agentic General Interface  
**Document Type:** 将来アーキテクチャRoadmap提案  
**Target:** 0.3 Personal AgentのAcceptanceと明示的なrelease activation後の0.4以降  
**Status:** Vision proposal / Implementation Specificationではない  
**Purpose:** 単一Personal AgentからローカルAgent Team、将来的なAI組織へ安全に拡張する道筋を定義する  
**Companion:** `Nagi_OS_0.3_Architecture_Roadmap_Revised.md`

> 本提案は0.1、0.2、0.3のrelease gateを変更しない。0.3のAcceptanceと別途0.4を有効化する判断の前に、実装を開始する許可でもない。

---

## 1. Vision and release boundary

**2026-11-16 後続確認（[DEC-074](../DEC-074-human-led-agent-organization.md)）:** 当面はAIがOS標準機能として自然に働くAI組み込み体験型OSの確立を優先し、階層型Agent Organizationは非優先の長期ビジョンとする。以下の組織拡張・Phase案は具体的な提供版や実装順序を確定しない。初期0.4の単一Nagiによるbuilder Role、§2.1 Entry Gate、§12 Acceptanceは変更しない。

ユーザーの主な窓口は、会話する秘書Agent **Nagi** ひとつとする。Nagiが依頼を理解して計画を説明し、Policyに応じて承認を求め、初期はNagi自身が必要なRoleで範囲限定Taskを実行する。将来は専門AgentにもTaskを委譲できるようにし、成果の証拠を確認してユーザーへ統合した結果を返す。

ローカル端末の能力に応じて、次のような専門Agentを追加できる。

- **プログラミングRole `builder`（初期はNagi自身が切替実行）:** 許可されたWorkspace内でコードを変更し、Build / Testを実行して確認可能な成果物を用意する。
- **Research Agent:** 出典と根拠を残して情報を収集・要約する。
- **Review Agent:** 提案、差分、証拠、Acceptance条件を確認する。
- **Verifier Agent:** 決定的な基準に照らして成果物を独立に検証する。

Nagiはユーザーに対して一貫した窓口と調整役であり続ける。専門Roleは権限を生まない。Permission、Capability、Action実行、取消、永続状態の決定権はOSにある。

**人間がManagerとなる長期的な組織方向（DEC-074）:** 長期的には、ユーザー1人が唯一の人間Managerとなり、複数の統括AIがそれぞれの専門Worker Agentをまとめる階層型の組織を目指す。端末資源に応じて組織の規模や階層を拡張できる方向を確認する。Nagiを一貫したユーザー窓口とする既存方針は維持するが、Nagiが統括AIの一員か別の調整役か、Supervisorや階層の数、実行・資源管理の仕組み、Version割当ては未確定。本方向は将来の製品構想であり、0.4初期の単一Nagi builder Role方式、0.4 Entry Gate、既存Permissionを変更しない。

段階的な規模拡張を想定する。

```text
0.3: 単一Personal Agentと正規のクロスアプリAction
0.4: 1ユーザー・1端末、Nagiが専門Roleを切り替えて実行する限定Workflow
0.5以降: ローカル組織の拡張、共有Workflow、必要に応じた複数端末連携
将来: 人間がManagerとなる階層型AI組織。GovernanceとSecurityの証拠が整った範囲で拡張
```

0.3以降のVersion番号は計画上の案であり、正式なrelease決定に従う。最初の0.4では規模より小さく有用なWorkflowを成立させる。

**初期`builder`実行方式Cの確定（DEC-015／DEC-024）:** 外部TypeSafe AIのJevを開発Agent名に誤用した旧案は撤回する。プログラミングRole名は`builder`（小文字）とし、**初期はNagi自身がRoleを切り替えて実行**する。Task状態、作業領域、権限Scope、検証・復旧情報はTask単位で分離する。初期から独立したBuild Agentを二体目として必須にする案は採用せず、将来は同じRoleを独立専門Agentにも割り当てられる構成とする。Role切替は権限を生まず、別PrincipalやAgent間委譲を初期に存在すると見なさない。Public Action、Stop／Activity、0.3 Acceptance、0.4 Entry Gate、別途Integration Owner checkpointを変更しない。高速な判断モデルは[ADR-0018](../../decisions/0018-decision-and-generative-model-architecture.md)のDecisionProvider境界で扱い、外部Jevを要件としない。詳細は[DEC-015](../DEC-015-builder-role-review.md)・[DEC-024](../DEC-024-external-jev-reference-and-builder-label-correction.md)。

**`builder` Role切替トリガーと表示の選択C（DEC-015／DEC-011）:** NagiはIntentに応じて開発作業の必要性を判断し、初期は自ら`builder` Roleへ自動切替する。ユーザーには「builderで作業中」などRoleとTaskの進捗が分かる状態を示す。手動で`builder`を指定でき、通常Nagi対話へ戻ることもできる。通常対話への復帰は進行Taskの自動取消・再実行ではない。Role切替自体は新たな権限やAction承認を生まず、DEC-011の既存承認UIとDEC-016／018のScope・Permissionを維持する。具体的な表示・操作UI、判定閾値は未確定。0.4 Entry Gateを変更せず実装開始も許可しない。

**別会話中の`builder` Task継続 — 選択C（DEC-015／020／011／022）:** Nagiはユーザーからの新しい会話を前景優先で処理し、承認済み`builder` Taskの状態・作業領域・権限Scope・証拠を維持する。利用可能なResource Budgetと既存OSの安全契約内で、非LLMのBuild／Test等はバックグラウンドで継続でき、推論や競合する処理はQueue／Throttle／資源待機とする。資源待機のみのTaskは現行Scope・Permission・期限・Budget・Revision・Cancel状態の検査を経て続行できる。明示Pause／Cancel／Stop、未承認・承認待ち・権限失効・状態不明、クラッシュ／再起動後の復旧待ちでは自動再開・副作用再実行を認めない。単一LLMの逐次推論を初期構成の基本とし、別Agentや複数Modelの並列は後続PhaseとGateに従う。Task UIでは原因別の待機・進捗を正しく表示し、0.4のEntry Gateを変更しない。

**`builder`作業中の追加指示・別件開発依頼 — 選択C（DEC-015／016／011）:** Nagiは追加指示を（1）元Taskと同一目的・承認Scope内の軽微な変更、（2）独立した開発Task、（3）大幅変更・Resource競合・優先順位変更／曖昧さとして分類する。（1）はTaskの作業計画・検証を更新して扱い、古い成果・Testを必要に応じて再検証する。（2）は独立したTask／承認／Context／Workspaceとして登録し、初期はQueueによる逐次実行を基本とする。（3）は安全な境界で確認待ちとし、ユーザーへ影響・選択肢・必要な追加承認を提示する。新指示だけでTaskを取消・無断再実行せず、DEC-018のResource Revision確認、隔離差分・原本保護、DEC-020の前景優先と既存のPermission／Policyを守る。新Taskへ元Taskの権限を移さない。分類閾値、Queue／Priority UI、状態遷移詳細はPROVISIONAL。0.4のEntry Gate、Activation／並列制限は変えない。

**Nagiが使う`builder` Roleの動作要件（DEC-016）:** 初期自律実行はB（承認Scope内で調査・修正・ビルド／テスト・再修正まで実行、Git commit／pushは既定では対象外）。Nagiが計画と最初の承認範囲を管理し、ユーザーは対象Workspace、対象Resource、許可Action、Resource権限を確認して承認する。Nagiは差分、テスト証拠、部分失敗、残課題を報告し、builderの権限は最小限の承認範囲とOSの現行Permission / Policyに限る。成果の最終確認はC（通常の検証済み修正をNagiが自動完了判定し、重要変更・テスト失敗・検証不能・Scope外はユーザー確認に回す）。これは完了判定方式であって権限拡大や0.4実装開始許可ではない。詳細は[DEC-016](../DEC-016-builder-autonomy-risk-based-completion.md)を正本とする。


**Build / Execute担当の修正反映要件（DEC-018）:** 初回0.4ではbuilderは隔離した作業領域でコード修正・テストを行い、Nagiが差分・検証証拠・権限・競合・元ファイルの保護を確認する。承認Scope内で条件を満たした通常の修正は正規Public Actionで自動反映できる。重要変更、検証不能、競合、保護失敗、Scope外はユーザー確認に回す。Git commit／pushや本番反映の権限を自動的に付与せず、0.4 Entry Gateは変えない。詳細は[DEC-018](../DEC-018-builder-isolated-edit-conditional-apply.md)に従う。


**`builder`製アプリのNagi Action API提案・生成方式C（DEC-040）:** `builder`はアプリの用途から自然言語経由のNagi操作に適した型付きPublic Action／Service API候補を自動提案し、必要なものを承認された開発Task／Action Scope内で実装・検証する。通常のGUIを残し、AI連携を必要としない単純なアプリへのAction実装を強制しない。APIを作成・発見してもPermission／Capability／Task実行承認は生じず、呼出しは現行Public Action／IPC／Provider、Resource Revision、Activity、Stop／Cancel、DEC-027の限定GUI fallbackに従う。0.2 SDK／統合レベル0〜3、0.3 Acceptance、0.4 Entry Gateを変更しない。Action Schema／提案判定／生成・テスト方法／実装版はPROVISIONALで初期0.4の必須完了項目ではない。正本は[DEC-040](../DEC-040-builder-generated-action-api-integration.md)。

**builderが作成した新規アプリの試用・初回正式導入 — 選択C（DEC-036、2026-11-05 CONFIRMED）:** 未導入のアプリは、安全に対応できる環境で試用し、Nagiが機能、確認できた検証証拠と未検証事項、要求Permissionを説明する。ユーザーが採用を選んだ場合に限り、正規Package検証、OS Permission／Policy確認、初回導入への別途の明示承認を経て正式インストール・永続登録する。試用成功、Task完了、採用判断は導入承認を兼ねない。試用方式と版別割当てはPROVISIONALであり、0.4 Entry Gate／Integration Owner checkpoint／Activation／Workstream開始を変更せず、新たな0.4完了要件にもならない。正本は[DEC-036](../DEC-036-builder-app-trial-and-installation.md)。

**`builder`製アプリの導入後保守・更新方式C（DEC-035）:** Nagiは許可された診断情報やユーザーの報告から不具合修正を提案できる。必要な承認済みTaskの範囲で`builder`が隔離環境で修正・Build／Testし、Nagiが機能・差分・証拠・権限・データ移行と復旧可能性を説明する。**正式更新・インストール済みVersionの変更はユーザーの明示確認・承認後に限り**、正規app package・manifest・Permission／Policyを再検証する。現行データ・設定の保護と、失敗時に実在する復旧可能範囲の明示を維持する。DEC-018の自動ソース反映は正式アプリ更新を認可しない。更新方式の技術詳細・対応版はPROVISIONALで、0.2 app platform契約、0.3 Acceptance、0.4 Entry Gateを変えず、初期0.4の完了要件を新設しない。正本は[DEC-035](../DEC-035-builder-created-app-maintenance-updates.md)。

**Build / Execute担当の作業データ整理方針（DEC-019）:** 隔離作業領域・一時生成物は状態別に管理する。正常完了して安全な反映・検証と必要な証拠・復旧データの保持が確認できれば、不要なTask専用データを自動整理できる。失敗・確認待ち・中断など未完了の作業やユーザー保護成果物、元ファイル、共有キャッシュ、復旧用データは無断整理しない。具体的な保持期間と容量上限は未確定。既存のPermissionとGateを変えず、詳細は[DEC-019](../DEC-019-builder-work-data-cleanup.md)に従う。


**Build / Execute担当の中断復旧方針（DEC-022）:** 再起動・クラッシュなどによるbuilderの中断Taskは、Task Log／Activity／隔離成果等から実際に確認できる状態を保持・再構成し、Nagiが「続きから再開／最初からやり直す／中止」をユーザーへ提示する。ユーザーの選択前に副作用を自動再実行せず、選択後も現行Permission、Delegation、Budget、Resource Revision、実行済みActionを再検証して二重実行や上書きを防ぐ。Workspace復元によるTask再実行も認めない。詳細は[DEC-022](../DEC-022-builder-interrupted-task-recovery.md)に従い、Entry Gateを変更しない。

**`builder`製アプリの共有・公開方式C（DEC-036）:** アプリは個人利用を初期標準とし、ユーザーが希望した場合のみ内容・依存・個人情報／秘密情報・個人設定／Memory・ライセンス・Permission等を確認し、必要な分離・検証と明示承認後にソースまたはPackageを出力する。友人等への限定共有とGitHub／Store等の一般公開は別々に選ぶ。共有作成許可と実際の外部送信／公開許可を分離し、受取側で独立検証・Permission再承認を行う。元ユーザーのMemory／Permission／認証情報は移転しない。完全な秘密除去・自動ライセンス判定・Store実装は保証しない。技術詳細・版割当ては未確定であり、0.2-M21、0.3 Acceptance、0.4 Entry Gate／Integration Owner checkpointを変更せず、初期0.4の完了要件や実装許可を追加しない。正本は[DEC-036](../DEC-036-builder-app-sharing-and-publication.md)。

**builder製アプリの開発Project保存方針C（DEC-040、2026-11-05 CONFIRMED）:** builder製アプリの正式な開発Project（ソース、必要な設定・依存関係／再構築情報、Version履歴、改修・検証・復旧に必要な記録）をローカル基本で長期保持し、再発見・現行Scope内での再開と将来の選択式端末移行を目指す。正式ProjectをDEC-019のTask一時データ整理へ混ぜない。アプリ本体、Project、ユーザー実データ、Canonical Memoryは別管理し、DEC-014 Workspace snapshotはソースバックアップではない。保存はGit commit／push、Cloud backup、外部送信、共有を許可しない。形式・数値条件・Git・移行形式・適用版はPROVISIONAL。正本は[DEC-040](../DEC-040-builder-app-development-project-retention.md)。この将来方針は0.1／0.2／0.3 Acceptance、0.4 Entry Gate／Integration Owner checkpoint、Activation、Workstream開始を変えない。

**`builder`製アプリのアンインストール・関連データ削除方式C（DEC-041、2026-11-05 CONFIRMED）:** 通常のアンインストールではアプリ本体のみ削除し、ユーザーデータ、設定、保護ソースコード、DEC-040の開発Project／履歴を原則保持する。完全削除時は対象、既知の共有参照、影響、回復可否を示し、ユーザーが対象ごとに明示選択・承認したものだけを正規Permission／Policy／Public Action経由で削除する。他アプリ／Taskの共有データ、作業中成果、復旧用データ、他ユーザー所有・未確認データを巻き込まない。保存Artifact／Manifest、互換性等に依存するため再インストール・再ビルドは保証せず、古いPermissionを自動再付与しない。保持期間・UI・技術実装・版割当てはPROVISIONALであり、DEC-014／019／040および既存Gateを維持し、0.4 Entry Gate／Integration Owner checkpoint／Activation／Workstream開始や初回0.4完了要件を変更しない。正本は[DEC-041](../DEC-041-builder-app-uninstall-and-data-deletion.md)。

## 2. Entry gate and invariants

### 2.1 Entry gate

0.4 Implementation Specificationの作成を検討できるのは、少なくとも以下を満たした後とする。

1. 対象Runtime上で0.3 Personal AgentのAcceptance / Exit CriteriaがPASSしている。
2. Offline音声、Public Action、Permission、Activity、取消、復旧経路の再現可能な証拠がある。
3. Agent / Session / Taskの相関が安定し、Version管理されている。
4. Identity、IPC、Capability、Action、Job、Activity / Transaction、Checkpoint / Waybackの契約が利用可能である。
5. 委譲の狭窄、Stop伝播、監査、失敗ケースのThreat Modelをレビュー済みである。
6. 別途0.4 release境界とIntegration Owner checkpointを明示し、対象Workstreamを有効化する。

Gateは関連契約に対する証拠を求める。Host-only作業をTarget Runtime PASSに読み替えない。実装仕様が理由を示さない限り、無関係な0.2 Workstreamを新たな依存条件にしない。

### 2.2 Non-negotiable invariants

- **Model RuntimeとAgent Runtimeは非特権。** 推論、計画、要求は行うが、Permissionを発行せず公開経路を迂回しない。
- **副作用の正規経路はPublic Action。** Actionは型付き・Version管理され、Permission、Revision、side effect、可逆性を検査できる。
- **OSが呼出しごとに判定する。** 過去の承認を恒久的な包括権限にしない。実行時にProviderがCaller、Session、対象、Scope、Revision、Policyを検証する。
- **委譲は権限を狭める。** 子Agentの権限、対象、期間、Resource budgetは親から受けた範囲を超えない。
- **人間の監督を常時保つ。** 計画確認、承認、拒否、Pause、Cancel、Root Task全体の停止を可能にする。
- **Offline-firstを維持する。** Cloud ModelやServiceを任意Adapterとして追加する場合も、ローカル利用を意味あるものに保ち、外部転送には明示的な同意を求める。
- **Memoryは所有者とScopeを保つ。** 個人、Task、Agent、Team、Organizationのデータを暗黙に混ぜない。
- **失敗を正直に示す。** 部分実行、結果不明、Retry、Timeout、不可逆な副作用を全面成功やUndo可能として表示しない。

## 3. Conceptual object model

正確なSchema、Wire format、Storage engine、IDの形式はImplementation Specificationまで確定しない。以下は意味上の提案であり、確定契約ではない。

| Concept | 意味 | Security / lifecycle rule |
|---|---|---|
| `AgentId` | インストール済みまたはユーザー作成Agent定義の安定ID | 振る舞い・設定の識別子であり権限ではない |
| `AgentSessionId` | 一回の実行Session | User、Profile、Device、現行Policyに結び付ける |
| `RoleId` | Build、Research、Review等の作業説明 | Role名や指示文はCapabilityではない |
| `TaskId` | Root依頼または範囲を限定した作業単位 | Owner、状態、期限、Budget、依存関係、証拠を持つ |
| `DelegationId` | Agent / Task間の範囲付き委譲Grant | 期限・取消・監査可能で、権限を拡大しない |
| `OrgId` | Agent、Task、手順、共有データの名前空間 | 所属だけで全Organizationデータへアクセスさせない |
| `ActivityId` / `TransactionId` / `CheckpointId` | 副作用と復旧状態をTask treeへ結び付ける | 既存Activity / Waybackの意味に従う |

Agent定義はModel profileと複数のRoleを持てる。Taskには親Taskや依存関係を設定できる。Delegation recordには委譲者、委譲先、目的、Resource範囲、許可Action種別、期間、Budget、承認条件、取消状態を含める。

### 3.1 Task and delegation state

Task状態の候補:

```text
CREATED -> PLANNED -> WAITING_APPROVAL -> READY -> RUNNING
         -> WAITING_DEPENDENCY -> REVIEW_REQUIRED -> SUCCEEDED
         -> FAILED | PARTIAL | CANCELLED | TIMED_OUT | BLOCKED
```

Retryの後に部分成果が出ても、過去のAttemptを`SUCCEEDED`へ書き換えない。Attemptを別々に記録し、Idempotency keyで意図しない副作用の重複を防ぐ。

Delegation状態の候補:

```text
PROPOSED -> APPROVED -> ACTIVE -> COMPLETED
                       |-> REVOKED | EXPIRED | FAILED
```

親Taskは成果を取りまとめるが、OSの権限制御は親の自己申告に依存しない。

## 4. Bounded delegation and permissions

### 4.1 Effective authority

委譲Taskで子Agentが利用できる権限は、次の共通部分だけとする。

```text
親が現在持ち、委譲可能なCapability
∩ ユーザーが明示的に承認した委譲範囲
∩ Task / Resourceの対象範囲
∩ 許可Actionの種類
∩ 時間 / Session / Deviceの制限
∩ Resource budget
∩ 現行OS PolicyとProviderの検査
```

共通部分は子AgentがActionを呼ぶたびに評価し、再利用できるBearer permissionとしてコピーしない。親が持たない、または委譲不可のCapabilityを渡せない。再委譲は既定で禁止する。明示的に認める場合も範囲をさらに狭め、全系譜を記録する。

### 4.2 Approval policy

承認は具体的なAction計画または範囲を限定した反復可能Actionに対して行う。承認内容には以下を含める。

- Actionと型付き引数
- 対象Resource / Objectと期待Revision
- side effectと可逆性の分類
- 委譲先AgentとTask範囲
- 期間、Attempt数、Resource上限
- 子Agentへの再委譲を許すか
- 成果を受け入れるための証拠

承認によって管理者権限を与えない。Risk分類と事前承認上限は決定的・可視・設定可能・取消可能でなければならない。Risk taxonomyと自動承認上限は保留事項とする。

### 4.3 Supervision and stop

Nagiの操作画面で次を確認できるようにする。

- Root依頼と現在の計画
- 実行中・待機中の子Task
- Agent、Model、Role、Action、Resource、承認状態
- 有用な進行状況とResource利用量
- TaskごとのPause / Resume / CancelとRoot全体のStop-all
- 拒否、失敗、Retry、Handoffの理由

Stopは子孫Task、Background Job、推論、音声出力、待機中Actionへ伝播する。Providerは変更処理の直前にCancel状態を再検査する。外部副作用を取り消せない場合は、完了したことと結果不明なことをUIで区別する。

## 5. Agent roles and organizational hierarchy

Phase Cの**初期構成はNagi一体が`builder` Roleへ切り替えて限定Taskを実行する方式C**（DEC-015）とする。旧Nagi＋専用開発Agentの二体構成は初期必須要件から外す。Research / Review / Verifierや独立した専門Agentは後続Phaseの検討対象とし、初回構成に含めない。

```text
User
  |
  v
Nagi (Secretary / Coordinator; one Agent initially)
  |
  +-- builder (programming Role of Nagi; scoped Task)
  |
  +-- future: separate specialist Agent using builder Role (not initial)
```

Nagiが委譲するのは範囲付きのTaskと成果物であり、無制限の制御権ではない。例えばbuilderは承認済みWorkspace内でPatchとBuild証拠を作成し、ReviewはDiffと証拠を確認する。Nagiは証拠をユーザーへ提示し、保護対象や外部に影響するActionの前に必要な承認を求める。

Role定義に含める内容:

- 対応するTask種別と出力形式
- 信頼できる入力と未信頼入力の境界
- 要求可能なデータScope
- 提案可能なAction種別
- Resource / 実行時間Budget
- 必須の証拠と検証ルール
- 再委譲を許すか

OSはActionごとに検査する。Roleに秘密鍵、System Capability、Raw Kernel API、隠れたMemory accessを持たせない。同じRoleを持つAgentも別Principal / Sessionとする。

## 6. Model choice, sharing, and resource scheduling

### 6.1 単一のローカルModelを複数Roleで利用

一つのLocal ModelでNagi、builder、Research、Reviewを順番に、または制限付きで実行できる。Modelの重複を抑えResourceを予測しやすい一方、応答時間が延びる場合がある。同じModelとContext biasに依存するため、それだけでは独立検証にならない。

### 6.2 異種ローカルModelの併用

作業に応じて、日本語性能、Coding品質、Context長、応答時間、メモリ使用量などで異なるLocal Modelを選ぶ。Reviewは別Modelまたは決定的な検査を用いることができる。Model選択は品質とResource需要にだけ影響し、Permissionは変えない。

### 6.3 Runtime共有と分離

対応時はModel artifactを重複排除し、不変のweightを共有してよい。各推論SessionはContext、Cache、Cancel、Budgetを分離する。Workerに秘書Nagiの個人会話を既定で渡さない。ContextはPermissionで絞ったTask単位のEnvelopeを通す。

### 6.4 BudgetとScheduler

各AgentSession / Taskに次の上限を設ける。

- CPU / GPU時間
- RAM / VRAMとModelロード分
- Context長、Token数、推論時間
- 同時実行する子Task数、待機Action数
- Wall-clock期限、Retry数、Tool call数
- Storageと生成Artifact容量

SchedulerはForegroundの音声 / UI応答性、ユーザー優先度、Task urgency、端末能力、取得可能なThermal / Battery状態、Modelロードコストを考慮する。Queue、逐次化、Throttle、Checkpoint、Pause、Cancel、Unload、ユーザー選択を行える。負荷時はShell、Foreground App、音声継続性をBackground Agentより優先する。Budget超過を成功扱いにしない。

| 容量区分 | 候補となる挙動 | 維持する条件 |
|---|---|---|
| Compact | 小型Local ModelでAgentを一体稼働し、Role作業は逐次実行 | Offline利用を維持し、子Agent並列を要求しない |
| Standard | 一つのModelを順番に共有し、必要に応じて別Modelを利用 | Task / Agent Scopeを分離する |
| High capacity | 明示Budget内で複数Modelや複数Agentを並行 | Permission / Memoryの範囲を広げない |
| Organization scale | 別Gate後にTask、Team、Deviceを拡張 | 全端末・Actionで同じPolicyを適用 |

容量判定、ユーザーOverride、Benchmark目標は基準Hardwareを決めてから定義する。

## 7. Memory and data boundaries

| Scope | Owner | 例 | 既定アクセス |
|---|---|---|---|
| User / Profile | User | 個人的な事実、嗜好 | 許可済みのOwner Profile Context |
| Agent | Agent定義または実体 | Role固有の慣習 | Agent Scope。自動的にUserの事実にしない |
| Task | 単一Task tree | 作業メモ、計画、中間成果物 | 既定で一時的 |
| Team | 名前付き人間Team | 共有手順、Project情報 | 明示的なメンバーとGrant |
| Organization | `OrgId` | Policy、共有業務知識 | 明示的なGovernance、保持、削除条件 |

ContextをModelへ渡す前にPrincipal、Purpose、Task、Resource、Permissionで絞る。全会話履歴を各子Agentへ複製しない。委譲Context packetには含めたデータ、provenance、機微度、有効期限、許可用途を記録する。成果物は子Agentの全Memoryを公開せず、根拠付きArtifactとして親へ返せる。

Task / Agent MemoryをUser、Team、Organization Memoryへ昇格するには明示Policyとprovenanceを要求する。個人Memoryを自動でTeam情報にしない。Agent削除やメンバー変更時に、誰がArtifactを所有し、Export / Delete / Retainできるか明確にする。

MemoryはAPI経由で検索・更新し、AgentやAppからDBへ直接接続しない。Model cacheやweightを長期Memoryの正本にしない。

## 8. Task Log, Activity Ledger, and Wayback

相互に関連しつつ用途が異なる記録を分ける。

- **Task Log:** 再開性、依存関係、Budget、Retry、Error、証拠参照、取消のための運用状態。アクセス制御と保持期間を設定する。
- **Activity Ledger:** ユーザーが理解できるAction、Access判定、承認、provenance、結果。説明と監査に使う。

各Eventには利用可能な相関情報を持たせる。

```text
OrgId? / AgentId / AgentSessionId / RoleId / TaskId / ParentTaskId
DelegationId? / UserId / ProfileId / SessionId / AppId
ActionId / ResourceId / expected RevisionId / ActivityId / TransactionId
model provider + version / policy decision / approval / attempt / result state
```

Prompt全文を既定で永久保存しない。明示Policyに従い、型付き入力、秘匿した最小抜粋、Hash、保護された参照など必要最小限を保存する。

Waybackは対応するローカル変更のCheckpointと実在する逆操作を扱う。送信済みMessage、変更済みRemote serviceなど不可逆操作のRollbackを約束しない。部分Restoreも独立したActivityとして記録し、戻った範囲と戻らなかった範囲を示す。

## 9. Failure, conflict, verification, and recovery

### 9.1 有限RetryとIdempotency

Action AttemptごとにRequest / Idempotency identity、親Task、期限、Failure classを記録する。Retry回数を制限し、原因の証拠に合わせて試行内容を変える。結果不明の非冪等Actionは新たなPermissionと確認なしに再実行しない。

### 9.2 Resource / Document競合

同じResourceに対する並行変更は、Provider境界でRevision確認、Lock、または書込み逐次化を行う。古い結果で新しいUser変更を上書きしない。Nagiは差分、Merge案を見せるか、どちらを採用するか確認する。

### 9.3 Worker成果の検証

専門Agentの成果はprovenance付きの提案またはArtifactとする。Review / Verify Agentは有用な検査役だが、自己申告の承認を証拠にしない。AcceptanceにはTest出力、Schema検証、出典、Checksum、Diff review、Platform acceptanceなど再現可能な証拠を使う。検証失敗または未確定ならRoot Taskを`REVIEW_REQUIRED`、`PARTIAL`、`FAILED`のいずれかにする。

### 9.4 未信頼入力

Task入力、検索ファイル、Remote content、Model出力、Worker Artifactはデータとして扱う。それらがAgent Policyを変更したり、権限を付与したり、委譲範囲やユーザー指示を覆したりすることはできない。ユーザーが採用するまでは、ファイルやWorker成果内の指示を未信頼として扱う。

## 10. Offline voice and interaction

ユーザーへ話すAgentはNagiに統一する。専門Agentは進捗や成果物をNagiへ返し、同じ音声チャネルでユーザーへ独自に話しかけない。

```text
LISTENING -> TRANSCRIBING -> ACKNOWLEDGED -> PLANNING -> WAITING_APPROVAL
          -> DELEGATING / WORKING -> REVIEWING -> RESPONDING -> COMPLETE
```

短い受付応答、長いTaskの可視進捗、Local TTSが使える場合の音声要約を提供する。ユーザーは音声を割り込み、Taskを停止し、Textへ切り替えられる。Audio capture、ASR、TTS、Modelの状態を別々に扱う。どれかが使えない場合、Nagiは理由を説明して利用可能なローカル経路を案内する。Cloud音声やModelは明示Opt-inとデータ開示を要する。

常時収音、Wake Word、録音音声保持、話者識別、Latency SLOは別途Privacy / Product判断とする。

## 11. Phased implementation proposal

Accepted implementation specificationに記載されたrelease activationと依存条件を満たした後に限り、Phaseを開始する。

### Phase A — Organization architecture and threat model

- Principal、所有、ID、Task、委譲、Memory scopeの意味を確定する。
- Prompt injection、権限昇格、Confused deputy、秘密情報、取消Race、古いRevisionをThreat model化する。
- 承認表示、Stop伝播、監査情報、保持期間を定義する。
- 基準となるローカルHardware profileと小さな決定的Workflowを選ぶ。

**Exit evidence:** Architecture decision、Capability attenuation model、misuse case、互換性計画、Risk分類レビューが完了。

### Phase B — Registry, Task state, and resource accounting

- Ownerと有効状態を持つAgent / Role Registry。
- AgentSession / Task lifecycle API。
- Budget、期限、Cancellation token、Idempotency、Scheduler queue。
- Activity / Task Log相関、migration、復旧fixture。

**Exit evidence:** 不正、期限切れ、Budget超過、取消済み、重複、Owner違いのケースがfail closed。Stopが待機中と実行中の作業へ伝わる。

### Phase C — 1端末でのNagi単一Agent／`builder` Role切替

- 初期はNagi Coordinatorが同じAgentとして`builder` Roleを有効にし、承認された限定Taskを逐次実行する（DEC-015）。独立した子Agent／Agent間委譲は初期必須にしない。
- `builder` TaskはRoot Taskから識別・追跡でき、Task Scope／Permission／期限／Budget／作業領域／証拠を分離する。既存のTaskとActionの安全境界を維持する。
- まず逐次実行し、監査と取消がPASSするまで並列を有効にしない。将来の専門Agent化でのみAgent間Delegationと別Principal／Sessionの受入条件を追加検証する。
- Public Action呼出しは既存ProviderとPolicyを通す。

**Exit evidence:** NagiのRole切替でも範囲外Resource / Actionを利用できない。取消・期限切れ後の次Actionを拒否する。Task・変更・検証結果のprovenanceを追跡でき、ActivityでRoleとTaskの系譜を再構成できる。別Agentを有効にする際は従来の委譲・Stop伝播・監査・Threat ModelのGateも満たす。

### Phase D — Research / Build / Review workflow

- Research、Review / Verify Roleを追加する。
- Worker成果を決定的な証拠と照合して構造化Reportを返す。
- 競合、部分失敗、有限Retry、Action中の停止、Provider再起動、対応可能なWaybackを確認する。

**Exit evidence:** ローカルな一連WorkflowがOfflineで完了する。Agentの成功申告だけではAcceptanceしない。競合時に新しいUser dataを上書きしない。

### Phase E — 並列実行と異種Model Policy

- Hardware profileに応じた上限付き並列実行。
- 単一ModelのRole切替と異種Local Model routingを比較する。
- Scheduler公平性、Weight共有時の分離、Context budget、負荷時の挙動を追加する。

**Exit evidence:** 計測値が承認Budget内に収まる。Agent間でCache / Contextが漏れない。負荷時もForeground audio / UIを利用できる。

### Phase F — Team / Organization scope

- OrgId、Team所有、メンバー、Offboarding、保持、共有手順、監査レビューを検討する。
- ローカル単一Userの不変条件がPASSした後にMulti-user / Multi-deviceを加える。

**Exit evidence:** 承認したGovernanceの下でMembership取消、Export、削除、競合、Backup、Restoreが動作する。このPhaseは後続Versionへ送ってよい。

## 12. Acceptance criteria for the initial 0.4 Role workflow

適用される条件をすべてTarget Runtimeで証明した場合に、初回0.4のNagi／`builder` Role workflowを受け入れる。独立専門AgentとAgent間Delegationに固有の条件は、その機能の後続Activationで追加確認する。

### Authority and delegation

- Modelが要求しても、Nagiが`builder` Roleを使うTaskは承認Scope外のActionやResourceを利用できない。将来の子Agent委譲にも同じ最小権限原則を適用する。
- 初期Role切替はAgent間の再委譲ではない。将来の子Agentは既定で再委譲不可とし、明示許可した場合もScopeと系譜を検証する。
- Permission拒否、古いRevision、期限切れSession、不正Request、重複Request、取消済Grant、Unavailable providerはfail closed。
- Role、Model、Agent identity、Org membershipでPrivilegeを与えない。
- Pause / Cancel / Stopが子孫、Inference、Jobs、音声、待機Actionへ届く。

### Task execution and evidence

- NagiがRoot plan、`builder` RoleのTask、状態、承認要否、最終証拠を示す。
- Workerの完了に構造化結果とSource / Artifact provenanceがある。
- Retryは有限で、Idempotencyにより副作用の重複を防ぐ。
- 部分成果と不可逆な外部副作用を正しく区別する。
- 古い結果をReviewまたは拒否し、現在のStateを上書きしない。

### Memory and audit

- Context filteringによって、私的なUser / Profile Memoryを許可のないTeam / Agent / Task Contextへ含めない。
- 明示PolicyなしにTask Artifactを永続共有Memoryへ昇格しない。
- 再起動後もLedgerからActor、Role、Task、Delegation、Permission判断、承認、Action結果、Checkpointを再構成できる。
- Task Logの期限切れがActivity Ledgerを壊さず、削除済み内容が取得可能と誤認させない。

### Runtime and hardware

- Compact対応profileで単一Modelの逐次Modeが動く。
- 並列 / 異種Modelは測定Budgetを満たしたprofileでのみ有効になる。
- 負荷時にForeground UIと音声が応答し、Background作業をPauseするかBudget超過を報告する。
- Model変更で権限、Profile Memory、委譲範囲が変わらない。

### Offline voice

- NetworkがなくてもRootとの対話と進捗確認を継続できる。
- Local ASR / TTS障害時にText fallbackが見え、Task stateを失わない。
- 初期`builder` Role実行がユーザーの音声チャネルを占有しない。将来の専門Agentでも同じ条件を検証する。

## 13. Reference acceptance workflow

次の例はPhase D以降にResearch / Reviewを追加する場合の将来Workflow案であり、Phase Cの初回0.4構成を規定しない。Phase CではNagiが`builder` Roleで作った成果のprovenanceと再現可能な検証証拠を確認する。

```text
1. ユーザーがOfflineでローカルProjectの確認・改善をNagiに頼む。
2. NagiがRoot Taskを作り、builderがPatchを準備しReviewが確認すると説明する。
3. ユーザーは対象Workspace、Action、期限、Resource上限を承認する。
4. builderには対象範囲のTask Contextと承認済Capabilityだけを渡す。
5. builderはPublic Actionを使ってPatchとBuild証拠を準備する。
6. ReviewはDiffと証拠を独立に検査するが、新しいPatchは適用できない。
7. Permission拒否Actionは記録し、権限を広げてRetryしない。
8. ソースRevisionが変わった場合、古いPatchをReviewへ留める。
9. ユーザーがTaskを停止し、待機処理を取消し、完了済み副作用を列挙する。
10. NagiがLedger eventとCheckpointを添えて結果を報告し、Undo可能と誤って主張しない。
11. ユーザーは対象のTask ArtifactとMemory昇格を確認・削除できる。
```

## 14. Decisions deliberately left open

0.4 Implementation Specificationで証拠とPolicy reviewに基づいて決める。

1. 正確なVersion番号、Organization機能が複数Versionにまたがるか。
2. ID形式、発行者、永続化、migration。
3. Role package形式と第三者Agentの導入可否。
4. 既定 / 最大委譲階層、期限、再委譲、許可Action種別。
5. Risk分類、必須承認、事前承認の上限。
6. Local ASR / TTS provider、言語、Wake Word、録音音声の保持、割込み挙動。
7. Hardware区分、Model一覧、並列性、公平性、数値Budget。
8. Coding、Research、Review向けModel profileと品質測定。
9. Context packet、Memory検索、機微情報の秘匿、出力昇格Policy。
10. Task Log保持、Export、redaction、削除、ユーザー向けActivityとの関係。
11. 共有File、Team手順、組織編集の競合解決。
12. OrgIdのOwner、Admin、Member、Guest、Offboarding、復旧。
13. Multi-device / Remote Agentの範囲と端末外データ転送の同意。
14. 未対応または不可逆なActionのうち人間専用にするもの。

## 15. 0.2 / 0.3文書との関係

- 0.2 Codex Implementation Specificationは0.2 Platform Foundation、Milestone順、Activation、Workstream状態、Target PASS証拠を引き続き規定する。
- 改訂0.3 Architecture RoadmapはGate付きPersonal Agent世代、Profile / Memory / Context / Learning / Model portability、Public Actionを使う単一Nagi Agentを規定する。
- 本文書は将来Architecture proposalであり、既存仕様を上書きせず、候補Schemaを実装契約に変えない。
- 0.2の`AGENT` Activity actorやAgent Ready SDK levelはPlatform準備であり、Multi-Agent対応の証拠ではない。
- 0.3 Team Profile / Shared Memoryはデータ共有とPermissionの境界である。Agentの自己組織化やTeam権限継承を意味しない。
- Modelが利用不能または信頼できない場合もOSが権限判断を担い、AI推論はOS基本利用の必須条件にならない。

## 16. ファイル保存とChatGPT Memory

本Markdownはプロジェクト成果物として保存したファイルである。ファイル保存はChatGPTの長期Memoryへの保存とは別機能・別操作であり、この作業ではChatGPT Memoryを更新しない。

---

**End of Proposal**

## PDF Reader / Editorの製品方針（2026-11-16）

[DEC-061](../DEC-061-first-party-pdf-reader-editor.md)で、Nagi純正PDF Reader兼Editorの採用と段階的拡張をCONFIRMEDとして記録した。高速表示・検索・ページ操作・注釈・印刷・フォーム入力などを先に整え、その後、既存本文／画像の高度編集・OCR・元内容の削除と検証を伴う墨消し・署名関連機能を段階的に検討する。通常閲覧・編集はAI非必須とし、Nagi Project / Smart Paste / Writer / Sheets / Slides等と共通Public APIで連携する。AIによる表抽出・要約・比較・引用は利用者の承認と元資料の権限・機密分類の範囲に限る。

具体的なリリース版・高度機能の順序・技術・合格値はPROVISIONAL。本ロードマップの対象版への割当て、Acrobat完全互換、全機能初期実装、実装済み・Acceptance通過を意味せず、既存Gateは変更しない。
