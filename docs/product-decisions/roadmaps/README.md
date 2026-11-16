# NAGI Future Roadmaps

## 近距離ファイル共有の製品方針（2026-11-16）

[DEC-064](../DEC-064-nagi-share-nearby-file-transfer.md)でNagi Share（仮称）をNagiのPC／Tablet／Phone間の純正ファイル共有機能として採用した。共有UIから近くの受信可能端末を発見・選択し、受信側の確認・許可、暗号化・端末認証と既存の認可・情報区分・所有／Profile境界に従ってローカルに直接転送する。Internet・Cloud account・LLMは基本利用に必須としない。

日常の単発ファイル転送はDEC-014のWorkspace移行・同期、Team／Organization／Remote Agentとは別の範囲である。名称・通信方式・互換・保存先・距離・UI／通知／承認詳細・対応版／端末・性能はPROVISIONALで、0.8等の特定版へ割り当てず、Desktop firstと既存Release／Activation Gateを維持する。Nagi Shareからページ／Project／Workspace／作業状態Handoffへ拡張する案はアイデアであり、既存Continuity仕様の撤回でも、その拡張の採用確定でもない。

**保存先:** `Roy-To/NagiOS` / `docs/product-decisions/roadmaps/`
**更新日:** 2026-11-16（公開移管は2026-11-15、[DEC-050](../DEC-050-public-specification-repository.md)を参照）

| ファイル | 位置付け |
|---|---|
| [Nagi_OS_0.3_Architecture_Roadmap_Revised.md](Nagi_OS_0.3_Architecture_Roadmap_Revised.md) | 0.3 Architecture Draft、2026-10-25 Vision addendum、2026-11-04 Memory Engineの未確定方式を明記。`docs/Nagi_OS_0.3_Architecture_Roadmap.md` の改訂版 |
| [Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md](Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md) | 0.4+ Vision proposal。Implementation Specificationではない |
| [Nagi_OS_0.5-1.0_Master_Roadmap.md](Nagi_OS_0.5-1.0_Master_Roadmap.md) | DEC-021に基づく段階案。継承方針と暫定の版割当てを区別し、Implementation Specification / Release commitmentではない |

0.3と0.4+の文書は製品構想、0.5–1.0 Master Roadmapは製品決定と承認済み方向を整理した提案である。どれも実装済みであることやreleaseの確定を意味しない。

## 端末間接続方針（2026-11-16）

**接続方針の変更履歴（2026-11-16）:** [DEC-066](../DEC-066-device-connectivity-lan-first.md)で初期は同一Wi-Fi／LANのNagi端末、共通Device Connectivity／Trust／Transport、Offline-firstと認可境界を確定。近距離直接通信・遠隔接続は拡張可能性を保持し、初期の遠隔実装・常時Cloud・オンラインアカウント・運営サーバーは必須にしない。具体方式・対象機器・運用費用・版・時期・性能は未確定。DEC-014の選択式移行／同期とCloud別許可を維持し、無断転送・同期・Task再実行なし、Clipboard未採用。各ロードマップの既存Gate・版割当ては変更しない。

**Foundation #32 Device Connectivity（2026-11-16）:** [DEC-101](../DEC-101-device-connectivity-nagi-protocol-linux-research.md)で、Linux／OSSの既存通信技術とNagi独自Connectivity Serviceを組み合わせるBを製品基本とし、Nagi Share／Handoff等のPC／Tablet／Phone間で共通利用するNagi独自の意味的連携Protocolを設計すること、必要な独自通信技術を並行研究することをCONFIRMED。上位Protocolの設計とTransport／暗号機構は分離し、全面独自通信Stackや自作暗号は今回採用しない。研究成果の製品採否は実測と別判断で決定する。DEC-066の同一LAN初期接続・後続直接／遠隔拡張余地、DEC-014／064／065の独立した認可・移行・Share／Handoff契約、Offline-firstと無断同期・転送／Task再実行禁止を維持する。具体Schema・OSS／Transport・暗号方式・遠隔／Relay・端末／版・性能・Linux受入はPROVISIONAL。既存Gateや版割当てを変更せず実装済みを示さない。

変更履歴: 2026-11-16 — DEC-101でFoundation #32のB標準＋Nagi上位Protocol＋必要な独自通信研究を記録。通信実装の採否・版は未決定。

## 当面の優先順位（2026-11-16）

[DEC-074](../DEC-074-human-led-agent-organization.md)の後続確認により、当面はAI組み込み体験型OSの確立を優先し、階層型Agent Organizationは非優先の長期ビジョンとする。各Roadmapに日付付き注記を反映した。具体的な提供版・階層数・実装順序は未確定であり、0.3の単一Personal Agent、初期0.4の単一Nagi builder Role、既存Gate・Acceptanceは維持する。

## Linux方針との整合(2026-11-13)

各ロードマップにDEC-042/043/044の後続関係を追記した。[仕様・実装影響表](../KERNEL-TRANSITION-ALIGNMENT-2026-11-13.md)を参照。

## 純正ユーティリティ4分野（2026-11-16）

[DEC-062](../DEC-062-first-party-utilities.md)で、画像ビューア・簡易編集、スクリーンショット・文書スキャン、ZIP等の圧縮・解凍、動画・音楽プレイヤーの全分野採用と、AIなしの高速・軽量な基本機能優先を確定した。Nagi Project / Files / Memory / 検索 / Smart Paste等との連携は必要に応じて検討する。具体的なアプリ構成・対応範囲・連携詳細・提供版・受入基準はPROVISIONALで、0.3・0.4+・0.5–1.0への版割当てや既存Release / Activation Gateを変更しない。

## IMEの応答性と学習方針（2026-11-16）

[DEC-060](../DEC-060-responsive-ime-ai-learning.md)は、IME／日本語入力のリアルタイム経路で速度と予測可能な応答を優先し、許可されたデータに基づく学習精度向上にAIを活用する製品方針を記録する。秘密入力の除外、明示的な禁止・忘却、機密分類、DEC-002のCloud許可、AI不可時の通常IMEを維持し、モデル非依存の学習状態を目指す。0.3のPersonal Agent／Learning、後続ロードマップの前景応答性・資源管理と整合するが、IME機能の具体方式・数値・版割当ては未確定で、各ロードマップのRelease／Activation Gateを変更しない。

## Nagi Projectとの整合（2026-11-16）

[DEC-055](../DEC-055-project-folder-cross-app-context.md)の通常フォルダをProjectに指定する製品仕様はCONFIRMED。Projectは仕事の対象・関連資料・長期文脈、Workspaceは作業画面・復元状態であり、1 Projectを複数Workspaceから扱える。DEC-023／025の分散正本・許可された共通参照と更新提案、DEC-003／004のMemory規則を継承する。

0.2のResource／Public Action／Permission基盤と0.3のPersonal Agent／Memory／アプリ横断Taskの方向に接続するが、0.3提供を確約せず、0.4+の複数Agentを前提にしない。0.5–1.0の版別範囲・順序も引き続きPROVISIONAL。Projectの技術、UI、リンク形式、詳細のローカル保存方式、版割当てを本追記で確定せず、既存のImplementation Spec、release／activation gate、Acceptance、担当範囲を変更しない。仕様記録は実装・動作検証済みを意味しない。

## OS基盤の検討と進捗（2026-11-16）

**Foundation #25 Resource Manager / Scheduler（2026-11-16）:** [DEC-094](../DEC-094-resource-manager-linux-baseline-custom-scheduling-research.md)でB＋C研究案をCONFIRMED。Linux標準scheduler／資源制御を製品の基本とし、Nagi Resource Managerで前景UI／アプリ・音声・AI Task／Job／Memory／Nagi Powerの予算と優先度を協調する。独自スケジューリング（sched_ext／eBPF等は候補）の比較研究を進め、実機の応答性・AI品質／所要時間・RAM・電力／熱・公平性／安定性を評価し、十分な改善を確認できた部分のみ後続採否を判断する。Linux標準への退避を保つ。具体的制御機構・測定値・独自方式正式採用・端末・Release割当てはPROVISIONAL、既存Gate維持。

**Foundation #26 App Lifecycle（2026-11-16）:** [DEC-095](../DEC-095-app-lifecycle-linux-supervisor-runtime-research.md)で、Linuxの標準process／隔離機構＋Nagi App Supervisorを製品の基本とするB案と、独自Runtimeの並行比較研究をCONFIRMED。新規Runtimeの隔離、起動／ready速度、RAM／電力、Workspace／Public API連携、移植・保守負担の効果をLinux標準経路と実測比較し、部分採用するかを後続判断する。独自Runtime・WASM等の具体技術、アプリ分類、対応端末、導入Version・性能はPROVISIONAL。0.2 M12／M13／M21のManifest・Supervisor・SDK契約、Nagi Native Apps専用方針、試用／更新／削除の承認・Permission／Policy、既存Release／Activation／Acceptance Gateは維持する。

**Foundation #27 Settings / Notification（2026-11-16）:** [DEC-096](../DEC-096-settings-notifications-context-adaptive-staged.md)でCの状況適応型Settings／Notificationを最終製品目標にする方針をCONFIRMED。Bの共通Settings／Notification Serviceと統合Settings UI・通知センターを途中段階で導入し、許可されたWorkspace／Task・Profile文脈、明示設定、Habit-first学習に基づく通知の提示・集約・タイミング調整へ段階的に発展させる。これはCを単なる研究候補に留める判断ではない。一方、手動制御・AI不在時の通常動作、DEC-008の重大警告と控えめな働きかけ、DEC-004／057／058の学習・訂正・忘却、Permission／Privacyを維持する。具体的自動化・UI・端末・提供版／受入はPROVISIONAL、0.2 M11／M14と既存Release／Activation／Acceptance Gateは変更しない。



[Nagi OS Foundation Checklist](../NAGI_OS_FOUNDATION_CHECKLIST.md)で35項目と7検討束を恒久管理する（[DEC-069](../DEC-069-foundation-checklist-management.md)）。台帳化はCONFIRMED、優先度・検討順序はREVIEW NOTE / PROVISIONAL。各ロードマップの版割当て・Gate・担当・実装開始条件を変更せず、全35項目の完了を新しいrelease条件にはしない。

## Foundation #28：通常a11yと長期操作思想（2026-11-16）

[DEC-097](../DEC-097-accessibility-foundation-and-ai-native-long-term-direction.md)で、keyboard／navigation・screen-reader等の通常a11y基盤を先に整え、将来のAI-nativeで正確なポインティングに依存しない操作体系と矛盾しない設計にする方針をCONFIRMEDとした。短期の整備と長期ビジョンを分け、視線／ジェスチャー／空間入力等の採用・時期、time／localeの詳細方式は未決定。従来GUI・keyboard／mouse、AI不在時の通常操作、Permission／Policyと必要なconfirmationを維持する。

0.2の基盤、0.3のPersonal Agent、0.5–1.0のoffline／accessibilityテーマへ接続するが、特定Versionへの割当て、既存Release／Activation／Acceptance Gate、担当を変更せず、実装済みを示さない。

変更履歴: 2026-11-16 — DEC-097とFoundation #28の短期／長期・確定／未確定を参照追加。

## Foundation #29：Nagi統合DiagnosticsとAI支援診断（2026-11-16）

[DEC-098](../DEC-098-diagnostics-linux-nagi-ai-assisted-staged.md)で、Linuxの診断／crash／service情報をNagi Diagnosticsで統合するB方式を基本にし、CのAI支援による原因候補の分析・根拠付き説明・対処提案を将来の正式製品目標として段階導入することをCONFIRMED。AI不在時の通常診断、Debug／Activityの分離、Redaction、User／Profile／Permission／Policy、観測事実とAI推測の区別、無断の修復・Cloud exportをしない境界を維持する。具体Linux収集backend・Log容量／Retention／export・AI方式・自動修復の採否・実装／受入・適用VersionはPROVISIONALで、0.2 M04と既存Release／Activation／Acceptance Gateを変更しない。

変更履歴: 2026-11-16 — DEC-098とFoundation #29のB製品基盤／C将来目標・未決事項を参照追加。

## Foundation #30：Nagi専用GUI Installer（2026-11-16）

[DEC-099](../DEC-099-installer-first-setup-linux-nagi-gui.md)でLinuxの既存disk／storage機構を利用するNagi専用GUI InstallerのB方式をCONFIRMED。初期OS／Owner／Locale等の導入操作とディスク破壊前の明示確認をユーザーが行えるようにし、Offline-first／AI非依存で基本セットアップを成立させる方針とする。ローカルLLMに高度な自動インストール判断を求めるCは今回採用しない。DEC-079のread-only System／User Data分離・A/B・Recoveryを継承し、対応機種・partition／暗号化方式・Recovery Key／UI・実機受入・提供版はPROVISIONAL。0.2 §16のPackage／Update契約とOSを物理ディスクへ導入する処理は混同せず、既存Release／Activation／Acceptance Gateは変更しない。

変更履歴: 2026-11-16 — DEC-099とFoundation #30のGUI＋Linux／Offline／明示確認・未決事項を参照追加。

## Foundation #31：System Update（2026-11-16）

[DEC-100](../DEC-100-system-update-linux-nagi-coordinated-ab.md)で、Linuxの既存更新機構＋Nagi統合Update Serviceの方式BをCONFIRMED。DEC-079の通常時read-only System／User Data分離、A/B更新・復旧、AI Modelの論理的分離を引き継ぎ、Nagiが署名・互換検証、Update状態管理、A/B切替と失敗時の復旧を統合する。System／App／Model更新は別のPolicy domainとし、builder Appの正式更新はDEC-035の別承認を維持。Linux採用backend、署名鍵・配布／Offline更新、slot・boot health／rollback、Data Migration、適用UI・自動化・対象端末／ReleaseとLinux実機受入はPROVISIONAL。方式Cの差分更新・段階配信・端末状況による自動最適化は今回未採用。既存Release／Activation／Acceptance Gateを変更せず、仕様記録は実装・実機成功の証拠ではない。

変更履歴: 2026-11-16 — DEC-100とFoundation #31のB統合Update方針、C未採用・Linux受入未確認を参照追加。

## Foundation #33：段階的Hardware Support（2026-11-16）

[DEC-102](../DEC-102-hardware-support-staged-multidevice-linux-drivers.md)で、Linuxの既存Driver／Hardware機構を活用し、PC／ARM64／Tablet等へ段階的に展開し、機種・機能別の実機検証を経て正式な製品対応範囲を判断するB方式をCONFIRMED。QEMU x86-64・x86-64 UEFI mini-PC・generic ARM64→Raspberry Piという0.2の開発・検証境界、DEC-052のRaspberry Pi（型番未選定）／PineTab2／Phone未選定の候補、DEC-078の端末別Boot、Desktop firstを維持する。Linux Driverの存在とNagi Linux実機での対応完了を同一視せず、Display／Input／Radio／Powerなどの未受入機能を対応済みとしない。具体機器・Driver／Firmware、必須Device Matrix・サポート保証・提供Version／実機結果はPROVISIONAL。既存Release／Activation／Acceptance Gate、Workstream許可と実装済み状態を変更しない。

変更履歴: 2026-11-16 — DEC-102でFoundation #33のLinux Driver活用と段階的マルチデバイス検証の製品方針を追加。

## Foundation #34：検証B先行と自動化・AI支援Cの将来目標（2026-11-16）

[DEC-103](../DEC-103-testing-release-evidence-baseline-ai-assisted-staged.md)で、Host→Nagi Linux Image／QEMU→対象実機の段階的・証拠に基づく検証Bを先に整備し、複数環境の自動テスト・障害注入・テスト不足分析・AIによる失敗原因調査等のCを将来の正式製品目標として段階導入する方針をCONFIRMED。Cは単なる研究候補ではない。AIの分析は原因仮説・補助であり、PASS／FAILは決定論的な試験結果と既存Acceptanceで評価する。未実行・部分成功・結果不明と実際の成功を区別し、Host成功をLinux Image／実機PASSへ読み替えない。

DEC-049の速度優先、コード不変時の環境・限界付き過去結果再利用、統合の節目でのLinux Image受入、短い証拠記録を維持する。0.1 M30、0.2 M21／Activation、0.3仕様化・Activation／Acceptance、0.4 Entry Gate等を保持し、全35項目完了やC完成を一律の新Gateにしない。具体試験網・AIモデル／テスト生成・ツール・費用・スケジュール・提供Version・Linux受入結果はPROVISIONAL。各Roadmapの版割当て・Workstream許可・実装状態を変更しない。

変更履歴: 2026-11-16 — DEC-103とFoundation #34のB先行／C正式将来目標・合否境界を参照追加。文書記録のみで新規runtime受入を示さない。

## 実機テスト候補（2026-11-16）

[DEC-052](../DEC-052-physical-test-device-candidates.md)で、PC＝Raspberry Pi（型番未指定）、タブレット＝PineTab2、スマートフォン／モバイル＝未選定という候補選定方針を記録した。CONFIRMEDは候補選定のみ。起動実績・正式対応・購入・実装時期・対応版は確定せず、各ロードマップの版境界と既存Gateは変更しない。[実機・複数端末の旧展望](../../roadmap/multi-device-outlook.md)の機種選定は同DECの範囲で置き換える。

## スリープ・復帰と段階的な休止状態（2026-11-16）

[DEC-068 — Nagi Sleep（仮称）](../DEC-068-nagi-sleep-resume-recovery.md)は、Linux機構と決定論的Serviceによるスリープ／復帰／画面ロック、必要な状態保存・接続再検証、AI Jobの安全な待機を記録する。RAM保持の通常復帰と障害後の保存状態復旧を区別し、DEC-014／022の保存範囲・選択式復旧・無断Task再実行禁止を維持する。Hibernateは対応実機での技術検証後に段階的に追加し、全端末対応を約束しない。0.5–1.0案の資源管理配置を含め、Release割当て・時期・Acceptance・Gateは確定・変更しない。

## Nagi Power（2026-11-16）

[DEC-067](../DEC-067-nagi-power-deterministic-resource-management.md)に、AI／LLM推論を必須としないOS全体の決定論的な電力・熱・負荷・背景資源管理を記録した。既存の前景優先・Model／Job／Memory資源制御と接続し、必要な低レイヤー基盤を広く整備する方向を確認した。具体的な全サブシステムの採用・実装着手・対応端末・効果・版割当て・Gateは確定しない。Master Roadmapの0.6配置は引き続きPROVISIONAL。

変更履歴: 2026-11-16 — Nagi Powerの決定を参照追加。各ロードマップの既存版境界とGateは変更しない。

## Nagi Handoff採用・変更履歴（2026-11-16）

[DEC-065](../DEC-065-nagi-handoff-work-continuation.md)でNagi Handoff（仮称）を、PC・タブレット・スマートフォン間で文書・Resource・編集箇所・意味的な作業状態を継続する純正OS機能として採用した。既存Continuity、Homeの `home.continue_from_device`、Device Capability、DEC-014の選択式Workspace移行／同期を最大限再利用し、受信先の画面・能力に応じてUIを再構築する。現行User／Profile／Owner／Permission／Privacy・Source有効性／Revisionを再検証し、実体がなければユーザー許可に基づく取得／転送経路または代替を提示する。無断のファイル・個人データ転送／クラウド同期・競合上書きをせず、中断AI Task／副作用Actionを自動再開・再実行しない。名称・UI・復元範囲・経路・共有方式・クラウド利用・瞬時性・対応版はPROVISIONAL。全アプリ・全状態の完全移行、常時オフライン成立、自動同期は保証しない。単発ファイル転送のNagi Shareとは別の概念である。既存Desktop first、Release／Activation／Acceptance Gate、版割当てを変更せず、実装済みを示さない。 0.3／0.4+／0.5〜1.0のどのVersionへ提供するかは本記録では確定しない。

## 端末横断の電話・連絡先・SMS（2026-11-16）

[DEC-063](../DEC-063-phone-contacts-messaging.md)は、電話・連絡先・SMS/Messagingの純正採用、共通アプリ／モデルと適応UI、可能な限りPeopleへの連絡先統合を確定する。通信・連携の実可用性はハードウェア／OS能力、ユーザー認可、必要なネットワーク・サービス・キャリアに依存し、全端末での通話／SMS送受信を保証しない。各ロードマップのProfile／Permission、単一ユーザーからの段階的拡張方針と整合させ、端末間の遠隔操作・引き継ぎ・同期、アプリ構成や実装版は未確定のままとする。既存の版割当て・Gate・Acceptanceを変更しない。

## PDF Reader / Editorの製品方針（2026-11-16）

[DEC-061](../DEC-061-first-party-pdf-reader-editor.md)で、Nagi純正PDF Reader兼Editorの採用と段階的拡張をCONFIRMEDとして記録した。高速表示・検索・ページ操作・注釈・印刷・フォーム入力などを先に整え、その後、既存本文／画像の高度編集・OCR・元内容の削除と検証を伴う墨消し・署名関連機能を段階的に検討する。通常閲覧・編集はAI非必須とし、Nagi Project / Smart Paste / Writer / Sheets / Slides等と共通Public APIで連携する。AIによる表抽出・要約・比較・引用は利用者の承認と元資料の権限・機密分類の範囲に限る。

具体的なリリース版・高度機能の順序・技術・合格値はPROVISIONAL。本ロードマップの対象版への割当て、Acrobat完全互換、全機能初期実装、実装済み・Acceptance通過を意味せず、既存Gateは変更しない。
