# DEC-071 — 初心者向けNagi Native App制作体験

**状態:** CONFIRMED（既存の初心者向け自然言語・ローカルLLM制作／試用・導入・共有の将来方向、PX-13の対話型共同制作B＋将来の継続的・個人適応型共同制作C、当面はローカル完結型制作環境よりCloud AIとの橋渡し・移植支援を重視する優先方向）／PROVISIONAL（画面・Preview・修正導線・Memory許可UI、Cloud接続・Provider／データ交換方式・モデル／SDK／技術・性能・具体版・受入）
**最終合意日:** 2026-11-17（PX-13と優先順位を後続確認。2026-11-15の制作構想は長期方針として維持）
**適用候補:** 将来のNagi Native App制作・開発者／配布環境。高度な統合制作体験は低優先の長期目標（1.0計画のごく後期が目安）とし、release確約・実装時期は未確定
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-002](DEC-002-cloud-ai-data-sharing.md)、[DEC-015](DEC-015-builder-role-review.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-053（AI Task開始前の復元可能性表示）](DEC-053-ai-task-recovery-preview.md)、[DEC-075（アプリ試用・導入）](DEC-075-builder-app-trial-and-installation.md)、[DEC-036（共有・公開）](DEC-036-builder-app-sharing-and-publication.md)、[DEC-040（開発Project保存）](DEC-040-builder-app-development-project-retention.md)、[DEC-076（Action API連携）](DEC-076-builder-generated-action-api-integration.md)、[DEC-045（Nagi Porting Toolkit）](DEC-045-nagi-porting-toolkit.md)、[DEC-043（独立OS方針）](DEC-043-nagi-userland-independent-os.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)、[0.5–1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

## 目的とユーザー体験

プログラミング経験が少ない人も、日常で使う小さな道具を自然言語で説明し、ローカルLLMの支援を使ってNagi Native Appとして形にし、試し、必要なら導入・共有できる純正の制作体験を目指す。

これは、既存アプリのコードをNagiへ移すPorting Toolkitの作業そのものとは別の入口である。DEC-045のToolkit構想を、新規アプリを一から作りたい初心者にも広げる製品方向を記録する。コード担当のbuilder Role（DEC-015／016）や、アプリの試用・導入・共有を定める既存DECを置き換えない。

## 確定した製品方向

1. **Nagi純正の初心者向け制作体験を設ける。** ユーザーはIDEやSDKの詳細を知らなくても、自然言語で小規模なアプリの目的や動きを説明し、制作を進められる。伝統的なIDEそのものにするか、会話型の案内画面を中心にするかは本決定では指定しない。
2. **制作支援にローカルLLMを活用する。** 端末上のローカルLLMを使って初心者のNagiアプリ制作を助けることを製品方向に含める。大規模なアプリや複雑な開発までローカルモデルで完成できるとは約束しない。
3. **最初の対象は日常使いする小さなアプリとする。** 大規模・高難度のソフトウェア制作を最初の成功条件にせず、小さな日常向けユーティリティを自然言語から作る体験を目指す。対象規模の判定基準や対応機能数値は定めない。
4. **成果物はNagi Native Appを目指す。** DEC-017／公開0.2アプリ契約が定めるNagi SDK、Manifest、Package、Public API、Permission／Capability／Policyの原則に沿わせる。未提供のAPI・UI・Package機能を利用可能と扱わない。一般的なWebアプリの生成Previewだけで終わることや、他OSのバイナリを無変更で実行することを本決定の成果条件にはしない。
5. **制作から試用・配布までの道筋を用意する。** 新しいアプリを作るだけでなく、結果を試し、個人利用向けに導入し、ユーザーの選択によって他者へ共有・配布できる製品体験を目指す。試用、採用判断、正式導入、外部送信、一般公開は既存の別々の承認境界を維持する。
6. **Porting Toolkitの製品方向を広げる。** 既存アプリの移植支援に加え、新しいNagi Native Appを作る初心者向け入口を扱う。ただし、同じアプリ／プロセスとして一体化するか、PT-01〜06のどの機能を再利用するかは未決定であり、初心者向けのユーザー体験を追加する今回の方向を、既存PTの実装完了や新規実装許可とは解釈しない。

## 既存決定との役割分担

- **builder Role（DEC-015／016／018）:** Nagiのプログラミング作業のRole、Task Scope、自律実行、証拠、隔離編集、条件付き反映を定める。これらは初心者向け製品体験や完成済みIDEの決定ではない。
- **開発者環境・Porting Toolkit（DEC-017／045）:** Nagi Native Apps向けの開発者環境、既存アプリの移植、PT-01〜06のツール設計を扱う。特にPT-02の雛形生成とPT-05の既存SDK／Packageに基づく検証は接続候補だが、Porting Toolkit文書は設計方針であり、PT-01〜06の実装済み・担当割当済みを示さない。
- **試用・導入（DEC-075）・共有（DEC-036）:** 試用可能範囲と証拠、採用判断、正式インストール、個人利用、限定共有・公開を定める。本DECで承認の流れを緩めない。
- **開発Project（DEC-040）・Action API（DEC-076）:** ソース等の長期保持と、必要なアプリ機能を正規Action APIとして提案・検証する境界を定める。本DECは保存形式やAction契約を作り直さない。
- **現状の実装との区別:** 公開正本で確認したのは製品方針・設計案であり、初心者向け制作ソフト、Porting Toolkit、Nagi Native App制作から配布までの動作が既に提供されていることではない。

## PX-13 — 対話型共同制作B＋将来の個人適応型共同制作C／当面のCloud AI橋渡し重視（2026-11-17）

### CONFIRMED — 将来の完成形はB案を基本、将来C案へ発展

- **B：対話型の共同制作体験。** 自然言語による目的説明を起点に、実際の機能・画面の設計案、利用可能なPreview、ユーザーの修正指示、承認されたScope内のBuild／Test、確認できる場合の安全な試用、採用判断から別途承認付きの正式導入までを、初心者にも分かりやすく一連の体験として目指す。Preview／Testは検証済み動作と未検証項目を区別し、試用・正式導入を一つの承認へまとめない。
- **将来C：継続的・個人適応型の共同制作体験。** 既存の[DEC-070](DEC-070-memory-informed-native-app-creation.md)に従い、同一User／Profileの許可されたMemory・明示設定・ユーザーの採用／修正履歴を利用して、設計候補や改善案を提案できる方向へ発展させる。[DEC-040](DEC-040-builder-app-development-project-retention.md)の保護された開発Projectから後日再発見・改良できる方向を維持する。導入済みアプリの無断変更は行わず、正式更新にはDEC-035の別承認を要する。
- 両案とも**製品体験の高水準方向**であり、IDE一体化・画面形式・Previewエンジン・Memory参照UI・技術方式・モデル・提供版の決定ではない。

### CONFIRMED — 当面はローカル完結型の開発環境に固執せず、Cloud AIとの橋渡しと移植支援を重視

- 現在すでに利用できる一般的な自然言語コーディング環境（例：外部のCloud AIを使った開発）と競合する**独自の全面統合型アプリ開発環境の早期完成を急がない**。Nagi側はまず、実在するNagi SDK／API・Package／Permissionの契約、移植対象の診断結果・未対応点、確認済みのBuild／Test結果等を、開発者が選択した既存IDE・人間・外部開発AIと**安全かつ円滑に受け渡せる開発補助／橋渡しの体験**を重視する。
- **ローカルLLMは橋渡し・説明・軽量な補助を担える方向**とし、例えばNagi固有の制約を説明したり、正規SDK/APIの利用可能範囲を確認して移植・実装候補を整理したり、許可された外部開発AIへ渡す候補情報と返された成果を理解・レビューする補助を目指す。Local AIだけでアプリ全体を完成させることを当面の成立条件にせず、外部Cloud AIを使う選択を可能にする。外部Cloud AI使用そのものは必須ではなく、人手・非AI診断・ローカル開発も利用できる。
- **他OSアプリのNagi Native移植支援を重要な用途**とする。[DEC-017](DEC-017-third-party-developer-ecosystem.md)／[DEC-045](DEC-045-nagi-porting-toolkit.md)のWindows／Linux先行、Portable coreの再利用、PT-01〜06の非AI診断／段階的変換・検証を維持し、ローカルLLMがCloud AI／開発者との間でNagi側の仕様・不足API・検証情報を橋渡しできる方向を追加する。移植支援は既存アプリの無変更実行や自動移植成功を意味しない。
- **優先順位と時期:** Nagi OSの基盤・日常のAI組み込み体験を先に進める。完成度の高い独立／統合アプリ制作環境と高度な共同制作体験は**1.0に至るロードマップの最終盤を目安とする低優先の長期目標**であり、近距離版のRelease Blockerや初期0.4の完了条件へ新たに昇格させない。**1.0の特定版・後半フェーズでの提供確約、または1.0までに必須として完成させる合意ではない**。一方、DEC-017／DEC-021で既にCONFIRMEDの**1.0までの公開開発者環境・アプリ配布基盤という高水準目標**は別対象として維持する。host-only診断等の非競合な先行準備を一律に止める方針でもない。

### 維持する既存の安全／製品境界（CONFIRMED）

- **[DEC-002](DEC-002-cloud-ai-data-sharing.md):** 外部AIへのソース・Task Context・開発情報・Resource・Memory送信は、利用者が許可したProvider／Service、Purpose、情報区分、対象Resource、User／Profile、期間／Task等のScopeと現行Permission・組織Policyに従う。分類不明・機微／秘密情報の無断送信を認めず、Activityに記録し取り消し可能にする。ローカルLLMによる整理・要約や橋渡しの役割は**送信許可ではない**。Cloud AIをNagi Memoryの正本としない。
- **builderとNative App契約:** 初期はNagi自身が`builder` Roleへ切替（DEC-015／016）。実行にはTask／Action／Workspace／Resource Scopeの承認、隔離編集・検証と元ファイル保護（DEC-018）、実在するSDK／Package・Permission／Capability／Policyを必要とする。AI生成結果を未検証のまま適用・正式導入しない。Git commit／pushや外部送信・公開は既存の別承認。
- **試用・導入・共有・更新:** DEC-075の安全な試用→ユーザー採用判断→正規Package検証→正式導入の別承認、DEC-036の個人利用標準と共有／公開の別承認、DEC-035の正式更新承認、DEC-040／041の開発Project・利用者データの保護／削除分離、DEC-076の用途別Public Action提案と通常GUI維持を継承する。
- **Memory個人化:** DEC-070で許可されたMemoryは設計案への利用であり、元のMemory・Credential・Permissionを生成アプリや外部Cloudに暗黙に転送せず、User／Profileの所有・分類・忘却の規則を守る。AIが不在でも非AIの開発者契約・通常アプリGUIを妨げない。
- **Release／実装Gate:** 本決定は新しいWorkstream着手許可、PT-01〜06の実装完了、0.4 Entry Gate／Integration Owner checkpointの解除、1.0公開の承認、性能保証ではない。現行Release／Activation／Acceptance GateとWorkstream ownerを維持する。

### PX-13の詳細未決事項（PROVISIONAL／OPEN）

- 対話・設計案・Preview・修正・実装／テスト・試用／採用・導入の具体的画面構成と導線、独立IDE／Nagi Home内などの製品形態、初期の対応Appタイプ／規模。
- Local LLMと外部開発AIの役割分担の具体的方式、Provider／Connector／認証／通信・データ交換／SDKメタデータSchema、ソース／Contextの選択・共有UI、Cloud許可UIの実装、モデル／性能・端末要件、Offlineで可能な工程とFallback。
- 既存アプリ移植でCloud AIを組み込むときの診断→補助→修正提案→差分検証の技術・対応対象／Framework、出力や検証証拠の互換、工数削減・安全性の受入値。
- Memory参照の選択・許可・取消・レビューの詳細UI、個人向け設計案の生成・表示、開発Project保存／後日再発見のUI、独立した配布／正式更新手順のUI。
- 具体的な完成時期と対象Version、1.0との関係、担当・Workstream・優先度の実行計画、品質基準・Acceptance、正式リリース判断。

## 差別化の候補（REVIEW NOTE／PROVISIONAL）

2026-11-15に確認したGoogle公式情報では、対応するChromebookのLinux環境で開発者がCLI、コードエディタ、IDEを導入してアプリを作れる。Googleはさらに、Googlebook上のAntigravityをアプリのbuild／test／deployに使えるAI開発環境として紹介し、Gemini Canvasによるスケッチからのアプリ生成とPreview、Google AI Studioのfull-stack vibe codingも公表している。したがって「自然言語でコードを作る」「生成結果をPreviewする」だけでは、Nagi独自の価値を説明しにくい。

- [ChromebookでLinuxを設定する（Google Chromebook Help）](https://support.google.com/chromebook/answer/9145439)
- [Googlebookの予約開始と開発者向け機能（Google、2026-09-21）](https://blog.google/products-and-platforms/devices/googlebook/pre-order-googlebook/)
- [Gemini Canvasでスケッチからアプリを作る（Google、2026-11-05）](https://blog.google/innovation-and-ai/products/gemini-app/turn-drawings-into-apps-gemini/)
- [Google AI Studioのfull-stack vibe coding（Google、2026-03-18）](https://blog.google/innovation-and-ai/technology/developers-tools/full-stack-vibe-coding-google-ai-studio/)

**2026-11-17後続確認:** 下記の旧REVIEW NOTE「Local-firstの制作」はローカルLLMのみを必須の既定開発経路とする決定ではなく、PX-13でCONFIRMEDとなったCloud AIとの橋渡し重視が優先する。外部送信はDEC-002の許可内に限定する。

**提案候補 — まだ製品仕様としては未承認:**

1. **Nagi Nativeを成果にする:** Web prototypeのPreviewではなく、実際に公開されたNagi SDK／Package／Permission契約とつながるOS上のNative Appを試せることを軸にする。
2. **Local-firstの制作:** 端末内のローカルLLMを第一の制作手段にでき、外部モデルへソースやContextを渡す場合はDEC-002と現行Permissionに従う。完全Offline動作、Cloud fallbackの有無、既定値は未決定。
3. **確認済みの事前表示を制作体験にも適用する:** AI Task開始前に、そのTaskでWaybackからどこまで復元できるかを示す方向は[DEC-053](DEC-053-ai-task-recovery-preview.md)に記録する。アプリ制作Taskでは、実際に保護・復元できる範囲や不明な範囲のうち何を表示するか、表示粒度、操作との対応は未確定。DEC-014のWorkspace復元PreviewとDEC-026の変更保護・検証を混同しない。
4. **Permissionを制作体験へつなぐ:** アプリが要求するCapabilityと理由を分かる言葉で説明し、ユーザーの現行OS Permission／Policyがそのまま実行時に適用されることを制作・試用の中で示す。アプリ生成自体でPermissionを与えない。
5. **証拠を見てから採用する:** 試用可能な環境で実際の動作、検証済み範囲、未検証事項、要求権限を示し、試用成功・採用判断・正式導入・配布の各段階を分ける。これは既存DEC-075の境界をUX上で理解しやすくする候補である。

これらはGoogle製品に対する実証済みの優位性でも、確定機能一覧でもない。技術・権限・SDK・Waybackの実装状況を確認し、対象ユーザーで有用性を測る必要がある。

## 未確定事項

- 製品名、GUI／IDEとしての形、会話・コード・視覚編集の比重、Porting Toolkitへの統合／分離、builder Roleとの接続。
- ローカルモデル、Runtime、モデル切替、端末性能・Memory要件、LLMを利用しない生成・検証工程、完全Offlineの可否、Cloud AIへのFallbackとContext送信方法。
- 初期に対象とするアプリの大きさ・分野、言語・Framework、SDK／UI／Renderer依存、Nagi Native上で試せる範囲。
- Wayback／Checkpointで保護・復元できるコードや生成物の範囲、事前表示の対象・精度、Build／TestやApp data等の副作用との境界。
- 正規Package／署名、試用環境、インストール・更新・共有・公開経路、受取側検証、Storeを含む流通方式。
- リリースへの割当て、Workstream、担当、対象端末、Acceptance条件、開発・性能予算。

## 例外・禁止・安全境界

- 自然言語生成、ローカルLLM利用、builder Role切替は、承認されたTask Scope、現行Permission／Capability／Policyを拡大しない。コード変更とBuild／TestはDEC-001／015／016／018の境界に従う。
- Cloud modelへのContext送信、ソース出力、Package共有、外部公開・配布は、それぞれDEC-002／036に従う。アプリを作る承認やローカルで試す承認だけで外部送信・公開を行わない。
- 試用・採用判断・正式インストールを一つの操作にまとめず、Package検証と別途の導入承認を維持する。実装されていないRecovery、Wayback対応、Package検証を可能と偽らない。
- Nagi Native以外の互換Runtime、既存Windows／Linux／Androidアプリバイナリの無変更実行、複雑なアプリの完全自動生成は本決定に含まない。移植支援はDEC-017／045の別の作業経路とする。
- 本決定は製品方向の記録であり、ソフトの実装開始・完成、PT-01〜06の担当割当、0.2-M21のSDK／Package契約変更、0.3 Acceptance、0.4 Entry Gate／Integration Owner checkpoint、Activation、Release commitmentを意味しない。0.5–1.0 Master Roadmapの版別配置も変更しない。

## Roadmapとの関連

DEC-017が定める1.0に向けた公開開発者環境とアプリ配布基盤、およびDEC-043の独立したNagi Native App環境の製品方針と整合する。0.4+ Roadmapの初期Nagi／builder Role workflowを補うユーザー向け構想だが、本DECでは0.4や他の特定Versionに割り当てない。リリース版、順序、Acceptance、実装開始は別途決定する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-15 | ユーザーが、ローカルLLMを使い初心者が自然言語から小規模なNagi Native Appを作成し、試用・正式導入・共有／配布できる純正体験を望む方向を確認。製品形態、実装・配布方式、モデル条件、適用版は暫定として分離した。 |
| 2026-11-16 | PR #110の旧DEC-052をDEC-071へ改番。mainの実機候補DEC-052との番号競合を解消。決定内容・状態・原合意日を保持。 |
| 2026-11-17 | PX-13は対話型共同制作Bを基本・将来の個人適応型継続制作Cを高水準CONFIRMED。ただし当面はローカル完結型の制作環境完成を急がず、ローカルLLMをNagi SDK／移植診断とCloud AI／既存IDE・人間との安全な橋渡しに活用する方向を優先。独立した高度な制作環境は1.0計画の最終盤を目安とする低優先長期目標（提供Version・時期はPROVISIONAL）とし、DEC-017の1.0公開開発者環境／配布基盤目標、DEC-002の事前許可、既存試用・導入・更新・共有Gateを維持。 |

関連する個人化の追加決定は[DEC-070](DEC-070-memory-informed-native-app-creation.md)を参照。初心者向け制作という共通方針を共有し、Memory利用の別決定を置き換えない。
