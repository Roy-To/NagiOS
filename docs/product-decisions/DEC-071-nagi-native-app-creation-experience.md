# DEC-071 — 初心者向けNagi Native App制作体験

**状態:** CONFIRMED（純正の初心者向け制作体験、自然言語とローカルLLMを活用した小規模な日常アプリ、試用・導入・共有／配布を目指す方向）／PROVISIONAL（製品形態、Porting Toolkitとの統合、モデル・SDK・実装・配布の詳細、適用版）
**最終合意日:** 2026-11-15
**適用候補:** 将来のNagi Native App制作・開発者／配布環境。release・実装時期は未確定
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-002](DEC-002-cloud-ai-data-sharing.md)、[DEC-015](DEC-015-builder-role-review.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-053（AI Task開始前の復元可能性表示）](DEC-053-ai-task-recovery-preview.md)、[DEC-036（アプリ試用・導入）](DEC-036-builder-app-trial-and-installation.md)、[DEC-036（共有・公開）](DEC-036-builder-app-sharing-and-publication.md)、[DEC-040（開発Project保存）](DEC-040-builder-app-development-project-retention.md)、[DEC-040（Action API連携）](DEC-040-builder-generated-action-api-integration.md)、[DEC-045（Nagi Porting Toolkit）](DEC-045-nagi-porting-toolkit.md)、[DEC-043（独立OS方針）](DEC-043-nagi-userland-independent-os.md)、[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)、[0.5–1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

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
- **試用・導入・共有（DEC-036の各文書）:** 試用可能範囲と証拠、採用判断、正式インストール、個人利用、限定共有・公開を定める。本DECで承認の流れを緩めない。
- **開発Project・Action API（DEC-040の各文書）:** ソース等の長期保持と、必要なアプリ機能を正規Action APIとして提案・検証する境界を定める。本DECは保存形式やAction契約を作り直さない。
- **現状の実装との区別:** 公開正本で確認したのは製品方針・設計案であり、初心者向け制作ソフト、Porting Toolkit、Nagi Native App制作から配布までの動作が既に提供されていることではない。

## 差別化の候補（REVIEW NOTE／PROVISIONAL）

2026-11-15に確認したGoogle公式情報では、対応するChromebookのLinux環境で開発者がCLI、コードエディタ、IDEを導入してアプリを作れる。Googleはさらに、Googlebook上のAntigravityをアプリのbuild／test／deployに使えるAI開発環境として紹介し、Gemini Canvasによるスケッチからのアプリ生成とPreview、Google AI Studioのfull-stack vibe codingも公表している。したがって「自然言語でコードを作る」「生成結果をPreviewする」だけでは、Nagi独自の価値を説明しにくい。

- [ChromebookでLinuxを設定する（Google Chromebook Help）](https://support.google.com/chromebook/answer/9145439)
- [Googlebookの予約開始と開発者向け機能（Google、2026-09-21）](https://blog.google/products-and-platforms/devices/googlebook/pre-order-googlebook/)
- [Gemini Canvasでスケッチからアプリを作る（Google、2026-11-05）](https://blog.google/innovation-and-ai/products/gemini-app/turn-drawings-into-apps-gemini/)
- [Google AI Studioのfull-stack vibe coding（Google、2026-03-18）](https://blog.google/innovation-and-ai/technology/developers-tools/full-stack-vibe-coding-google-ai-studio/)

**提案候補 — まだ製品仕様としては未承認:**

1. **Nagi Nativeを成果にする:** Web prototypeのPreviewではなく、実際に公開されたNagi SDK／Package／Permission契約とつながるOS上のNative Appを試せることを軸にする。
2. **Local-firstの制作:** 端末内のローカルLLMを第一の制作手段にでき、外部モデルへソースやContextを渡す場合はDEC-002と現行Permissionに従う。完全Offline動作、Cloud fallbackの有無、既定値は未決定。
3. **確認済みの事前表示を制作体験にも適用する:** AI Task開始前に、そのTaskでWaybackからどこまで復元できるかを示す方向は[DEC-053](DEC-053-ai-task-recovery-preview.md)に記録する。アプリ制作Taskでは、実際に保護・復元できる範囲や不明な範囲のうち何を表示するか、表示粒度、操作との対応は未確定。DEC-014のWorkspace復元PreviewとDEC-026の変更保護・検証を混同しない。
4. **Permissionを制作体験へつなぐ:** アプリが要求するCapabilityと理由を分かる言葉で説明し、ユーザーの現行OS Permission／Policyがそのまま実行時に適用されることを制作・試用の中で示す。アプリ生成自体でPermissionを与えない。
5. **証拠を見てから採用する:** 試用可能な環境で実際の動作、検証済み範囲、未検証事項、要求権限を示し、試用成功・採用判断・正式導入・配布の各段階を分ける。これは既存DEC-036の境界をUX上で理解しやすくする候補である。

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

関連する個人化の追加決定は[DEC-070](DEC-070-memory-informed-native-app-creation.md)を参照。初心者向け制作という共通方針を共有し、Memory利用の別決定を置き換えない。
