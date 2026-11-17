# 共同開発者の役割・権限・担当領域割当案・実行バックログ

**更新日:** 2026-11-15  
**状態:** PROVISIONAL（具体的な担当、workstream、branch/path、着手、GitHub権限は未確定）  
**対象:** Core Developer候補がNagi Developer PlatformのPT-02〜PT-06を主導する案  
**PT-01:** 既存担当を維持し、本案の割当対象に含めない

> **2026-11-15 追記:** 創設者がNDP-01〜12をCore Developerの担当範囲として確定した（[DEC-046](../product-decisions/DEC-046-co-developer-governance.md)）。NDP-01は完了し、NDP-10は[開始記録](co-developer-starter-kit/START_AUTHORIZATION.md)の条件で開始する。Integration Owner、NDP-10以外のpath・branch、並行数、GitHub権限など本書のその他の値はPROVISIONALのまま。

> この文書は提案と実行候補の整理です。保存だけでタスクの割当、実装開始、workstream登録、branch/worktree作成、検証環境の利用、GitHub招待・権限付与、release gate変更を行ったことにはなりません。個人名・個人情報は記載しません。

## 1. 目的と進め方

友人をCore Developerとして迎え、割り当てた領域の設計・実装を日々の承認待ちなしに自主的に進めてもらうことを目指します。担当案は、競合しにくいNagi Developer PlatformのPT-02〜PT-06です。PT-01の既存担当と既存のSDK、Package、CLI、Linux移行の各担当領域を維持します。

PT-02〜PT-06を一つの大きなタスクとして渡すのではなく、12件の短い実行候補に分けます。契約確認と作業場所の整理を先に行い、入力・出力・担当pathが独立した3〜4件までを初期並行候補とします。実際の並行数と開始順は、担当者、既存owner、環境、branch/path、受入範囲を確認してから決めます。

本書のNDP-01〜NDP-12はこの提案専用の仮IDであり、既存のMilestoneや`.dev/workstreams.json`のworkstream IDではありません。

## 2. 確定していること／提案のままのこと

### CONFIRMED

- Founder（創設者）はNagi全体の方向性と最終判断を担います。
- Core Developer（共同開発者）は、割り当てられた担当領域の設計・実装を主体的に進めます。担当内の日常的な判断に毎回Founderの事前承認は要りません。
- 製品全体、担当領域をまたぐ仕様、共通契約、セキュリティ、release gateに影響する判断は、Founderと関係ownerの範囲で扱います。
- Porting ToolkitはNagi Native App向けの移植支援です。Windows／Linuxを対象にし、既存バイナリやGUI Frameworkの無変更実行を約束しません。AIなしの決定的処理を基本にし、LLMは任意の補助にします。
- PT-02は既存の公開SDK、Manifest、Package契約に依存する薄い雛形生成層です。独自SDKやManifest/Package規格を作りません。
- PT-05は既存Nagi CLI／SDK／Package Toolを使い、host検証とNagi target受入を別段階にします。
- 人間の共同開発体制は、0.3 Team Profileや0.4+ Nagi内のMulti-Agent機能を有効化する決定ではありません。

### PROVISIONAL／未決定

- Core DeveloperにPT-02〜PT-06全体を任せる案、12タスク、優先順位、3〜4件並行の初期案。
- 個別task owner、Integration Owner、reviewer、workstream登録、branch、worktree、変更可能なpath、統合先、実行環境、資源上限、着手時期。
- GitHubの個別アカウント権限、直接writeアクセスの要否。
- PT-02が外部アプリから既存SDKを参照する正規の配送・version pin方法、PT-03の対象API、PT-04が解析するUI形式、PT-06の実アプリ・測定指標。
- PT-01の公開側の最終的な実装経路。PT-01記録と未マージPR #91に、重複しうる読み取り専用診断の別案があります（第6節）。

本書に掲載したタスクはいずれも **PROPOSED / NOT STARTED** です。現在のownerによる引受け、path予約、必要な契約の確認、環境確認、Founderによる具体的な割当が済むまで、実装タスクとして開始しません。

## 3. 役割・判断境界

| 役割 | 裁量と責任 |
|---|---|
| Founder | 製品全体の方向、担当範囲、優先順位、release gate、権限、資源、最終的な統合方針を決める。担当領域を越える判断を行う。 |
| Core Developer | 割り当てられたPT領域内で技術案・分解・実装順を自主決定し、担当内の作業を進める。日常の細部ごとにFounderの承認を待たない。新しい製品仕様、所有範囲、共有契約、権限、release gateを自分で変更しない。 |
| Integration Owner | 現行workstream registryと関係ownerを確認し、共有path、契約接続、統合先、shared-file変更、PRのbase/merge boundaryを管理する。現時点ではDeveloper Platform担当のIntegration Ownerは未確定です。 |
| 既存Owner／Reviewer | SDK、Package、CLI、PT-01、UI等の既存契約・pathへの影響を確認する。関連pathを変更する場合は、そのownerとIntegration Ownerが定めた手順に従う。 |

### Core Developerに委ねる候補

- 担当内の技術調査、issueの分割、実装案と検証案、日々の実装判断。
- 許可されたbranch/path内の編集、担当PRの作成と更新、割り当てられたhost-only確認。
- 同じ担当内の小タスク順序の調整。Founderへの報告は、方針判断・共有契約・競合・blockerが生じたときに絞る。

### 委譲しない事項

- 製品方向やCONFIRMEDな契約、API/IDL/Manifest/Package規格、Security/Permission/Capabilityの意味を変えること。
- 他workstreamのpathやstate、共有Registry、Cargo workspace/lock、CI、生成binding、SDK、Package validator、mainや他branchを独断で変更・統合すること。
- PT-01の引受け・pathを変更すること、Linux移行laneの作業場所やDEC-047の許可を別laneへ転用すること。
- Nagi target acceptance、release activation、署名・配布、branch保護・GitHub権限を承認すること。
- 未許諾コードの取得・実行、秘密情報の扱い、ソースの外部AI送信、課金資源や新しいclone/worktree/環境の利用を独断で決めること。

## 4. GitHub・AI利用の権限案

### GitHub

1. 公開repositoryのreadと自分のforkからのPRを初期候補にします。既存credentialやアカウントを共有しません。
2. 直接branchへpushする必要があるとFounderが判断した場合のみ、本人確認後に別途最小限のWrite権限を検討します。Admin/Maintain、branch protection bypass、mainへの直接pushは付与候補にしません。
3. GitHub招待、権限付与、repository setting変更は、この文書では実施していません。必要な権限は別判断です。

### AI

- AIは開発補助であり、Core Developerの判断やレビュー責任を置き換えません。AIにGitHub資格情報や別人のsessionを共有しません。
- 決定的な解析・生成・検証を優先し、AI利用は任意です。外部AIへソースやログを渡す場合は、対象と送信先を開発者が選び、既存のPrivacy／Permission／Policyと必要な承認に従います。
- AIは担当範囲、既存path、契約、branch/PR、release gateを拡張する権限を持ちません。実行していないコマンドや結果を報告させません。

## 5. 作業場所、path、共有ファイル

### 着手前に確定する項目

各有効タスクは、現在のregistryと関連Stateを再確認し、最低限、owner、branch、許可・禁止path、依存契約、検証範囲、Integration Owner、PR baseとmerge boundaryを記録します。並行タスクは互いに異なる書込みpathと独立したfixtureを持つことを条件とします。

DEC-047の`codex/linux-integration-20261114`とLinux移行用worktree/操作許可は、そのLinux移行laneに限定します。Developer Platformのbranch、作業場所、統合先には自動適用しません。今回の文書作成branchも将来の実装branchではありません。

### 既存pathと担当の保護

2026-11-15の確認時点で、以下は変更対象外または要owner調整です。

- PT-01記録は`tools/nagi-porting/**`を候補専有範囲としており、Integration Ownerとの最終調整と実装開始条件が未完です。Core DeveloperはこのpathもPT-01の責務も引き継ぎません。
- 公開mainの`.dev/workstreams.json`では`developer-tooling` ownerが未割当です。一方、open draft PR #91のbranchではCodex Integration Ownerのhost-only checkpointとして`tools/nagi-cli/**`、`samples/porting/portable-files/**`、`docs/porting/app-migration/**`、関連State/Registryを変更しています。これはmainへ未統合です。
- `app-sdk-contract`は`sdk/**`と`tools/nagi-pkg/**`を持つ既存ownerがあります。外部向けSDK配布経路は未確認で、公開済みのCrates.io packageやGit依存を想定してはいけません。
- PR #57が`docs/porting/multiarch/**`を変更中です。ここをPorting Toolkit側で編集しません。
- `.dev/workstreams.json`、`.dev/schemas/**`、`.github/workflows/**`、共有Cargo workspace/lock、IDL/ABI、生成bindingはIntegration Ownerまたは契約ownerと調整する共有領域です。

### 候補path

Core Developer専用の実装pathはまだ割り当てられていません。候補を検討するときは、PT-01、#91、SDK/Package、multiarch、既存0.1/0.2 ownerのpathと競合しない新規領域を選び、Integration Ownerが現物・registryを確認してから登録します。たとえば専用Toolkit実装／test fixture／文書の新規pathは候補になり得ますが、名前を作るだけで書込み許可にはなりません。上記path、main、他workstream branchへの変更は含みません。

## 6. 2026-11-15時点の公開workstream／PR確認

この表は担当案を作るためのsnapshotです。作業開始時にはGitHubの最新状態とregistryを再確認します。

| PR | 確認時の状態 | 本案への関係 |
|---|---|---|
| [#101](https://github.com/Roy-To/NagiOS/pull/101) | merged。DEC-045〜050、roadmap、onboarding等がmainに入った。 | 本提案は#101へ追加していません。 |
| [#91](https://github.com/Roy-To/NagiOS/pull/91) | open / draft。head `0bd3af4`。 | 読み取り専用readiness scannerとhost sampleが`tools/nagi-cli/**`等にあり、PT-01/PT-03の診断面と機能上重なります。PT-03の独自scanner実装より前に正規PT-01入力・report interfaceを合意します。 |
| [#42](https://github.com/Roy-To/NagiOS/pull/42) | open / draft。 | Files、複数workstream、shared Registry/CI等に広く触れます。共有pathとFilesの所有に干渉しません。 |
| [#43](https://github.com/Roy-To/NagiOS/pull/43) | open / draft。 | Session/Model/Kernel等の統合中です。これらのAPIや共有領域を変更しません。 |
| [#48](https://github.com/Roy-To/NagiOS/pull/48) | open / draft。 | Desktop/Nagi Bar/Session/Kernel等の統合中です。UI・Session担当pathを変更しません。 |
| [#57](https://github.com/Roy-To/NagiOS/pull/57) | open / draft。 | Multiarchの設計・証拠を`docs/porting/multiarch/**`等に集約しています。そこへ書き込みません。 |
| [#99](https://github.com/Roy-To/NagiOS/pull/99) | open / draft。 | Dependency/cacheと`tools/nagi-cli/**`、Cargo関連pathに触れます。そこへ書き込みません。 |

### PT-01との機能境界

PT-01の既存割当記録は、PT-01の診断実装・report schema・fixtureを`tools/nagi-porting/**`にまとめる案を記録していますが、同記録上はIntegration Ownerとの競合確認・統合受領が未完で、実装・host-only試験は未開始です。別途、#91のopen draftにはNagi CLI側のread-only source readiness scannerがあります。両方とも現行mainに統合済みの実装とみなさず、同じ診断機能を二重に作らないでください。NDP-01で両者のowner、契約、正規report/CLI接続を照合し、FounderとIntegration Ownerが先行関係を確認するまでPT-03の実装範囲を広げません。

## 7. 実行バックログ案（12件）

全件の担当案はCore Developerです。各statusは **PROPOSED / NOT STARTED**、pathとbranchは全て未割当です。

> 2026-11-15: 全件の担当がCore DeveloperにCONFIRMED。NDP-01はDONE、NDP-10は開始承認済み（[開始記録](co-developer-starter-kit/START_AUTHORIZATION.md)）。下表の状態欄は提案時点のもの。

| ID | PT | 目的・想定成果 | 依存・先行 | 状態 |
|---|---|---|---|---|
| NDP-01 | 共通 | read-only ownership/contract map。PT-01と#91の診断責務、SDK/Manifest/Packageの正本、既存owner・registry、候補環境、blockerを短くまとめる。 | なし。ここでPT-01の正規interfaceを合意できない場合、関連実装を保留する。 | PROPOSED / NOT STARTED |
| NDP-02 | PT-02 | Level 0の雛形仕様。既存Manifest schema/validatorと、SDKを外部から参照する正規方法をownerと確認し、生成物と未対応API表示を定義する。 | NDP-01、App SDK owner。SDK配送方式の合意前は具体的な依存先を生成しない。 | PROPOSED / NOT STARTED |
| NDP-03 | PT-02 | 明示選択した出力先に最小app templateを作るpilot。既存ファイルを上書きせず、現行SDKに存在しないAPIや権限を含めない。 | NDP-02、専用pathとhost-only実行条件の登録。 | PROPOSED / NOT STARTED |
| NDP-04 | PT-03 | Windows由来APIの静的diagnostic rules/fixtures。Win32や.NET等は検出根拠と未対応・未検証表示を出し、互換性を認定しない。 | NDP-01、既存PT-01の入力/report interfaceとscanner owner合意。 | PROPOSED / NOT STARTED |
| NDP-05 | PT-03 | Linux由来APIの静的diagnostic rules/fixtures。POSIX、GTK、Qt等は候補検出と調査点を出し、包括的対応を主張しない。 | NDP-01、NDP-04と同じPT-01 interface合意。 | PROPOSED / NOT STARTED |
| NDP-06 | PT-04 | 許諾済みの小さなdeclarative UI fixtureを使うbounded static analysis pilot。構造・部品・event・unknownを分類し、入力を変更しない。 | NDP-01、対象UI形式と既存Nagi Native UI contractの確認。 | PROPOSED / NOT STARTED |
| NDP-07 | PT-04 | UI mapping/report案。見た目と操作、入力/focus、accessibility、localization、未実装backendを分けて記録する。 | NDP-06、UI contract ownerとの接続点合意。 | PROPOSED / NOT STARTED |
| NDP-08 | PT-05 | 正規tool利用表とstage contract。既存`./nagi`のdoctor/fetch/build/image/run/testと`tools/nagi-pkg`のManifest検証を再利用し、重複runnerを作らない。 | NDP-01、CLI/Package owner確認。 | PROPOSED / NOT STARTED |
| NDP-09 | PT-05 | 承認された場合のみthin orchestration pilot。Stageごとの実行状態・診断・未検証をまとめる。任意build scriptや未信頼codeを無断実行しない。 | NDP-08、独立pathと実行Scopeの明示。 | PROPOSED / NOT STARTED |
| NDP-10 | PT-06 | 評価fixtureの権利・選定表。最初は合成または自作の小さなfixtureを候補にし、実アプリは利用・変更・公開許諾を確認してから選ぶ。 | NDP-01、既存PT-06 template確認。 | PROPOSED / NOT STARTED |
| NDP-11 | PT-06 | toolkit評価票と比較run案。手動／Toolkit、AIなし／任意のlocal LLMを区別し、作業時間・再利用・手戻り・未対応・検証結果を記録する。 | NDP-10、PT-02〜05の使える範囲。 | PROPOSED / NOT STARTED |
| NDP-12 | 共通 | 統合reviewとhandoff。重複・SDK不足・未解決API・次の小分け案を既存ownerへ返し、次のbacklogとmerge境界を提案する。 | NDP-02〜11の該当成果、Integration Owner確認。 | PROPOSED / NOT STARTED |

### 優先キューと初期並行案

1. **前提整理:** NDP-01のowner/path/contract照合を先に行います。PT-01診断の正規経路やSDK配送が未解決なら、コードの重複を避けるため該当PT-02/03実装は開始しません。
2. **最初の3〜4件候補:** 独立path・branch、PT-01 interface、許諾済みhost fixture、担当者・検証条件が確定した後に限り、NDP-04（Windows診断rules）、NDP-05（Linux診断rules）、NDP-06（UI静的解析）、NDP-10（PT-06 fixture準備）を並行候補とします。NDP-04/05は新scannerではなく既存の正規診断面へ接続する案です。
3. **次:** NDP-02でSDK/Manifest契約と外部SDK参照方法を固め、NDP-08で正規toolのStageを整理します。その後に限り、NDP-03/NDP-07/NDP-09を担当pathに応じて検討します。
4. **評価・引継ぎ:** NDP-11で許諾済みfixtureを評価し、NDP-12で結果を各ownerと次のIntegration Ownerへ返します。

並行数「3〜4」は初期の提案値です。path競合、owner不在、未確定契約、資源/環境の不明が一つでもあれば並行作業を減らすか保留します。並行のために共有CLI、SDK、Package、Registry、CIや他owner pathを触りません。

## 8. PR、レビュー、統合、受入

- コードtaskはregistryに登録された専用branchと許可pathから行い、指定されたbase/merge boundaryへのPRでレビューを受けます。path/ownerが未確定の間はPR先や統合branchを推定しません。
- 1つのPRは独立して読める小さな目的に絞り、task ID、変更path、既存契約との関係、実施した確認、未検証事項を記載します。共有ファイル変更はIntegration Ownerのcheckpointと関係ownerの契約確認が先です。
- PR review/CIは指定ownerの手順に従います。速度優先のDEC-049に合わせ、報告や補助資料を必要以上に増やしません。実行した確認と結果は1〜3行程度で足りますが、未実行をPASSと書かず、host-onlyとNagi target acceptanceを混同しません。
- **PT-05 Stage 1:** Manifest/API/依存の利用可能な静的・契約確認。対象アプリのbuild scriptや実行ファイルを走らせません。
- **PT-05 Stage 2:** 明示的に許可されたhost build/test/package確認。Nagi target acceptanceではありません。
- **PT-05 Stage 3:** 対象targetと必要gate/owner/環境が別途利用可能・許可済みになった場合のみ、同じartifactでinstall/launch/permission/UI等を確認します。Stage 1/2の結果をStage 3 PASSにしません。
- 署名鍵、正式公開、配布、製品gate変更はこの割当案の範囲外です。結合テスト・merge・mainへの反映はIntegration OwnerとFounderが決める既存手順に従います。

- **0.2 activation:** `docs/0.2/WORKSTREAMS.md`は、M30 PASS前の一般作業をdevelopment-foundationに限定し、例外は範囲を明示した個別checkpointとして扱います。#91 draft branchのhost-only許可もその正確なpathに限られ、本案へは引き継がれません。M30前にPT-02〜06のhost実装を行うなら別の明示的checkpointが必要です。0.2 runtime/target integrationにはM30 PASSと明示的な0.2 release境界が必要です。

## 9. 最小限の報告とblocker対応

- **着手時:** task ID、owner、branch/base、許可path、依存、検証範囲を一度記録します。
- **途中:** path競合、shared contract変更、release gate、検証失敗、担当外への影響、外部送信・費用が判断に必要になったときだけ報告します。
- **完了時:** commit/PR、実際に変更したpath、実行した確認と結果（1〜3行）、host/targetの区別、未解決事項、次のowner/actionを短く渡します。
- **blocker:** 権限、path、SDK/Manifest契約、fixture許諾、environment、共有資源の不明は関連taskを止めてownerに戻します。別branch・別環境・別担当pathから迂回しません。作業差分と実行結果は残し、DEC-049の有限retry方針に従います。製品仕様・セキュリティ・release gate判断はFounder、共有path/統合判断はIntegration Ownerへ分けます。

## 10. 実装開始前の確認

- FounderがPT-02〜PT-06の担当案、最初のtask群、Core Developerの責任範囲を具体的に決める。
- 既存PT-01担当・Integration OwnerがPT-01/#91の診断面を照合し、PT-03が使う正規interfaceを決める。
- App SDK ownerが外部appのSDK参照方法、実在するManifest/API、PT-02接続条件を確認する。
- Integration Ownerが新しいworkstream ID、owner、branch、path allowlist/denylist、state、PR base/merge boundaryを整える。
- Founderが必要なGitHub権限、利用環境、host-only検証条件、資源を別途承認する。未承認の招待や環境作成は行わない。
- 各確認後、backlog statusと並行数を更新します。これらの確認前にコード実装・試験・target acceptanceが始まったと報告しません。

## 参照資料

- [DEC-045 — Nagi Porting Toolkit](../product-decisions/DEC-045-nagi-porting-toolkit.md)
- [DEC-046 — 共同開発者の役割と自律性](../product-decisions/DEC-046-co-developer-governance.md)
- [DEC-047 — Linux移行レーンの作業場所と統合先](../product-decisions/DEC-047-linux-migration-workspaces-and-integration.md)
- [DEC-049 — 速度優先の開発手続き](../product-decisions/DEC-049-development-process-speed-priority.md)
- [DEC-050 — 公開仕様リポジトリ](../product-decisions/DEC-050-public-specification-repository.md)
- [製品決定の索引](../product-decisions/README.md)、[共同開発者向けオンボーディング](co-developer-onboarding-ja.md)
- [0.2 Workstream Contract](../0.2/WORKSTREAMS.md)、[`.dev/workstreams.json`](../../.dev/workstreams.json)
- [PT-01 host-only担当記録](../workstreams/PT-01-readonly-host-assignment-2026-11-13.md)
- [PT-02 SDK surface audit](../porting-toolkit/PT02_SDK_SURFACE_AUDIT_2026-11-13.md)、[PT-06 evaluation template](../porting-toolkit/PT06_MIGRATION_EVALUATION_TEMPLATE_2026-11-13.md)
- [App SDK Contract v1](../../sdk/rust/docs/APP_SDK_CONTRACT.md)、[App Manifest schema](../../sdk/rust/schemas/app-manifest.schema.json)
- [0.3 Revised Roadmap](../product-decisions/roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[0.4+ Multi-Agent Roadmap](../product-decisions/roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)、[0.5–1.0 Master Roadmap](../product-decisions/roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

**この文書の保存・PR作成は、提案の公的な記録です。PT-02〜PT-06の実行権限や着手許可ではありません。**
