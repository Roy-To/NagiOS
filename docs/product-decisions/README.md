# NAGI Product Decisions — 決定済み仕様の保管庫

**目的:** チャット上で合意したNAGIの製品挙動・AIポリシー・ユーザー体験を、消えないMarkdownとして継続蓄積する。

**作成日:** 2026-10-25  
**管理場所:** `docs/product-decisions/`  
**対象:** 0.2以降の製品判断（実装前の要求を含む）

## Omarchy参考の追加選別：ローカルAIの簡易診断・バグ報告支援（2026-11-24）

[DEC-098](DEC-098-diagnostics-linux-nagi-ai-assisted-staged.md)と[DEC-119](DEC-119-user-initiated-problem-reports.md)に、**ローカルLLMは簡単な問題切分けとユーザーのバグ報告準備を助ける**高水準方向をCONFIRMEDで追加。高精度な根本原因の特定をローカルLLMの標準要件にしない。決定論的Diagnosticsの事実とAIの推測を区別し、既存の伏せ字・内容プレビュー／除外・本人送信・非公開脆弱性窓口・自動送信なしを維持する。詳細・技術・版はPROVISIONAL。

前回採用済みのWorkspace手動表示モードは[DEC-010](DEC-010-intent-first-adaptive-ui.md)で維持。Webアプリの独立したアプリ風起動は今回**新規採用しない（将来禁止の決定ではない）**。Omarchy参考の高度なインストール高速化は今回は**見送り**、[DEC-099](DEC-099-installer-first-setup-linux-nagi-gui.md)の既存GUI Installer方針を撤回しない。ワンキーAI呼出しの具体仕様は未決。既存Release／Activation／Acceptance Gateを変更せず、今回の記録は新たな実装着手許可を意味しない。

## Omarchy参考の3件：Nagi System Skills・AI稼働状況・Workspaceモード（2026-11-24）

[DEC-124 — Nagi System Skills](DEC-124-nagi-system-skills.md)、[DEC-125 — AI稼働状況パネル](DEC-125-ai-operations-status-panel.md)、[DEC-010 — Workspace手動表示モード](DEC-010-intent-first-adaptive-ui.md)の3案をユーザーが明示的に採用。高水準の製品方針は**CONFIRMED**、Skill Schema・対象表示項目・モード一覧・実装・版・Acceptanceは**PROVISIONAL**。Omarchyは参考事例であり、既存Distribution／Desktopや自由なシェル実行経路の採用ではない。正規Public Action／Permission、AIなしの通常GUI、DEC-123のLinux版Developer Previewと既存Release／Activation Gateを維持。

## Supervisor Councilの将来方針（2026-11-22）

[DEC-122 — 端末能力適応型Supervisor Councilと独立したOSセキュリティ](DEC-122-adaptive-supervisor-council-security.md)を追加。CouncilをローカルLLMネイティブな追加審査層として活用し、**重要Action・不審な指示へのセキュリティ審査を主目的**として日常的な回答の深掘りに常時合議しない方針、端末Resourceに応じたSupervisor数・運用の柔軟性、OS中核セキュリティの独立を高水準**CONFIRMED**として記録。高度な調査へのCloud LLM利用は選択肢とし、DEC-002の事前許可・Offline-firstを維持。重大／致命的変更の合議ルール、低スペックのCouncil省略または重大案件のみ直列3回推論、技術・提供版は**PROVISIONAL**。DEC-074の長期・後順位と0.3／初期0.4の既存Gateは維持。

## Implementation Readiness — 50項目の未決詳細棚卸し（2026-11-17）

[DEC-111：実装準備の未決詳細仕分けと条件付き判断](DEC-111-implementation-readiness-triage.md) により、Foundation 35項目＋PX 15項目の高水準方針の後続として、**実装上の障害になる判断だけを必要な工程で先に扱う**運用をCONFIRMED。一次分類は **E：技術担当が先行23件、V：比較・実測待ち5件、U：機能有効化／公開前の製品判断8件、R：詳細後回し14件**。区分と順序はREVIEW NOTE／PROVISIONALであり、各DECの高水準CONFIRMED、PX進捗、Foundationの推奨優先度、既存Release／Activation／Acceptance Gateや担当・実装着手権限を変えない。

**最初の扱い（2026-11-17）:** Linux最小起動・通常アプリの開発を、全50項目の詳細決定待ちで止めない。F04のInit A/Bは短期Linux比較後の最終判断、PX-05／08／09はAction・重要Memory・自動削除を有効化する前、PX-12／F16／PX-11／14／15は対象機能や1.0の正式な提供範囲を決める段階で必要な判断に絞る。分類は公開仕様の一次棚卸しであり、現行実装がBLOCKEDであることを示すものではない。

**変更履歴:** 2026-11-17 — 50項目横断のImplementation Readiness運用と[DEC-111](DEC-111-implementation-readiness-triage.md)の一次仕分けを追加。分類・判断順序は未確定の技術／製品採用を確定せず、Release Gateを追加しない。

## Product & Experienceの優先15項目（2026-11-17、仕様検討用）

[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md) に、[Foundation 35項目](NAGI_OS_FOUNDATION_CHECKLIST.md)とは別のPX-01〜PX-15（基本UI、AIとTask、Memory、純正アプリ連携、開発者体験、1.0提供範囲）を設けた。各項目は関連DECで**既にCONFIRMEDの方針を維持**し、追加詳細を別チャットで検討する入口。**PX-01〜PX-15の高水準製品方針はすべてCONFIRMED（詳細はPROVISIONAL、Checklist進捗はPARTIALLY_RECORDED）**。検討テーマの最初の順序は **REVIEW NOTE / PROVISIONAL**。既存のGate、35項目の状態・優先度・実装評価は変更しない。既存のGate、35項目の状態・優先度・実装評価は変更しない。

## PX-01〜PX-15の統合済み製品決定（2026-11-17）

**PX-01追加決定（2026-11-17）:** [DEC-009 — Nagi-first Home／DesktopとPX-01 B＋将来C](DEC-009-nagi-first-home-desktop.md)で、対話中心のシンプルな統合Home Bを基本に、将来はProject／Task等の状況に合う高度なHome適応Cへ発展する高水準方針をCONFIRMED。[DEC-010](DEC-010-intent-first-adaptive-ui.md)のIntent-first／新規Workspace自動配置と既存Workspace手動配置保護、[DEC-011](DEC-011-task-approval-progress-ui.md)のTask承認・進捗、[DEC-097](DEC-097-accessibility-foundation-and-ai-native-long-term-direction.md)の通常a11y先行とAI不在時GUIを維持。Homeからアプリ直接起動でDesktop切替、委任Task中は原則Home維持、前回Workspaceの復元、状態保持も従前どおり。具体レイアウト・導線・カード数・Nagi Bar位置・AI停止時の詳細UI・技術・提供版はPROVISIONAL。進捗は[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)のPARTIALLY_RECORDEDを参照。並行PXの独立PRは統合時に照合する。

**PX-02追加決定（2026-11-17）:** [DEC-010 — Intent-first UIとPX-02 B＋将来C](DEC-010-intent-first-adaptive-ui.md)で、作業目的に応じたカード／パネルをユーザーが選択・却下・固定・調整できる適応UI Bを基本とし、許可された作業文脈・明示設定・採用履歴に応じて将来の高度な個人適応UI Cへ発展する高水準の方向をCONFIRMED。[DEC-008](DEC-008-proactive-behavior.md)の控えめな提案・無断音声割り込み禁止・権限境界、既存の新規Workspace自動配置／複数ディスプレイ活用・既存Workspaceと固定／手動配置の保護、AI不在時の通常GUIを維持。カードの詳細UI・表示順序・適応条件・学習技術・性能・提供版はPROVISIONAL。[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)ではPARTIALLY_RECORDEDとする。並行PX PRの決定は統合時に調整する。

**PX-03追加決定（2026-11-17）:** [DEC-055 — Projectフォルダ中心の純正アプリ横断AI体験／PX-03](DEC-055-project-folder-cross-app-context.md)で、Project中心に関連資料・参照・Task・Workspace・作業継続候補を一元的に把握するBを基本とし、将来は許可されたProject文脈・出典関係・履歴から根拠付きに作業の再発見と継続を支援するCへ発展する高水準方針をCONFIRMED。[DEC-012](DEC-012-workspace-naming-organization.md)／[DEC-014](DEC-014-workspace-state-checkpoints.md)のWorkspace保存・自然言語検索・本人による復元先選択／復元前保護、[DEC-065](DEC-065-nagi-handoff-work-continuation.md)の別端末Handoff境界、Project／Workspace・原本文・Memoryの分離と現行Permission／Privacyを維持。統合作業ビューの詳細UI、関連付け・一覧・検索・復元導線・技術・性能・提供版はPROVISIONAL。[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)はPX-03をPARTIALLY_RECORDEDとして記録。並行PXの未統合PRは統合時に各決定を保持する。

**PX-04決定（2026-11-17）:** [DEC-008](DEC-008-proactive-behavior.md) と [DEC-011](DEC-011-task-approval-progress-ui.md) に、B「明示操作で始める複数ターンの音声会話、同じ対話内の音声／テキスト切替、画面での誤認識訂正」と将来C「ユーザーが明示的に有効化するハンズフリー音声操作へ段階的に拡張する」高水準方向をCONFIRMEDとして記録。Wake Word、常時待受、録音／文字起こしの保持・削除、詳細UI、音声本人認証、技術・モデル・提供版・時期・性能数値はPROVISIONAL。画面最終承認、Microphone Permission、DEC-002のCloud送信別許可、音声割り込み抑制、停止・Text/UI fallback・Task状態保持を維持する。進捗は[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)の**PARTIALLY_RECORDED**を参照。

**PX-05追加決定（2026-11-17）:** [DEC-001 — AIの自律性とAction・PX-05 B＋将来C](DEC-001-agent-autonomy-actions.md)に、段階的なリスク対応承認Bを基本とし、将来はTask文脈・利用者設定・検証状態を踏まえて確認体験を高度化するCへ発展させる方針をCONFIRMEDとして追記。[DEC-026](DEC-026-ai-change-verification-and-recovery.md)の「変更の検証／復元可能性を優先する選択A」は維持し、Risk taxonomy・閾値・未判定時の条件・UI・技術・版はPROVISIONAL。更新範囲と進捗は[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)を参照。並行PXの未統合PRを今回の決定と混同しない。

**PX-06追加決定（2026-11-17）:** [DEC-011 — Task進捗表示とPX-06統合TaskセンターB＋将来C](DEC-011-task-approval-progress-ui.md)で、長時間・複数Taskを一元的に把握・管理できるB案を基本とし、将来CのTask依存・資源・優先度等に応じた高度なオーケストレーションへ発展する高水準方針をCONFIRMED。[DEC-020](DEC-020-ai-os-interaction-path.md)の前景操作優先・明示的優先度変更・条件付きResource継続、[DEC-022](DEC-022-builder-interrupted-task-recovery.md)の障害後の本人選択、既存Gateを維持する。Queue／Priority・状態表示の細部、依存関係・Scheduler・並列度、実装技術・提供版はPROVISIONAL。進捗は[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)の**PARTIALLY_RECORDED**を参照。並行PXの独立PRは未統合の場合があるため、統合時に矛盾を解消する。

**PX-07追加決定（2026-11-17）:** [DEC-008 — 標準的な働きかけとPX-07 B＋将来C](DEC-008-proactive-behavior.md)に、許可された文脈・好みに合わせて作業を妨げず自然に提案するBを基本とし、将来は複数の改善候補の整理・根拠／期待効果の比較・Feedback活用を進めるCを高水準方針としてCONFIRMED。[DEC-073](DEC-073-proactive-friction-discovery.md)の根拠付き潜在的不便発見、静かな提案、重大警告の別扱い、Privacy／Permission／実行承認を維持する。頻度・タイミング・抑制・却下／再提示・重要度判定・詳細UI・技術・版はPROVISIONAL。進捗は[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)のPARTIALLY_RECORDED。並行PXの未統合PRは統合時に照合する。

**PX-08決定（2026-11-17）:** [DEC-004 — Memory保存・学習・自然忘却・PX-08 B基本方針](DEC-004-memory-learning-forgetting.md)に、許可された情報から自然に学ぶ、推測は推測のまま扱う、重要な内容は本人確認するというバランス型をCONFIRMEDとして追加。対象情報の細分類・確認条件／UI・閾値・実装詳細はPROVISIONAL。既存DEC-003／005／058／059および各Gateを変更しない。進捗は[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)の**PARTIALLY_RECORDED**を参照。

**PX-09決定（2026-11-17）:** [DEC-004 — Memoryの保持・忘却・管理体験](DEC-004-memory-learning-forgetting.md)にB案「自然忘却＋ユーザー管理」を基本方針としてCONFIRMEDで記録し、将来C案の高度な重複統合・矛盾検出を追加する方向もCONFIRMEDとした。保持条件・日数・UI・Cの具体方式・提供版／時期はPROVISIONAL。既存DEC-003／005／059、Source／Profile／Permission境界と既存Gateを維持する。進捗は[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)の**PARTIALLY_RECORDED**を参照。

**PX-10追加決定（2026-11-17）:** [DEC-057 — 成長するPersonal Agentと対話スタイル](DEC-057-growing-personal-agent-os.md)に、利用者が許可した学習・設定に基づく自然な口調／説明量／説明方法の適応Bと、将来のProject／Task／作業状況に応じた高度な状況適応Cを高水準CONFIRMEDとして記録。具体UI・アルゴリズム・モデル・評価・版はPROVISIONAL。既存Profile／Memory所有・訂正／忘却・明示設定優先を維持。

**PX-11追加決定（2026-11-17）:** [DEC-021 — 1.0ロードマップ境界とPX-11品質重視の純正アプリ構成](DEC-021-roadmap-0.5-1.0.md)で、**日常の主要作業が純正アプリと通常GUIで実用的に完結する品質優先B**を基本とし、**将来Cで幅広い総合純正ソフトウェア環境へ発展**する高水準の方針をCONFIRMED。[純正アプリ実装仕様](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)のPhase 1〜3、[DEC-061](DEC-061-first-party-pdf-reader-editor.md)のPDF Reader／Editor、[DEC-062](DEC-062-first-party-utilities.md)のユーティリティ4分野、DEC-017の1.0公開開発者環境・アプリ配布基盤目標とAI非必須操作を維持する。**個別アプリの1.0必須／任意／後続リスト、最小機能、対応形式、性能、技術、搭載／配布方式と提供版はPROVISIONAL**。[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)はPX-11 PARTIALLY_RECORDED。並行PXの未統合PRは統合時に別途調整する。

**PX-12追加決定（2026-11-17）:** [DEC-056 — Smart PasteとPX-12のB＋将来C](DEC-056-smart-paste-semantic-transfer.md)に、利用可能なコピー／構造化転送／参照とその差異・制限をユーザーに分かりやすく案内する統合体験B、将来は許可されたProject／Task・Provenance・変更影響を扱う高度なアプリ横断ワークフローCへ発展する高水準方針をCONFIRMED。[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)の共通参照A・更新検知／提案A・承認済みTaskのScope内限定更新、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)の正規Public Action優先A・明示許可下の限定GUI fallback、DEC-056のSheets→Slides／Albert→Writer／画像→Slidesと従来操作を維持。最初のApp組合せ、コピー／参照の既定、詳細UI・Preview・Transfer／fallback制御・技術・性能・提供版はPROVISIONAL。[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)はPX-12 PARTIALLY_RECORDED。未統合の他PXの決定は統合時に照合する。

**PX-13後続決定（2026-11-17）:** [DEC-071 — 初心者向けNagi Native App制作／PX-13](DEC-071-nagi-native-app-creation-experience.md)で、将来の**対話型共同制作B＋個人適応・継続開発C**を高水準CONFIRMEDとしつつ、**当面はローカル完結の独立した高度制作環境の完成を急がず、ローカルLLMが既存IDE／人間・許可されたCloud開発AI（例：GPT／Codex等）とNagi SDK・API・Porting Toolkitの診断／移植結果を安全につなぐ支援を優先**する方針を記録。[DEC-045](DEC-045-nagi-porting-toolkit.md)のWindows／Linuxからの移植支援と非AI診断・検証、[DEC-070](DEC-070-memory-informed-native-app-creation.md)の本人同意に基づくMemory個人化、[DEC-002](DEC-002-cloud-ai-data-sharing.md)のCloud送信事前許可、DEC-075の試用／正式導入の独立承認を維持。高度統合制作環境は**1.0計画最終盤を目安とする長期・低優先**であり提供版・完成はPROVISIONAL。[DEC-017](DEC-017-third-party-developer-ecosystem.md)／[DEC-021](DEC-021-roadmap-0.5-1.0.md)の**1.0公開開発者環境・アプリ配布基盤という既存目標は維持**。[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)ではPX-13 PARTIALLY_RECORDEDとし、並行PXの未統合PRは統合時に照合。

**PX-14追加決定（2026-11-17）:** [DEC-017 — 第三者アプリ開発／移植／配布とPX-14 B＋将来C](DEC-017-third-party-developer-ecosystem.md)で、**一般的なIDE・公開SDK／Porting Toolkit（PT-01〜06）・Local LLMと許可されたCloud開発AIの橋渡し・Build／Test／Package／独立Target受入を一貫した開発者体験にするB**と、**将来は互換性・更新・保守・配布準備まで支援するC**の高水準方針をCONFIRMED。[DEC-045](DEC-045-nagi-porting-toolkit.md)のPT・非AI基盤、[DEC-002](DEC-002-cloud-ai-data-sharing.md)のCloud送信事前許可、Nagi Native専用・Rust SDK／C ABI・Level 0〜3とWindows／Linux重点・Android将来／Apple系後順位を維持。PX-13の高機能な独立開発IDE後順位、[DEC-021](DEC-021-roadmap-0.5-1.0.md)の**1.0公開開発者環境／配布基盤**目標、Package署名／導入／共有・公開の別承認とHost／Target結果分離を維持。初期言語・Framework・SDK／CLI／Cloud／署名・配布実装・性能・版はPROVISIONAL。[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)ではPX-14 PARTIALLY_RECORDED。並行PRは統合時に確定内容を保持する。

**PX-15追加決定（2026-11-17）:** [DEC-021 — Nagi OS 1.0の完成方針B＋将来C](DEC-021-roadmap-0.5-1.0.md)に、**正式対応のTarget／機能で日常の代表作業を安全・安定に完結・継続できる実用OS B**を1.0の高水準完成基準とし、将来は対応機器・アプリ・利用形態を広げた**総合OS C**へ発展する方針をCONFIRMED。[DEC-074](DEC-074-human-led-agent-organization.md)のAI組み込み体験優先・階層型Agent組織後順位、[DEC-017](DEC-017-third-party-developer-ecosystem.md)の1.0公開開発者環境／アプリ配布基盤目標、通常GUI・AI不在時操作・Permission／Privacy・Target実受入と既存Release／Activation Gateを維持。**代表作業・必須アプリ・正式対応PC／Hardware・性能／品質の閾値・具体的公開判定手順と日程はPROVISIONAL**。[0.5〜1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)に高水準完成方針を補足し、版別マップと§6の候補証拠はそのまま候補として維持。[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)はPX-15をPARTIALLY_RECORDEDと記録。未統合の他PX確定PRは統合時に保持する。

**統合の解釈:** 各PXでCONFIRMEDなのは明示採用された高水準の方向のみ。各DEC内のPROVISIONAL／OPEN、個別の仕様・性能・対象端末／Version、既存Permission／PrivacyとRelease／Activation／Acceptance Gate、Linux／Targetの実装証拠は変更しない。[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)へ15件を一覧化した。

## Foundation統合とDEC改番（2026-11-16）

PR #133〜#140のFoundation #6〜#35に関する方針を、先行する#1〜#5の決定と共存させる。並行branchで重複したIDは以下のように分離し、内容・索引・関連参照を維持する。DEC-094〜104とFoundationの`f25`〜`f35`は改番しない。

| 元のPR・ID | 統合後の正本ID | 対象 |
|---|---|---|
| #133 DEC-082〜087 | DEC-082〜087（維持） | 電源ライフサイクル・Identity・Permission・IPC・Secrets・Trust |
| #135 DEC-082 | [DEC-105](DEC-105-user-environment-backup-and-wayback.md) | #16 Backup / Restore |
| #136 DEC-083〜087 | DEC-106〜110 | #17 Device Manager・#18 Network Devices・#19 Display / GPU・#20 Input・#21 Audioの順 |
| #136 DEC-092 | DEC-092（維持） | #22 周辺機器 |
| #139 DEC-094〜103／#140 DEC-104 | DEC-094〜104（維持） | #25〜#35 |

日付付き本文の「main未マージ」「別PR側の記録」とFoundation概要表の初回評価は、記録当時の履歴として保持する。現在の製品方針は本索引の正本DECと[Foundation各詳細欄](NAGI_OS_FOUNDATION_CHECKLIST.md)を参照する。全35項目の方針検討と技術選定・実装・Linux image／実機受入を区別し、既存Release／Activation／Acceptance Gate、担当・開始権限は変更しない。#4 InitのA/B正式選択など、各DECの未確定事項も継続する。

## 正本と記録のルール(2026-11-15、[DEC-050](DEC-050-public-specification-repository.md))

**相談はチャット、決定済み仕様の正本はこの公開リポジトリ `Roy-To/NagiOS`。** 詳細は [OPERATING_RULES.md](OPERATING_RULES.md)。2026-11-15に、それまで非公開リポジトリ `Roy-To/NagiOS-internal` に保存していた仕様・構想をここへ移した。各文書に残る「Privateに保存」「Publicへ転載しない」は移管前の記述で、DEC-050が優先する。

**2026-11-16追加決定:** 公開 `Roy-To/NagiOS` はコード・決定済み仕様・必要なCIを維持し、Private `Roy-To/nagi-checkpoints` はEntireの会話履歴・関連メタデータ専用、GitHub Actions無効とする。旧 `NagiOS-internal` への新規記録は再開しない。設定・検証の実施範囲と残件は[Entire運用記録](../development/entire-private-history.md)を参照し、方針の記録を保存試験の成功と扱わない。

- [ChatGPTプロジェクト指示(登録用)](CHATGPT_PROJECT_INSTRUCTIONS.md) — ChatGPTのNagi_chatプロジェクトの指示へコピーして適用。**ファイル保存だけでは設定は変わらない。**
- [Codex開始・引き継ぎ規則](CODEX_START_HERE.md) — このリポジトリで動くCodexが決定事項を参照・記録する手順。
- [0.3改訂・0.4+構想・0.5〜1.0 Master Roadmap](roadmaps/README.md) — ロードマップ案。

ChatGPT・Codexとの会話で仕様が決まったら、`CONFIRMED`か`PROVISIONAL`かを分け、該当DECファイルと索引を更新して**このリポジトリにcommitし、確認できた場合だけ「保存済み」**と報告する。認証情報・個人情報・ユーザー名入りのパス・チャット識別子は書かない。

## 8GB RAM級での軽快な純正アプリと実用的ローカルAI連携（2026-11-16）

[DEC-020 — AIのOS操作経路と計算資源](DEC-020-ai-os-interaction-path.md)へ、RAM 8GB級の限られた端末でも純正アプリの日常操作と必要十分な軽量・量子化ローカルLLMによるアプリ間AI連携が**実用的に共存することを目指す**製品目標を追記。単なる起動確認ではなく前景UIの応答性とAI Taskの品質を実測する方針を再確認した。モデル選択・入替えとNagiのMemory継続、AI不在時の通常操作、Permissionと既存Release／Activation境界を維持する。8GBでの快適動作は**未実証**で、代表端末・実測数値・モデル構成・適用版はPROVISIONAL。本記録は性能保証でも実装完了でもない。

複数の量子化モデルをストレージに保存し、必要時にのみメモリへロードして、利用可能な標準・軽量モデルを切り替えられる方向も**CONFIRMED**。負荷・Taskの必要能力に応じた**自動的な軽量モデルへの切替を目指し、初期に難しい場合も手動切替を確保する**。ユーザーが明示固定したモデル・自動切替の無効設定、Task品質、権限・明示停止、Nagi Memoryの継続を尊重する。自動化の採用時期、RAM閾値、切替中断や再開、完全に切れ目のない応答、適用版は**PROVISIONAL**であり、8GBでの達成性能は未実証。

## 8GB RAM級性能試験の先行開始（2026-11-17）

[DEC-020](DEC-020-ai-os-interaction-path.md)で、0.3完成を待たずに、8GB RAM級のNagi通常操作とローカルAI連携の実用性を段階的に試験することをCONFIRMED。OS／純正アプリの前景応答性とAI Task速度・品質を分けて評価し、host-only／Linux image・QEMU／実機を区別する。端末・モデル・担当・数値閾値・日程・Release割当てはPROVISIONAL。未実証の快適さを保証せず、既存Gateを変更しない。

## 学習履歴の可視化（2026-11-16）

[DEC-059 — Nagi Insights（仮称）](DEC-059-learning-history-insights.md)で、ユーザーがNagiの学習内容・経緯・根拠・支援への反映を分かりやすく閲覧する体験をCONFIRMEDとした。DEC-003のMemory可視化と訂正・削除、DEC-004の自然・明示的忘却、DEC-005のSource連動を継承し、学習履歴を全操作の無制限な保存・自動学習の口実としない。表示方式・指標・実装版はPROVISIONALであり、既存Release／Activation Gateを変更しない。

## 中核コンセプト — 成長するPersonal Agent OS（2026-11-16）

[DEC-057 — 学習・適応・継続によって成長するPersonal Agent OS](DEC-057-growing-personal-agent-os.md)により、**使い続けるほどユーザーの目的・好み・習慣を理解し、許可された経験とフィードバックを実用的な作業支援に反映するOS**をNagiの中核コンセプトとしてCONFIRMEDとした。記憶・学習・継続性はモデル非依存かつユーザー所有で、訂正・自然忘却・明示忘却・Permissionの既存規則を維持する。0.3のPersonal Agent OS方針を全体の製品アイデンティティとして明確化するものであり、実装済み・性能達成・Release Gateの変更を意味しない。具体的な学習評価や適応方式・提供時期はPROVISIONAL。

## 学習の主経路：自然な習慣学習（2026-11-16）

[DEC-058 — 習慣学習を主軸とするPersonal Agent](DEC-058-habit-first-learning-and-optional-teach-mode.md)で、ユーザーが日々Nagiを使うことで、許可された作業の反復・採用・修正・明示指示を元に習慣を学ぶ経路を**中心**とすることをCONFIRMEDとした。**Teach Modeは任意の補助機能**であり、習慣学習の前提条件にはしない。Teachと自然言語・GUIによるWorkflow草案も同じ承認・Permission・Activity境界を維持する。Teachの具体的な記録UI・技術・提供版はPROVISIONALとし、既存Release Gateを変更しない。

## カーネル方針の確定変更（2026-11-12）

[DEC-042 — Linuxカーネルへの方針転換](DEC-042-linux-kernel-transition.md)を参照。**独自Nagi KernelからLinuxカーネルへ変更する方針はCONFIRMED**。DEC-017 / 020の独自カーネル選択前提を限定置換する。追加確定の[DEC-043 — Nagi専用ユーザーランドと独立OS方針](DEC-043-nagi-userland-independent-os.md)により、**Linuxカーネル + Nagi専用ユーザーランド／既存ディストリビューション基盤非採用／個別OSS選択的利用可能／Android・ChromeOSに対抗する独立OS目標はCONFIRMED**。具体的な基本ユーザーランド構成・互換方式・Package形式・移行時期・ライセンス詳細はPROVISIONAL。現行AGENTS / ADR / 実装仕様との不整合と移行手順はDEC-042に記録し、既存0.1等のGateは変更しない。Publicへの転載・実装開始・移行完了を意味しない。

## このフォルダの使い方

- **原則、1テーマ1ファイル。** 会話で方針が固まったら対象ファイルを追加・更新する。1冊の巨大な仕様書に追記し続けない。
- 各ファイルには **決定事項／未確定事項／適用バージョン／関連文書／変更履歴** を区別して記録する。
- 状態は `CONFIRMED`（ユーザー合意済み）、`PROVISIONAL`（暫定値・提案）、`SUPERSEDED`（後の決定に置換）を使う。**「決定済み」の内容だけを確定仕様として扱う。**
- 決定を変更するときは旧内容を無言で上書きせず、変更日と理由を追記する。
- ChatGPTの記憶、別チャットの文脈、ローカルの一時出力を唯一の正本にしない。Gitのcommitで履歴を残す。
- ここへの記載は**機能が実装済みであることや、release / workstream activationを許可することを意味しない**。
- 秘密鍵・パスワード・個人情報そのものは、仕様の例としても書き込まない。

## DECとは別の長期ビジョン

**当面の優先順位（2026-11-16、[DEC-074](DEC-074-human-led-agent-organization.md)）:** AIがOS標準機能として自然に働くAI組み込み体験型OSの確立を優先する。階層型Agent Organizationは非優先の長期ビジョン。具体的な提供版・階層数・実装順序はPROVISIONALとし、既存0.3／0.4 Gateと初期builder Role方式を維持する。

以下は高水準の方向を記録する長期ビジョンであり、具体的な技術・実装時期を定める仕様ではない。個別の確定済み製品挙動は該当DECを正本とする。VISION-001の潜在的不便の発見方針は[DEC-073](DEC-073-proactive-friction-discovery.md)でCONFIRMEDとして記録する。

| ID | 文書 | 位置付け |
|---|---|---|
| VISION-001 | [人間に合わせるコンピューターを目指すNagi OS](VISION-001-human-adaptive-os.md) | 長期ビジョン。確定した不便発見方針はDEC-073、優先順位と組織方向は[DEC-074](DEC-074-human-led-agent-organization.md)を参照。具体的な技術・端末・性能・実装時期は未確定 |

## ファイル一覧

| ID | テーマ | 状態 | 最終決定 |
|---|---|---|---|
| DEC-001 | [AIの自律性とAction／PX-05 B＋将来C承認体験](DEC-001-agent-autonomy-actions.md) | CONFIRMED（既存自律実行・安全境界／PX-05のB基本＋将来C方向）／PROVISIONAL（Risk分類・閾値・未判定時の条件・UI・実装・版） | 2026-11-17 |
| DEC-002 | [クラウドAI・外部情報共有](DEC-002-cloud-ai-data-sharing.md) | CONFIRMED | 2026-10-25 |
| DEC-003 | [Memoryの機密分類と所有範囲](DEC-003-memory-classification.md) | CONFIRMED | 2026-10-25 |
| DEC-004 | [Memory保存・学習・自然忘却／PX-08・PX-09基本方針](DEC-004-memory-learning-forgetting.md) | CONFIRMED（既存Memory lifecycle、PX-08 B、PX-09 B、将来Cの方向）／PROVISIONAL（条件・UI・数値・Cの具体方式／版／時期） | 2026-11-17 |
| DEC-005 | [Files・AlbertとMemoryの連動](DEC-005-files-albert-memory-lifecycle.md) | CONFIRMED（一部パラメータは暫定） | 2026-10-25 |
| DEC-006 | [日常利用シナリオ](DEC-006-daily-use-scenarios.md) | CONFIRMED | 2026-10-25 |
| DEC-007 | [ロードマップ・版境界との整合](DEC-007-roadmap-alignment.md) | REVIEW NOTE | 2026-10-25 |
| DEC-008 | [控えめな働きかけ／小さな用事の静かな準備・学習・Reminder／PX-02・PX-04・PX-07](DEC-008-proactive-behavior.md) | CONFIRMED（既存の控えめな通常提案・客観的重大警告、PX-02適応UIとの整合、PX-04音声／Text基本B＋将来ハンズフリーC、PX-07文脈提案B＋将来高度管理C、小さな用事の静かな準備・会話からの一時記録・状況／期限型Reminder・確証による完了／重要度の学習と不要な催促抑制）／PROVISIONAL（頻度・UI・Wake Word・録音／Transcript・AI技術・性能・版） | 2026-11-17 |
| DEC-009 | [Nagi-first操作思想・Nagi Home / Desktop／PX-01統合Home B＋将来適応C](DEC-009-nagi-first-home-desktop.md) | CONFIRMED（既存Home／Desktop遷移・状態保持・AI不在時通常GUI、PX-01 B基本＋将来C方向）／PROVISIONAL（画面配置・導線・カード数・Nagi Bar位置・AI停止時UI・技術・提供版） | 2026-11-17 |
| DEC-010 | [Intent-first／Workspace配置・PX-01／02適応UI・手動モード切替](DEC-010-intent-first-adaptive-ui.md) | CONFIRMED（Intent-first／新規Workspace自動配置・既存手動配置保護・PX-01／02とWorkspace表示モードのユーザー手動切替）／PROVISIONAL（詳細モード・UI・技術・性能・提供版） | 2026-11-24 |
| DEC-011 | [Task承認・進捗・PX-04音声／TextとPX-06統合Taskセンター](DEC-011-task-approval-progress-ui.md) | CONFIRMED（従前の承認・進捗・Role／Stop／復旧、PX-04音声／Text切替・画面訂正、PX-06統合TaskセンターB＋将来高度Task管理C）／PROVISIONAL（具体UI・Queue／Priority／Scheduler・Voice UI・技術・性能・版） | 2026-11-17 |
| DEC-012 | [Workspaceの自動命名・整理と自然言語検索／PX-03との整合](DEC-012-workspace-naming-organization.md) | CONFIRMED（既存の自動命名・分類・意味検索／ユーザー選択優先とPX-03統合作業ビューの高水準接続）／PROVISIONAL（具体的なProjectとの一覧・検索／再開UI・技術・版） | 2026-11-17 |
| DEC-013 | [Hardware-Adaptive Memory Engineの先行設計と性能評価](DEC-013-lightweight-memory-engine-benchmark.md) | CONFIRMED（適応方針・評価方針）／profile・技術選択PROVISIONAL | 2026-11-05 |
| DEC-014 | [Workspace自動保存・Checkpoint・履歴・復元／端末移行・同期／PX-03との整合](DEC-014-workspace-state-checkpoints.md) | CONFIRMED（既存Workspace自動保存・履歴・復元本人選択・現行権限／Handoff分離、PX-03の高水準接続）／PROVISIONAL（Projectビューからの一覧・検索／復元導線・技術・性能・版） | 2026-11-17 |
| DEC-015 | [`builder` Roleの初期Nagi実行・切替・別会話継続・追加依頼分類](DEC-015-builder-role-review.md) | CONFIRMED（Role名／初期実行C／自動切替C／会話継続C／追加・別件依頼C）／PROVISIONAL（Runtime・Queue詳細） | 2026-11-05 |
| DEC-016 | [`builder`のScope内自律実行・リスク別完了・追加指示の承認境界](DEC-016-builder-autonomy-risk-based-completion.md) | CONFIRMED（自律B／完了判定C／再試行C／追加指示分類Cと承認・Scope制限）／PROVISIONAL（実装詳細） | 2026-11-05 |
| DEC-017 | [Nagi Native開発・移植・公開SDK／配布基盤とPX-13／14／15](DEC-017-third-party-developer-ecosystem.md) | CONFIRMED（従前のNative専用・既存IDE・SDK／移植支援・1.0開発者／配布目標、PX-13 Cloud AI橋渡し・統合IDE後順位、PX-14開発・移植／検証B＋将来C、PX-15実用1.0 B＋将来総合OS C）／PROVISIONAL（初期SDK・Cloud／CLI・署名／配布・性能・Release範囲／日程） | 2026-11-17 |
| DEC-018 | [`builder`の隔離編集と条件付き自動反映](DEC-018-builder-isolated-edit-conditional-apply.md) | CONFIRMED（隔離編集C、正式アプリ更新は別承認・DEC-035）／PROVISIONAL（実装詳細） | 2026-11-05 |
| DEC-019 | [`builder`作業データの状態別整理](DEC-019-builder-work-data-cleanup.md) | CONFIRMED（整理C・初期Nagi Role切替）／PROVISIONAL（保持期間等） | 2026-11-05 |
| DEC-020 | [AIのOS操作経路・適応型推論と人間優先の資源配分／PX-06の整合](DEC-020-ai-os-interaction-path.md) | CONFIRMED（既存Context／Event・前景優先・適応推論・8GB級の実用性目標・モデル切替方針・`builder` Resource待機継続C、PX-06との整合、0.3完成前の8GB級先行性能評価開始）／PROVISIONAL（Queue／Priority／Scheduler・並列度の詳細、実装・性能・適用版） | 2026-11-17 |
| DEC-021 | [0.5〜1.0ロードマップ・PX-11／13／14／15の高水準完成方向](DEC-021-roadmap-0.5-1.0.md) | CONFIRMED（既存一般公開品質・開発者／配布目標、PX-11品質重視純正アプリB＋将来C、PX-13 Cloud橋渡し優先・高度IDE後順位、PX-14開発者／移植B＋将来C、PX-15実用1.0 B＋将来総合OS C）／PROVISIONAL（必須アプリ・Target／Hardware・数値・具体技術／公開手順・提供版／順序） | 2026-11-17 |
| DEC-022 | [`builder`の中断・再起動後の作業復旧](DEC-022-builder-interrupted-task-recovery.md) | CONFIRMED（復旧C、資源待機とクラッシュ復旧の区別、初期Nagi Role切替）／PROVISIONAL（技術詳細） | 2026-11-05 |
| DEC-023 | [作業・データ中心のOS基本思想](DEC-023-work-data-centric-os.md) | CONFIRMED（作業中心・従来操作・Permissionの維持）／PROVISIONAL（連携方式・適用版） | 2026-11-05 |
| DEC-024 | [TypeSafe AI Jev名称の訂正と`builder` Role・DecisionProvider方針](DEC-024-external-jev-reference-and-builder-label-correction.md) | CONFIRMED（名称訂正、Role切替C、DecisionProvider優先）／PROVISIONAL（将来Agent・専用判断モデル） | 2026-11-05 |
| DEC-025 | [アプリ横断の共通参照・更新提案／PX-12との整合](DEC-025-cross-app-data-reference-and-updates.md) | CONFIRMED（共通参照・分散正本A、変更検知／更新提案A、事前承認Taskの限定自動更新、PX-12高水準体験と整合）／PROVISIONAL（詳細Transfer・更新UI・技術・性能・版） | 2026-11-17 |
| DEC-026 | [AIによる変更の検証可能性と復元可能性／PX-05との整合](DEC-026-ai-change-verification-and-recovery.md) | CONFIRMED（既存の思想⑨・選択A／リスク別検証・Wayback事前表示、PX-05 B＋将来C方針との両立）／PROVISIONAL（Risk taxonomy・未判定時の条件・UI・数値・実装・版） | 2026-11-17 |
| DEC-027 | [正規Public Action優先・限定GUI fallback／PX-12との整合](DEC-027-public-action-and-scoped-gui-fallback.md) | CONFIRMED（Public Action優先A、明示許可・限定ScopeのGUI fallback、PX-12高水準体験と整合）／PROVISIONAL（具体GUI／fallback制御・対象App・性能・適用版） | 2026-11-17 |
| DEC-028 | [反復Workflowの学習・改善案・承認制改訂](DEC-028-deterministic-workflows-from-habits.md) | CONFIRMED（Workflow化と正式改訂に明示承認・履歴保持）／PROVISIONAL（変換・検証・UI・保持詳細） | 2026-11-05 |
| DEC-029 | [明示許可に基づくScheduled / Event-triggered Workflow実行](DEC-029-scheduled-event-triggered-workflows.md) | CONFIRMED（条件付き無人起動・実行時再検証・停止制御）／PROVISIONAL（実装詳細） | 2026-11-05 |
| DEC-030 | [Workflowテンプレートの移植・共有と権限の再承認](DEC-030-workflow-template-portability-and-sharing.md) | CONFIRMED（個人状態・権限から分離、明示的共有、受取側で再設定・再承認）／PROVISIONAL（形式・検証・依存解決・配布経路） | 2026-11-05 |
| DEC-031 | [外部Workflowの未信頼検査と固定版更新管理](DEC-031-external-workflow-validation-and-version-updates.md) | CONFIRMED（⑮：段階分離・未信頼検査・隔離事前検証／⑯：固定版・許可制更新確認・明示承認）／PROVISIONAL（検査・更新方式・適用版） | 2026-11-05 |
| DEC-032 | [Workflowの安全な有限再試行と失敗時の復旧](DEC-032-workflow-safe-retry-and-recovery.md) | CONFIRMED（安全な有限Retry・危険／不明時の停止と確認）／PROVISIONAL（Retry条件・上限・実装・適用版） | 2026-11-05 |
| DEC-033 | [自然言語とGUIによるWorkflowの新規作成](DEC-033-natural-language-workflow-authoring.md) | CONFIRMED（自然言語草案・構造化確認編集・承認後保存・GUI直接作成）／PROVISIONAL（Schema・UI・検証・適用版） | 2026-11-05 |
| DEC-034 | [人間の作業工程・他ユーザー割当を組み込むHuman-in-the-loop Workflow](DEC-034-human-in-the-loop-workflow-steps.md) | CONFIRMED（⑲：Human Step・待機／再検証、⑳：他ユーザーへの範囲限定割当・Identity／Permission確認）／PROVISIONAL（通知・受諾・代行・UI・適用版） | 2026-11-05 |
| DEC-035 | [`builder`製アプリの保守・検証付き更新](DEC-035-builder-created-app-maintenance-updates.md) | CONFIRMED（Nagi提案・隔離修正検証・ユーザー承認後の正式更新C／データ保護と実際の復旧範囲）／PROVISIONAL（検知・Package更新機構・適用版） | 2026-11-05 |
| DEC-036 | [`builder`製アプリの個人利用・共有・公開](DEC-036-builder-app-sharing-and-publication.md) | CONFIRMED（個人利用を既定とする共有・公開方式C／明示的な範囲選択・内容確認・別個の送信承認・受取側での独立検証と再承認）／PROVISIONAL（形式・検証手段・適用版） | 2026-11-05 |
| DEC-037 | [Human Stepの差し戻し・再提出・期限管理](DEC-037-human-step-revision-deadlines-reminders.md) | CONFIRMED（㉑：差し戻し・修正・再提出と承認済みScope／㉒：期限・控えめなリマインド・明示許可先へのエスカレーション）／PROVISIONAL（状態・頻度・経路・UI・適用版） | 2026-11-05 |
| DEC-038 | [WorkflowのAI判断Stepとモデル非依存の入出力契約](DEC-038-ai-judgment-step-and-model-portability.md) | CONFIRMED（㉓：制約付きAI判断Step／㉔：モデル非依存・切替検証）／PROVISIONAL（品質閾値・モデル選択・UI・適用版） | 2026-11-05 |
| DEC-039 | [WorkflowのDry Run・事前シミュレーション・段階的検証](DEC-039-workflow-dry-run-and-validation.md) | CONFIRMED（㉕：副作用なしの事前検査・未検証の明示・本番Action別承認）／PROVISIONAL（隔離・Mock・受入条件・適用版） | 2026-11-05 |
| DEC-040 | [`builder`製アプリの開発Project長期保存](DEC-040-builder-app-development-project-retention.md) | CONFIRMED（ローカル基本の長期保持・保護・分離・再発見／端末移行目標）／PROVISIONAL（形式・保持／容量・Git・移行方式・適用版） | 2026-11-05 |
| DEC-041 | [`builder`製アプリのアンインストール・関連データ削除](DEC-041-builder-app-uninstall-and-data-deletion.md) | CONFIRMED（通常アンインストールはアプリ本体のみ／完全削除は対象別の説明・明示選択・承認）／PROVISIONAL（保持期間・UI・実装方式・適用版） | 2026-11-05 |
| DEC-042 | [独自Nagi KernelからLinuxカーネルへの方針転換](DEC-042-linux-kernel-transition.md) | CONFIRMED（カーネル選択）／PROVISIONAL（構成・互換方式・移行時期） | 2026-11-12 |
| DEC-043 | [Linuxカーネル + Nagi専用ユーザーランドと独立OS方針](DEC-043-nagi-userland-independent-os.md) | CONFIRMED（製品方針、後続DEC-017のNagi Nativeアプリ環境）／PROVISIONAL（具体構成・移植方式・Package・移行・ライセンス） | 2026-11-13 |
| DEC-044 | [Linux移行開発の開始承認と並列開発](DEC-044-linux-migration-development-authorization.md) | CONFIRMED（限定開発承認）／実行場所・個別構成は各割当で確認 | 2026-11-12 |
| DEC-045 | [Porting Toolkit PT-01〜06とPX-13 Local／Cloud橋渡し・PX-14統合開発者体験](DEC-045-nagi-porting-toolkit.md) | CONFIRMED（Windows／Linux重点・PT-01〜06・非AI診断／Target受入分離、PX-13 Cloud開発AI・人間との橋渡し優先、PX-14移植／検証の統合B＋将来保守／配布C）／PROVISIONAL（初期対象・Framework／CLI・Cloud Provider／署名／Package・性能・担当・版） | 2026-11-17 |
| DEC-046 | [共同開発者の役割と担当領域の自律性](DEC-046-co-developer-governance.md)／[PT-02〜PT-06担当案](../development/CO_DEVELOPER_ALLOCATION_PROPOSAL.md) | CONFIRMED（役割分担）／PROVISIONAL・未決定（個別担当・path・権限・着手） | 2026-11-15 |
| DEC-047 | [Linux移行レーンの作業場所・操作許可・統合先](DEC-047-linux-migration-workspaces-and-integration.md) | CONFIRMED（lane別worktree・操作範囲・統合先） | 2026-11-14 |
| DEC-048 | [DeepSeekによるNagiコード等レビューへの継続承認](DEC-048-deepseek-review-authorization.md) | CONFIRMED（全lane・全payloadのDeepSeekレビュー承認） | 2026-11-14 |
| DEC-049 | [速度優先の開発手続き](DEC-049-development-process-speed-priority.md) | CONFIRMED（開発手続きの速度優先・継続制約） | 2026-11-14 |
| DEC-050 | [仕様・構想の公開正本とEntire履歴・CIの分離](DEC-050-public-specification-repository.md) | CONFIRMED（公開NagiOSにコード・仕様・必要なCI／Private nagi-checkpointsにEntire履歴・関連メタデータ、CI無効）／PROVISIONAL（未検証環境への適用） | 2026-11-16 |
| DEC-051 | [レビュー方式A/B](DEC-051-review-methods.md) | CONFIRMED（現在は方式A） | 2026-11-15 |
| DEC-052 | [実機テスト端末の候補選定方針](DEC-052-physical-test-device-candidates.md) | CONFIRMED（候補選定のみ：PC＝Raspberry Pi・型番未指定／タブレット＝PineTab2／スマートフォン・モバイル＝未選定）／対応・購入・時期は未確定 | 2026-11-16 |
| DEC-053 | [AI Task開始前のWayback復元可能性表示](DEC-053-ai-task-recovery-preview.md) | CONFIRMED（Task開始前に復元可能性を表示する高水準の方向）／PROVISIONAL（分類・UI・判定基準・実装・適用版） | 2026-11-15 |
| DEC-055 | [Projectフォルダ中心の純正アプリ横断AI体験／PX-03 B＋将来C](DEC-055-project-folder-cross-app-context.md) | CONFIRMED（Project指定・Resource参照／横断Task・ProjectとWorkspace分離、PX-03統合作業ビューB＋将来文脈継続Cの高水準方針）／PROVISIONAL（画面／関連付け・一覧・検索／復元導線・技術・性能・版） | 2026-11-17 |
| DEC-056 | [Smart Paste・意味付き転送／PX-12案内型B＋将来高度Workflow C](DEC-056-smart-paste-semantic-transfer.md) | CONFIRMED（従来の意味付き転送・通常GUI、PX-12の方式案内B＋将来Workflow Cの高水準体験）／PROVISIONAL（初期App組合せ・既定転送方式・UI／Preview・Transfer／fallback技術・性能・提供版） | 2026-11-17 |
| DEC-057 | [学習・適応・継続によって成長するPersonal Agent OS](DEC-057-growing-personal-agent-os.md) | CONFIRMED（中核コンセプト・個人適応・モデル非依存の継続・ユーザー主導・PX-10 B／将来C方向）／PROVISIONAL（UI・状況判定・モデル／アルゴリズム・評価・提供版／時期） | 2026-11-17 |
| DEC-058 | [習慣学習を主軸に、Teach Modeを任意の補助機能とする](DEC-058-habit-first-learning-and-optional-teach-mode.md) | CONFIRMED（Habit-first・Teach補助・既存の保存／実行承認維持）／PROVISIONAL（Teach UI・記録技術・実装版） | 2026-11-16 |
| DEC-059 | [学習履歴を分かりやすく閲覧するNagi Insights（仮称）](DEC-059-learning-history-insights.md) | CONFIRMED（学習履歴・訂正権とPX-09のB管理方針／将来C方向との整合）／PROVISIONAL（UI・履歴Schema・C処理詳細・評価指標・適用版） | 2026-11-17 |
| DEC-060 | [応答速度を優先するIMEとAIを活用した学習](DEC-060-responsive-ime-ai-learning.md) | CONFIRMED（入力応答優先・許可されたAI学習・秘密入力除外と忘却・Cloud許可・AI不可時の通常IME・モデル非依存の継続性を目指す方針）／PROVISIONAL（方式・数値・版） | 2026-11-16 |
| DEC-061 | [純正PDF Reader兼Editorと段階的な高度編集／PX-11品質優先方針への整合](DEC-061-first-party-pdf-reader-editor.md) | CONFIRMED（PDF純正採用・基本機能先行・AI非依存／公開API、PX-11高水準方針との整合）／PROVISIONAL（1.0搭載区分・最小機能・高度機能／形式・技術・性能・版） | 2026-11-17 |
| DEC-062 | [純正ユーティリティ4分野採用とPX-11品質優先方針への整合](DEC-062-first-party-utilities.md) | CONFIRMED（4分野すべて採用・AI不要の軽量基本機能優先／PX-11高水準方針との整合）／PROVISIONAL（アプリ分割・1.0搭載区分・最小機能・形式・技術・性能・版） | 2026-11-17 |
| DEC-063 | [電話・連絡先・SMSの端末横断採用](DEC-063-phone-contacts-messaging.md) | CONFIRMED（純正採用・共通モデルと適応UI・能力／認可依存・People統合原則）／PROVISIONAL（アプリ構成・通信／端末間連携・学習・版） | 2026-11-16 |
| DEC-064 | [Nagi Share（仮称）：近距離の端末間ファイル共有](DEC-064-nagi-share-nearby-file-transfer.md) | CONFIRMED（純正OS機能・端末横断・ローカル直接転送・受信許可・暗号化／認証・LLM非必須）／PROVISIONAL（名称・技術・UI詳細・対応版／端末・性能） | 2026-11-16 |
| DEC-065 | [Nagi Handoff（仮称）：別端末での作業継続](DEC-065-nagi-handoff-work-continuation.md) | CONFIRMED（純正採用・意味的継続・安全境界）／PROVISIONAL（名称・UI・復元範囲・経路・クラウド・性能・適用版） | 2026-11-16 |
| DEC-066 | [同一LANから始める共通Device Connectivity／Trust／Transport](DEC-066-device-connectivity-lan-first.md) | CONFIRMED（初期LAN・共通抽象・将来遠隔拡張・安全境界）／PROVISIONAL（具体方式・運用・時期・性能） | 2026-11-16 |
| DEC-067 | [Nagi PowerとOS全体の決定論的な電力・熱・資源管理](DEC-067-nagi-power-deterministic-resource-management.md) | CONFIRMED（自動管理・安全／通常時の前景優先・簡易モード選択、Automation Mode構想のAI資源集中という意図・端末別実行戦略の許容）／PROVISIONAL（Automation Mode正式採用・起動条件／制御・端末・効果・実装版・Gate） | 2026-11-23 |
| DEC-068 | [Nagi Sleep（仮称）— スリープ・休止状態・復帰の管理](DEC-068-nagi-sleep-resume-recovery.md) | CONFIRMED（非LLM管理・状態保存と再検証・通常復帰と障害復旧の区別・段階的Hibernate）／PROVISIONAL（名称・方式・端末・時期・Gate） | 2026-11-16 |
| DEC-069 | [35項目のFoundation Checklist管理](DEC-069-foundation-checklist-management.md) | CONFIRMED（台帳管理）／PROVISIONAL（推奨優先度・検討順序） | 2026-11-16 |
| DEC-070 | [Memoryを活用した個人最適化Nagiアプリ制作／PX-13後期優先・Cloud橋渡しとの整合](DEC-070-memory-informed-native-app-creation.md) | CONFIRMED（許可されたMemoryに基づく個人向け制作／改善の方向、Cloud利用は別承認）／PROVISIONAL（Memory許可・UI・Bridge技術・動的適応・提供版） | 2026-11-17 |
| DEC-071 | [初心者向けNagi Native App制作体験／PX-13 B＋将来C・Cloud AI橋渡し・後期優先](DEC-071-nagi-native-app-creation-experience.md) | CONFIRMED（将来の共同制作B＋個人適応C、当面Cloud AI／移植橋渡し優先・高度制作環境は後順位）／PROVISIONAL（画面／Preview・Model・Cloud接続／許可UI・技術・性能・版） | 2026-11-17 |
| DEC-072 | [Follow／Evolution — Memoryからの操作支援とアプリ改善ビジョン](DEC-072-follow-evolution.md) | CONFIRMED（概念区分・改善試作を伴う提案の長期方向）／PROVISIONAL（実装・受容性・版割当て） | 2026-11-15 |
| DEC-073 | [許可された利用傾向からの根拠付き改善提案／PX-07との整合](DEC-073-proactive-friction-discovery.md) | CONFIRMED（潜在的不便の根拠付き提案・PX-07 B＋将来Cとの整合）／PROVISIONAL（分析・データ・Score・頻度／抑制・却下／再提示・UI・自動適用・技術・版） | 2026-11-17 |
| DEC-074 | [AI組み込み体験の優先・階層型Agent組織は長期構想／PX-15との整合](DEC-074-human-led-agent-organization.md) | CONFIRMED（当面のAI組み込みOS体験優先、階層Agent非優先の長期ビジョン、PX-15の1.0実用性基準との整合）／PROVISIONAL（組織構造・技術・適用版・1.0の個別受入条件） | 2026-11-17 |
| DEC-075 | [`builder`が作成した新規アプリの試用・正式導入](DEC-075-builder-app-trial-and-installation.md) | CONFIRMED（方式C：安全な試用・機能／証拠／権限の説明・採用判断後の別途承認付き正式導入）／PROVISIONAL（実装方式・適用版） | 2026-11-05 |
| DEC-076 | [`builder`製アプリのNagi連携・Action API提案／生成](DEC-076-builder-generated-action-api-integration.md) | CONFIRMED（方式C：用途別Public Action候補の自動提案／承認済みScope内実装検証／通常GUI両立）／PROVISIONAL（生成方式・Schema・適用版） | 2026-11-05 |
| DEC-077 | [LinuxカーネルLTS標準採用とStable先行利用](DEC-077-linux-kernel-lts-stable-policy.md) | CONFIRMED（LTS標準・Stable先行利用許可）／PROVISIONAL（具体版・更新保守・patch／config管理・適用版） | 2026-11-16 |
| DEC-078 | [端末別Boot方式と共通Nagi起動処理](DEC-078-boot-platform-entry-and-common-nagi-startup.md) | CONFIRMED（PCのUEFI基本・機器別方式許容・Linux起動後の共通Nagi処理）／PROVISIONAL（bootloader・復旧／信頼・適用版） | 2026-11-16 |
| DEC-079 | [System Imageの読み取り専用化・A/B更新／復旧・User Data／AIモデル分離](DEC-079-system-image-readonly-ab-data-separation.md) | CONFIRMED（read-only System・User Data分離・A/B更新／復旧・モデル論理分離）／PROVISIONAL（partition・filesystem・更新／復旧実装・適用版） | 2026-11-16 |
| DEC-080 | [Init／Service Manager：A案第一候補・B案との短期実装比較](DEC-080-init-service-manager-ab-evaluation.md) | CONFIRMED（A第一候補・短期A/B実装比較後に最終選択）／PROVISIONAL（A/B正式採用・試験詳細・期間・適用版） | 2026-11-16 |
| DEC-081 | [Nagi主体の基本ユーザーランドと汎用OSSの選択的活用](DEC-081-nagi-owned-userland-selective-oss-components.md) | CONFIRMED（Nagi固有サービス／API／アプリ環境の独自設計、libc・基礎ライブラリ・基礎ツールのOSS活用、既存distributionユーザーランド一式非流用）／PROVISIONAL（具体部品・ABI・適用版） | 2026-11-16 |
| DEC-082 | [電源ライフサイクルのNagi一元管理とサービス協調](DEC-082-power-lifecycle-coordination.md) | CONFIRMED（全電源遷移のNagi一元管理・各サービスの保存／停止／復帰協調・失敗時の中止／再試行／警告・データ／安全性優先）／PROVISIONAL（状態機械・具体順序／timeout・実装／適用版） | 2026-11-16 |
| DEC-083 | [Nagi Identityを正本とするLinux資格情報との連携](DEC-083-nagi-identity-linux-credentials-binding.md) | CONFIRMED（Nagi User／Profile／Session正本、Linux UID／GIDを低層保護に利用、ログイン／ロック管理、Profile別分離、AI権限非昇格）／PROVISIONAL（ID対応・認証方式・失効／実装・適用版） | 2026-11-16 |
| DEC-084 | [Nagi Permissionを正本とするLinux Sandboxの強制隔離](DEC-084-nagi-permission-linux-sandbox-enforcement.md) | CONFIRMED（Nagiが権限を判断、Linuxの保護機構で実効隔離、直接アクセス迂回防止、最小権限・失効／取消／狭窄、AI非特権）／PROVISIONAL（個別Linux技術・Threat Model・実装／適用版） | 2026-11-16 |
| DEC-085 | [Nagi独自IPC／Service BrokerとLinux標準Transportの分離](DEC-085-linux-transport-nagi-service-broker.md) | CONFIRMED（B案＝Nagi独自の型付きService API／BrokerとLinux IPCを組合せ・Transport独立の契約・認証済みCaller／権限境界）／PROVISIONAL（UDS等の具体transport・wire・性能・適用版） | 2026-11-16 |
| DEC-086 | [Nagi Secrets Serviceとハードウェア・ソフトウェア両立型の鍵保護](DEC-086-nagi-secrets-service-hybrid-key-protection.md) | CONFIRMED（C案＝Nagi独自Secrets Service、対応機器でTPM／TEE優先・非対応機器は暗号化ローカル保管と回復経路、用途／Profile別の鍵分離・非特権AI／秘密非露出・鍵更新／失効・自動クラウド預託なし）／PROVISIONAL（暗号・KDF・具体Key Store／回復・適用版） | 2026-11-16 |
| DEC-087 | [段階的なシステム信頼基盤と署名検証](DEC-087-tiered-system-trust-and-signature-verification.md) | CONFIRMED（B案：対応機器はHardware-backed Boot Trust・非対応機器もSoftware署名検証、限定Developer Mode／未署名アプリ、製品鍵と開発鍵の分離・失効／安全な復旧）／PROVISIONAL（具体Trust Anchor・Secure Boot実装・Rollback機構・実機受入／適用版） | 2026-11-16 |
| DEC-088 | [用途別Linux FilesystemとNagi Storage／Files APIの分離](DEC-088-linux-filesystem-nagi-storage-api.md) | CONFIRMED（B案＝Linux標準FSを用途別に選択活用、Nagi Storage／Files API・論理Resource IDを維持、snapshotとWayback／Backup／Workspace復元を区別）、Linux 0.1 Files の表示範囲・復元先・既知の制限を追記／PROVISIONAL（具体FS・mount／暗号・durability／ID binding・Linux受入／適用版） | 2026-11-24 |
| DEC-089 | [Nagi Volume ManagerとLinux Mount／Hotplugの責務分担](DEC-089-nagi-volume-manager-linux-mount-hotplug.md) | CONFIRMED（B案＝Nagi Volume ManagerがVolume検出・論理識別・認可・安全な取り外し・突然の切断の状態管理、Linuxのblock／mount／hotplug活用）／PROVISIONAL（自動mount条件・対応媒体／FS・識別／eject詳細・Linux受入／適用版） | 2026-11-16 |
| DEC-090 | [共通Persistence契約とデータ正本の分散保持](DEC-090-common-persistence-contract-distributed-authority.md) | CONFIRMED（B案＝Nagi共通の保存／移行／復旧契約＋用途／App／Provider別の分散正本、保存結果・Version／Migration／所有・忘却、Canonical Memoryと派生Index分離）／PROVISIONAL（具体DB／形式・durable commit／Migration・Linux受入／適用版） | 2026-11-16 |
| DEC-091 | [Nagi Recovery Coordinatorとサービス別責務の障害復旧](DEC-091-nagi-recovery-coordinator-service-owned-recovery.md) | CONFIRMED（B案＝共通Recovery CoordinatorとServiceごとの復旧、整合性／保存証拠確認・結果不明Actionの無断再実行防止、現行権限／Revision再検証・通常Sleep復帰と区別）／PROVISIONAL（Journal／Checkpoint・State Machine／復旧順序・Linux受入／提供版） | 2026-11-16 |
| DEC-092 | [周辺機器：USBストレージ・基本HIDと段階対応](DEC-092-peripherals-usb-storage-hid-staged-support.md) | CONFIRMED（方式B：USBメモリ・外付SSDと基本HIDを標準対象、カメラ・プリンター・スキャナーは段階対応、Linux／OSSとNagi管理・権限・通常UI連携）／PROVISIONAL（機種・範囲・API・プロトコル・版・実機受入） | 2026-11-16 |
| DEC-093 | [Network Service：Linux通信基盤＋Nagi独自Network Service](DEC-093-network-service-linux-nagi-policy-offline.md) | CONFIRMED（方式B：Linux通信基盤再利用、Nagi Serviceによる接続状態・設定・アプリ権限・Offline対応）／PROVISIONAL（OSS stack・強制方式・対象機器・版・受入） | 2026-11-16 |
| DEC-094 | [Resource Manager：Linux標準＋Nagi独自管理、スケジューリングの並行研究](DEC-094-resource-manager-linux-baseline-custom-scheduling-research.md) | CONFIRMED（#25 B＋C研究案：Linux標準を製品基本、Nagi Resource Managerの統合配分と独自schedulerの比較研究・標準方式への退避）／PROVISIONAL（cgroup／sched_ext等、アルゴリズム・性能・実機・採用版） | 2026-11-16 |
| DEC-095 | [App Lifecycle：Linux基盤＋Nagi App Supervisorと独自Runtimeの並行研究](DEC-095-app-lifecycle-linux-supervisor-runtime-research.md) | CONFIRMED（#26 B基本：Linux Native Process＋Nagi Supervisor、独自Runtimeの比較研究・標準経路保持）／PROVISIONAL（WASM等の採否・性能・対応機器・実装／版） | 2026-11-16 |
| DEC-096 | [Settings / Notification：Cの状況適応を最終目標に、Bの統合基盤を段階導入](DEC-096-settings-notifications-context-adaptive-staged.md) | CONFIRMED（#27 Cが最終目標、Bの統合Settings／Notificationを段階導入。明示設定・重大警告・Privacy／Permission維持）／PROVISIONAL（状況判定・習慣連携・自動調整詳細・版／受入） | 2026-11-16 |
| DEC-097 | [通常a11y基盤の先行整備とAI-native操作の長期方向](DEC-097-accessibility-foundation-and-ai-native-long-term-direction.md) | CONFIRMED（#28 通常a11y先行、長期方向との両立、GUI・AI不在時操作・認可維持）／PROVISIONAL（具体入力技術・時期・time／locale詳細・受入） | 2026-11-16 |
| DEC-098 | [Diagnostics / Logging：Nagi統合診断・ローカルAI簡易調査と報告支援](DEC-098-diagnostics-linux-nagi-ai-assisted-staged.md) | CONFIRMED（B統合診断＋将来C、ローカルAIは簡易調査・バグ報告の支援を中心とする）／PROVISIONAL（backend・モデル・具体解析・UI・性能・版・受入） | 2026-11-24 |
| DEC-099 | [Installer / First Setup：Linux基盤＋Nagi専用GUI（方式B）](DEC-099-installer-first-setup-linux-nagi-gui.md) | CONFIRMED（#30 Linuxの既存disk等を活用したNagi GUI Installer、AI非依存の安全なOffline-first導入、破壊前確認）／PROVISIONAL（Linux部品・GUI・partition／暗号化・端末・実機受入・適用版） | 2026-11-16 |
| DEC-100 | [System Update：Linux更新基盤＋Nagi統合Update Service（B）](DEC-100-system-update-linux-nagi-coordinated-ab.md) | CONFIRMED（#31 B：Linuxの更新機構とNagiの署名・互換検証／A/B切替・失敗時復旧・System／App／Model別管理）／PROVISIONAL（Linux採用部品・配信／鍵・更新単位・UI／自動適用・実機／版） | 2026-11-16 |
| DEC-101 | [Device Connectivity：Linux／OSS基盤＋Nagi独自端末連携Protocol、独自通信技術の並行研究](DEC-101-device-connectivity-nagi-protocol-linux-research.md) | CONFIRMED（#32 Bを標準、Nagiの意味的端末連携Protocolを設計、必要な低層通信は限定研究）／PROVISIONAL（具体Protocol／Transport・独自通信採否・端末／Release／受入） | 2026-11-16 |
| DEC-102 | [Hardware Support：Linux Driverを活用した段階的マルチデバイス対応](DEC-102-hardware-support-staged-multidevice-linux-drivers.md) | CONFIRMED（#33 B：PC／ARM64／Tabletへ段階的に拡張、Linux Driver活用・機種別実機検証で正式対応を判断）／PROVISIONAL（対応機種・Driver／Firmware Matrix・具体対応Version・Linux実機受入） | 2026-11-16 |
| DEC-103 | [Testing / Release：段階的検証Bを先行、自動化・AI支援Cを最終目標](DEC-103-testing-release-evidence-baseline-ai-assisted-staged.md) | CONFIRMED（#34 Host→Linux Image／QEMU→実機の証拠に基づくB先行、Cを将来の正式製品目標に段階導入。AIは原因仮説・合否は決定論的試験と既存Acceptance）／PROVISIONAL（試験網・モデル／生成・ツール／費用・時期／版・Linux受入） | 2026-11-16 |
| DEC-104 | [Build / Supply Chain：B先行と独立検証・強化自動化Cの正式な将来目標](DEC-104-build-supply-chain-baseline-independent-verification-staged.md) | CONFIRMED（統合供給網管理B先行・C正式目標）／PROVISIONAL（ツール・CI規模・予算・日程・閾値・必須Release・Linux実証） | 2026-11-16 |
| DEC-105 | [ユーザー環境のバックアップとWaybackの分担・連携](DEC-105-user-environment-backup-and-wayback.md) | CONFIRMED（B基本・Wayback連携・必要時のC追加）／PROVISIONAL（詳細・技術・適用版） | 2026-11-16 |
| DEC-106 | [Device Manager：内部責務分離と統合UI](DEC-106-device-manager-local-remote-unified-ui.md) | CONFIRMED（方式B：周辺機器・Nagi端末管理の内部責務分離、Settings／Devices UI統合、Linux機器基盤の活用）／PROVISIONAL（stack・機器・実装版） | 2026-11-16 |
| DEC-107 | [Network Devices：Ethernet／Wi-Fiを基本、Bluetoothを段階対応](DEC-107-network-devices-ethernet-wifi-bluetooth-staged.md) | CONFIRMED（方式B：Ethernet・Wi-Fiを基本機能、Bluetooth段階対応）／PROVISIONAL（chipset・無線stack・適用版） | 2026-11-16 |
| DEC-108 | [Display／GPU：Nagi独自Shell＋OSS描画基盤](DEC-108-display-gpu-nagi-shell-oss-graphics.md) | CONFIRMED（B改良案：Nagi独自のHome／Desktop・Workspace・端末適応UI、Linux／OSS描画基盤の活用）／PROVISIONAL（Wayland／Smithay等の採否・対象機器・版・受入） | 2026-11-16 |
| DEC-109 | [Input System：Linux入力基盤＋共通Nagi Input Service](DEC-109-input-system-cross-device-linux-foundation.md) | CONFIRMED（方式B：Linux既存入力基盤活用、Keyboard／Mouse／Touchの共通Input Service、端末横断UI）／PROVISIONAL（stack・IME／focus契約・機器・版・受入） | 2026-11-16 |
| DEC-110 | [Audio System：OSS音声基盤＋Nagi Audio Serviceと計測に基づく部分最適化](DEC-110-audio-system-oss-nagi-service-measured-optimization.md) | CONFIRMED（条件付きB：Linux／OSS音声基盤再利用、Nagi独自管理、低遅延・省メモリ・省電力の実機測定・不足箇所の個別対応）／PROVISIONAL（stack・機器・測定値・版・受入） | 2026-11-16 |
| DEC-111 | [50項目のImplementation Readiness棚卸しと条件付き判断](DEC-111-implementation-readiness-triage.md) | CONFIRMED（実装を妨げる未決判断だけ必要時に先行する運用）／REVIEW NOTE・PROVISIONAL（23／5／8／14の一次分類・順序） | 2026-11-17 |
| DEC-112 | [Linux開発イメージのIdentityサービスとREADY](DEC-112-development-identity-service-readiness.md) | CONFIRMED（Task24開発イメージで独立必須Identityと実READY、Data／NotesはPID 1内、独立Dataは別レーンで並行）／PROVISIONAL（製品全体の構成・接続仕様の詳細） | 2026-11-19 |
| DEC-113 | [外部コンテンツの扱いとプロンプトインジェクション対策](DEC-113-untrusted-content-and-prompt-injection.md) | CONFIRMED（外部由来の内容の扱い、高リスク操作の確認、Memory学習・無人Workflowの制限）／PROVISIONAL（リスク区分の詳細・確認UI・技術・適用版） | 2026-11-20 |
| DEC-114 | [Web利用・開発者向けLinux環境・アプリ互換の説明](DEC-114-web-use-developer-linux-environment-and-app-compat-message.md) | CONFIRMED（WebはAlbert、開発者モードの隔離Linux環境、`builder`用最低限ツール、アプリ互換の説明方針）／PROVISIONAL（環境の技術・ツール一覧・最終文言・適用版） | 2026-11-20 |
| DEC-115 | [Web互換性・ブラウザーエンジン・DRM・コーデック](DEC-115-web-compatibility-engine-drm-and-codecs.md) | CONFIRMED（Servoで実測を先行しLinux版の予備エンジン制約を解除、サイト報告・DRM・コーデックの方針）／PROVISIONAL（予備エンジンの採否・評価方法・コーデックの最終判断・適用版） | 2026-11-20 |
| DEC-116 | [性能の仮目標値・基準端末・最低動作環境](DEC-116-performance-targets-and-minimum-requirements.md) | CONFIRMED（基準端末と、仮目標値・最低動作環境案を実測で見直す進め方）／PROVISIONAL（数値・測定方法・実機調達・適用版） | 2026-11-20 |
| DEC-117 | [ユーザーデータの暗号化と端末紛失時の方針](DEC-117-user-data-encryption-and-lost-device.md) | CONFIRMED（ユーザーデータの既定暗号化、Memory／Profileの常時暗号化、解錠・回復と端末紛失時の方針）／PROVISIONAL（暗号方式・Filesystem連携・鍵階層・UI・適用版） | 2026-11-20 |
| DEC-118 | [サポート期間と脆弱性対応の目標](DEC-118-support-period-and-vulnerability-response.md) | CONFIRMED（1.0までは最新版のみサポート、種類別の脆弱性修正目標、GitHubの非公開報告機能を利用） | 2026-11-20 |
| DEC-119 | [不具合報告：ユーザー送信・伏せ字・ローカルAIによる報告準備支援](DEC-119-user-initiated-problem-reports.md) | CONFIRMED（本人送信・事前プレビュー／項目除外・自動クラッシュ報告なし、ローカルLLMの簡易調査・報告支援）／PROVISIONAL（報告形式・UI・AI整理・伏せ字・版） | 2026-11-24 |
| DEC-120 | [開発名称「Nagi」とDCOによる貢献の受け付け](DEC-120-development-codename-and-dco.md) | CONFIRMED（「Nagi」を開発名称として使用し、1.0までに製品名の扱いを決定、DCOで貢献を受け付け） | 2026-11-20 |
| DEC-121 | [無操作時の自動ロックとセッションの有効期間](DEC-121-idle-lock-and-session-lifetime.md) | CONFIRMED（無操作15分で自動ロック・設定は1〜60分または「しない」、ロック中もセッション維持・承認操作は受け付けない、絶対期限なし）／PROVISIONAL（ゲスト・端末別初期値・ロック画面の通知・版） | 2026-11-22 |
| DEC-122 | [Supervisor Council：端末適応型の安全審査・独立OS認可](DEC-122-adaptive-supervisor-council-security.md) | CONFIRMED（Councilはセキュリティ審査を主目的とし通常の深掘りでは常時合議しない、端末Resource適応、独立OS認可）／PROVISIONAL（Council審査条件・票数・低スペック時の省略／直列推論・Cloud深掘りの具体ルーティング・技術・版） | 2026-11-22 |
| DEC-123 | [独自カーネルの終了とLinux版 Nagi OS 0.1 Developer Preview](DEC-123-kernel-unification-and-linux-0.1.md) | CONFIRMED（独自カーネル凍結・Linux版0.1公開・旧M0〜M30記録保持・0.2有効化条件の置き換え）／PROVISIONAL（機能範囲・マイルストーン・旧コード整理・CI・公開時期） | 2026-11-23 |
| DEC-124 | [Nagi System Skills：OS機能のAI向け操作知識](DEC-124-nagi-system-skills.md) | CONFIRMED（OSの機能・設定・Public Actionの知識をAIが利用する方向）／PROVISIONAL（形式・登録・技術・UI・版） | 2026-11-24 |
| DEC-125 | [AI稼働状況パネル：Task・Model・Resource可視化](DEC-125-ai-operations-status-panel.md) | CONFIRMED（ローカルAI／Task／資源の統合表示、取得可能なCloud利用枠の確認方向）／PROVISIONAL（指標・Provider接続・UI・技術・版） | 2026-11-24 |
| DEC-126 | [Writer／Sheets／Slidesの標準保存形式とOffice互換](DEC-126-office-native-format-and-interchange.md) | CONFIRMED（B：公開Native形式を標準、Office形式入出力、共通コンテナと用途別内部表現、Office互換OSS活用と編集／連携への重点投入）／PROVISIONAL（拡張子・コンテナ・Schema・互換範囲・性能・版） | 2026-11-25 |


端末間連携の接続方針（2026-11-16追加）:


**変更履歴（2026-11-16）:** DEC-066を追加。同一Wi-Fi／LANを初期ターゲットとし、近距離直接通信・遠隔接続は段階的な拡張候補として保持する。初期の常時Cloud・オンラインアカウント・運営サーバーは必須にしない。DEC-014の移行／同期とCloud別許可を維持し、Clipboard採用にはしない。Share／Handoffの並行PRの状態は同DECを参照。


技術評価の実行計画: [MEMORY-BENCH-01 — Lightweight Memory Engine比較・性能評価](benchmarks/MEMORY-BENCH-01.md)。限定的なhost-only A/B smoke結果は[こちら](benchmarks/MEMORY-BENCH-01-HOST-SMOKE-2026-11-04.md)（CはNOT_MEASURED、QEMU/target PASSではない）。

追加のhost-only合成評価: [MEMORY-BENCH-01 V3–V5](benchmarks/MEMORY-BENCH-01-V3-V5-HOST-2026-11-04.md) — seed 42で1k/10k/100kを比較。C Hybrid/TokenizerはNOT_MEASURED、採用判断・閾値・runtime acceptanceではない。

新しいテーマは [TEMPLATE.md](TEMPLATE.md) を複製して追加する。

## Nagi Smart Pasteの採用（2026-11-16）

[DEC-056 — Smart Paste](DEC-056-smart-paste-semantic-transfer.md)により、通常のコピー＆ペースト／ドラッグ＆ドロップを維持しつつ、対応する純正アプリ間で構造化Object、出典、参照関係を活かした貼り付けを選べる体験をCONFIRMEDとした。Sheets→Slides、Albert→Writer、画像→Slidesを例とし、Projectフォルダとの将来連携を視野に入れる。正式名称・UI・Photos・実装版はPROVISIONAL。既存DEC-023／025の権限・分散正本・更新提案とRelease Gateを維持する。

## 既存文書との役割分担

- `docs/decisions/`: カーネル、権限、IPCなどの**技術的なArchitecture Decision Record (ADR)**。
- `docs/product-decisions/`（本フォルダ）: **ユーザーと決めた製品挙動／機能要求／プライバシー／体験**。
- `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`: 0.1の実装仕様。
- `docs/Nagi_OS_0.2_Codex_Implementation_Spec.md`: 0.2 Platform Foundationの実装仕様とGate。
- `docs/Nagi_OS_0.3_Architecture_Roadmap.md`: 0.3 Architectureの初版。改訂版は本フォルダの `roadmaps/` にある。
- `docs/workstreams/`、`.dev/workstreams.json`: 個別実装と進捗の正本。

2026-10-25にユーザーが提供した `Nagi_OS_0.3_Architecture_Roadmap_Revised.md` と `Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md` は本フォルダの内容を整合させる参照資料。**本記録を追加するだけで、それらの改訂ファイルがGitHub mainへ自動反映されたわけではない**。

## 決定履歴 — 近距離ファイル共有（2026-11-16）

[DEC-064](DEC-064-nagi-share-nearby-file-transfer.md)にNagi Share（仮称）の純正OS機能としての採用を記録した。写真・PDF・一般ファイルの日常の単発転送を対象とし、DEC-014の選択式Workspace移行・同期と区別する。ページ／Project／Workspace／作業状態のHandoffへの拡張は今回の確定範囲に含めない。技術・対応版・性能の確約や実装完了を意味しない。

## 将来の追加手順

1. 会話で決める（決定した選択肢と例外まで明確にする）。
2. 同じテーマのMDに追記し、なければ新規MDを作る。
3. 確定事項と数値案・実装検証待ちを分ける。
4. READMEの索引を更新し、**このリポジトリにcommitして保存内容を確認**する。小規模な文書修正は`main`へ直接commitしてよい。大規模な再編はpull requestで行う。
5. 後続の0.3 Implementation Specなどへ正式に移す際、参照元IDと受入条件を付ける。

**原則:** Memoryに覚えさせるのではなく、仕様をファイルに残す。

## OS基盤の恒久管理台帳（2026-11-16）

- [Nagi OS Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md) — 元の35項目を6分類群・7検討束で管理。製品決定、技術設計、host／旧guest／Linux／実機の実装・受入、依存・責任候補、次の判断を分離する索引・進捗台帳。DEC／ADR／実装仕様／workstreamの代替ではない。
- [DEC-069 — Foundation Checklistの恒久管理](DEC-069-foundation-checklist-management.md) — 台帳化自体はCONFIRMED。P0/P1/P2と検討順序はREVIEW NOTE / PROVISIONAL。個別35項目の技術・版・一括着手・担当・性能数値・既存Gate変更は確定していない。


### 変更履歴

| 日付 | 内容 |
|---|---|
| 2026-11-24 | [DEC-098](DEC-098-diagnostics-linux-nagi-ai-assisted-staged.md)・[DEC-119](DEC-119-user-initiated-problem-reports.md)に、ローカルAIを簡易調査とバグ報告支援中心に置く高水準の製品方針を追加。[DEC-099](DEC-099-installer-first-setup-linux-nagi-gui.md)ではOmarchy参考の高度なInstaller高速化案を今回は見送り、既存GUI Installerは維持。Workspace手動モード採用を維持し、Webアプリ風起動は未採用／ワンキー操作は未決。実装・UI・版はPROVISIONAL。 |
| 2026-11-24 | [DEC-124](DEC-124-nagi-system-skills.md)と[DEC-125](DEC-125-ai-operations-status-panel.md)を新設、[DEC-010](DEC-010-intent-first-adaptive-ui.md)へWorkspace表示モード手動切替を追加。3案の高水準方針のみCONFIRMED。実装・UI・技術・提供版はPROVISIONAL、既存Permission／通常GUI／Gateを維持。 |
| 2026-11-23 | [DEC-067](DEC-067-nagi-power-deterministic-resource-management.md)にAutomation Mode構想の意図を補足。PCの無人時にAI Taskへ資源を集中する目的は省電力ではなく処理能力の最大活用であり、PC／タブレット／モバイルで実行戦略が異なることを許容。モバイルでは復帰速度と電池を優先し短時間・断続処理を検討候補とする。具体制御・正式採用・版・実機対応はPROVISIONAL。DEC-020／029／067／082／094および既存Gateは維持。 |
| 2026-11-22 | [DEC-122](DEC-122-adaptive-supervisor-council-security.md)の後続決定で、Councilをセキュリティ審査のための必要時起動を主眼とする方針に明確化。普段の回答深掘りには常時合議せず、外部Cloud LLMはDEC-002の事前許可を条件とする選択肢。具体的なCouncil招集条件・Cloudルーティング・性能・版はPROVISIONAL。[DEC-074](DEC-074-human-led-agent-organization.md)と[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)に整合。 |
| 2026-11-22 | [DEC-122](DEC-122-adaptive-supervisor-council-security.md)を新設し、[DEC-074](DEC-074-human-led-agent-organization.md)と[0.4+ Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)へ連携。Councilを追加レビュー層とし、端末Resource適応・OS認可独立をCONFIRMED、低スペックの直列3回推論／省略・全会一致等はPROVISIONAL。Release Gate・既存の実装優先順位は不変。 |
| 2026-11-17 | DEC-020に8GB RAM級の前景操作＋ローカルAI連携の性能試験を0.3完成前に先行開始する決定を追記。実施詳細は未確定とし、既存Gate・測定証拠の区別を維持。 |
| 2026-11-17 | [DEC-009](DEC-009-nagi-first-home-desktop.md)／[DEC-010](DEC-010-intent-first-adaptive-ui.md)にPX-01のB（会話中心の統合Home）＋将来C（Project／Task等に応じる高度適応Home）を高水準でCONFIRMEDとして追記。[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)をPARTIALLY_RECORDEDへ更新。既存のHome標準・Desktop自動切替条件・委任Task中のHome維持・Workspace復元と手動配置保護・AI不在時の通常GUI・Gateを維持。具体レイアウト・導線・カード数・Nagi Bar位置・AI停止時詳細UI・技術・版はPROVISIONAL。 |
| 2026-11-17 | [DEC-010](DEC-010-intent-first-adaptive-ui.md)／[DEC-008](DEC-008-proactive-behavior.md)にPX-02のB（ユーザー主導で選択・却下・固定できる適応型UI）＋将来C（高度な個人適応UI）の高水準方針をCONFIRMEDとして記録。[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)はPARTIALLY_RECORDED。既存Intent-first・新規Workspace自動配置・複数画面／手動配置保護・控えめな通知・AI不在時通常GUIとGateを維持。カード／パネルの詳細UI・表示順序・適応条件・学習技術・性能・提供版はPROVISIONAL。 |
| 2026-11-17 | [DEC-055](DEC-055-project-folder-cross-app-context.md)にPX-03のB（Project中心の統合作業ビュー）＋将来C（文脈・関係・履歴に基づく高度な作業再発見／継続）を高水準でCONFIRMEDとして追加。[DEC-012](DEC-012-workspace-naming-organization.md)・[DEC-014](DEC-014-workspace-state-checkpoints.md)との整合を明記し、[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)はPARTIALLY_RECORDED。Resource参照とProject／Workspace分離、Workspace検索・保存・本人による復元先選択、Handoff・Memory・Permission／PrivacyとTask無断再開禁止を保持。統合UI・関連付け・一覧／検索／復元導線・技術・性能・版はPROVISIONAL。 |
| 2026-11-17 | [DEC-008](DEC-008-proactive-behavior.md)／[DEC-011](DEC-011-task-approval-progress-ui.md)にPX-04のB基本方針と将来Cの高水準方向をCONFIRMEDとして追記。[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)をPARTIALLY_RECORDEDに更新。Wake Word・常時待受・録音／文字起こしの保持・削除・詳細UI・音声本人認証・技術／モデル／版／時期／性能数値はPROVISIONAL。画面最終承認、Microphone Permission、Cloud送信別許可、音声割り込み抑制、停止・Text/UI fallback・Task状態保持を維持。 |
| 2026-11-17 | [DEC-001](DEC-001-agent-autonomy-actions.md)／[DEC-026](DEC-026-ai-change-verification-and-recovery.md)にPX-05のB基本＋将来C高水準方針をCONFIRMEDとして追記。[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)をPARTIALLY_RECORDEDへ更新。既存のPermission・画面最終承認・Wayback復元可能性表示・思想⑨の選択AとRelease Gateを維持し、Risk閾値・未判定時条件・UI・技術・提供版はPROVISIONAL。 |
| 2026-11-17 | [DEC-011](DEC-011-task-approval-progress-ui.md)と[DEC-020](DEC-020-ai-os-interaction-path.md)にPX-06の統合TaskセンターB＋将来の高度オーケストレーションCを高水準方針としてCONFIRMED。[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)をPARTIALLY_RECORDEDに更新。前景優先、Permission・Scope、資源待機と明示停止／障害復旧待ちの区別、0.3・初期0.4の既存Gateを維持。Queue／Priority・表示・依存関係・Scheduler・並列度・技術・版はPROVISIONAL。 |
| 2026-11-17 | [DEC-008](DEC-008-proactive-behavior.md)・[DEC-073](DEC-073-proactive-friction-discovery.md)にPX-07のB（文脈に応じた自然な提案）＋将来C（高度な複数提案候補の管理・比較・Feedback）を高水準でCONFIRMEDとして追加。[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)をPARTIALLY_RECORDEDへ更新。控えめな先回り、重大警告の別扱い、根拠・不確かさ・Memory／Privacy／PermissionとAction承認を維持。頻度・タイミング・抑制・却下／再提示・重要度判定・UI・技術・提供版はPROVISIONAL。 |
| 2026-11-17 | [DEC-004](DEC-004-memory-learning-forgetting.md)にPX-08のバランス型B基本方針（許可された情報の自然な学習・推測の区別・重要な内容の確認）をCONFIRMEDとして追記。[Product & Experience Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)をPARTIALLY_RECORDEDに更新。情報の細分類・重要判定・確認UX・数値／実装版はPROVISIONAL、既存Memory保護・忘却・Gateは不変。 |
| 2026-11-17 | [DEC-004](DEC-004-memory-learning-forgetting.md)でPX-09のB「自然忘却＋ユーザー管理」を基本方針として、将来Cの高度な重複統合・矛盾検出を追加する方向をCONFIRMED。[DEC-059](DEC-059-learning-history-insights.md)と[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)も整合更新。保持条件・管理UI・Cの具体方式・版／時期はPROVISIONAL。Source／Profile／Permission・明示的忘却・既存Gateは維持。 |
| 2026-11-17 | DEC-057へPX-10のB基本方針と将来Cの高水準な状況適応方向を追記し、Checklistの進捗をPARTIALLY_RECORDEDへ更新。既存のCONFIRMEDとRelease／Activation／Acceptance Gateを維持し、詳細・技術・評価・提供版／時期は未確定。 |
| 2026-11-17 | [DEC-021](DEC-021-roadmap-0.5-1.0.md)にPX-11のB（日常の主要作業を完結できる純正アプリ構成・品質優先）＋将来C（総合的な純正ソフトウェア環境）を高水準でCONFIRMEDとして追加。[DEC-061](DEC-061-first-party-pdf-reader-editor.md)・[DEC-062](DEC-062-first-party-utilities.md)との整合を追記、[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)はPARTIALLY_RECORDED。Phase 1〜3、PDF／ユーティリティ採用、AI非依存操作、1.0公開開発者環境と配布基盤の既存目標を維持。個別アプリの1.0必須／任意／後続、機能、形式、性能、技術、搭載／配布形態、提供版はPROVISIONAL。 |
| 2026-11-17 | [DEC-056](DEC-056-smart-paste-semantic-transfer.md)にPX-12のB（利用可能な貼り付け方式を説明・選択できる体験）＋将来C（高度アプリ横断Workflow）を高水準CONFIRMEDとして追記。[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)の共通参照A・更新提案A／承認済みTask限定更新、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)の正規Public Action優先A・限定GUI fallbackを維持。[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)をPARTIALLY_RECORDEDへ。初期対象・既定のコピー／参照・詳細UI／Preview・Transfer／fallback技術・性能・版はPROVISIONAL。 |
| 2026-11-17 | [DEC-071](DEC-071-nagi-native-app-creation-experience.md)にPX-13の対話型共同制作B＋将来の個人適応型共同制作Cを高水準CONFIRMEDとして記録。ただし独立・ローカル完結型の高度な制作環境は1.0計画の最終盤を目安とする長期・低優先に整理し、当面は[DEC-045](DEC-045-nagi-porting-toolkit.md)の他OS移植とローカルLLMによるCloud開発AI／既存IDE・人間との橋渡しを重視。[DEC-017](DEC-017-third-party-developer-ecosystem.md)／[DEC-021](DEC-021-roadmap-0.5-1.0.md)の1.0開発者環境／配布基盤、[DEC-070](DEC-070-memory-informed-native-app-creation.md)のMemory／[DEC-002](DEC-002-cloud-ai-data-sharing.md)のCloud事前許可、試用・正式導入・共有の別承認を維持。[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)はPARTIALLY_RECORDED。具体的UI・Provider／Schema・モデル・性能・対象・版はPROVISIONAL。 |
| 2026-11-17 | [DEC-017](DEC-017-third-party-developer-ecosystem.md)にPX-14のB（既存IDE・公開SDK・Porting Toolkit・任意Cloud AIの橋渡し・Build／Test／Package／Target検証を統合する開発者体験）＋将来C（互換性・継続保守／更新・配布準備支援）を高水準CONFIRMEDとして追加。[DEC-045](DEC-045-nagi-porting-toolkit.md)・[DEC-021](DEC-021-roadmap-0.5-1.0.md)および0.5〜1.0 Master Roadmapに整合させ、[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)はPARTIALLY_RECORDED。Nagi Native、SDK／Level 0〜3、移植優先OS、PT-01〜06非AI基盤、PX-13 Cloud橋渡し優先／高機能IDE後順位、Cloud事前許可、Host／Target独立・署名／導入／外部公開別承認、1.0開発者／配布目標を維持。初期言語・SDK／CLI／AI・署名／配布技術・性能・版はPROVISIONAL。 |
| 2026-11-17 | [DEC-021](DEC-021-roadmap-0.5-1.0.md)でPX-15の1.0実用OS B（正式対応のTargetと機能で代表的日常作業を安全・安定して完結・継続）＋将来総合OS C（対応機器・アプリ・利用形態を広げる）を高水準CONFIRMED。[DEC-074](DEC-074-human-led-agent-organization.md)と[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[0.5〜1.0 Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)へ整合し、[Checklist](NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md)はPX-15 PARTIALLY_RECORDED。既存の一般公開品質・公開SDK／配布基盤、AI組み込み体験優先／階層Agent後順位、通常GUI・Permission／Privacy、Target／Host試験区別、Release／Activation／Acceptance Gateを維持。代表作業・1.0アプリ必須リスト・対応Hardware・性能／品質数値・公開判定・日程はPROVISIONAL。 |
| 2026-11-17 | PX-01〜PX-15の公開PRにある確定方針を一括統合。重複するDEC-008／010／011／017／021／045、README／Checklistと0.5〜1.0ロードマップを整合し、個別の高水準CONFIRMEDを保持。未決の詳細・既存Gate・Target実装状態を昇格させない。 | 
| 2026-11-16 | [DEC-087](DEC-087-tiered-system-trust-and-signature-verification.md)を追加。Foundation #11で段階的な信頼基盤B案を確定。Secure Boot／Hardware root of trust対応機器では保護を活用し、非対応機器でもSoftware署名検証を行い保証の差を明示。OS／更新／アプリの署名・検証、限定Developer Mode・Owner認可、製品鍵／開発鍵分離・更新／失効・危険なRollback防止と復旧を設計対象にした。具体的Secure Boot／Trust Anchor／鍵・Recovery運用・Linux実機検証・提供版は未決定。 |
| 2026-11-16 | [DEC-086](DEC-086-nagi-secrets-service-hybrid-key-protection.md)を追加。Foundation #10のC案（Nagi Secrets Service＋対応機器ではTPM／TEE等で保護を優先、非対応機器では暗号化ローカルKey Store／パスフレーズ等で利用・回復）を確定。Identity／Permission、User／Profile・用途別鍵分離、AIへの秘密値の不要な露出防止・鍵失効／更新・ログ秘匿・クラウド鍵預託を既定で行わない方針を記録。暗号技術・TPM／TEE・用途別回復可能性・提供版は未決定。B2 #6〜#10の高水準方針が一巡したが実装・Linux受入は別。 |
| 2026-11-16 | [DEC-085](DEC-085-linux-transport-nagi-service-broker.md)を追加。Foundation #9のB案を基本方針に選択し、Nagi独自の型付きIPC／Service API・Service Brokerを保持しながら、データ輸送はLinux標準IPCを活用。Linux資格情報とNagi Identity／PermissionのBinding、Transport独立契約、低レイヤー再実装を前提としない方向を記録。Unix Domain Socket候補・具体技術・実測性能・適用版は未決定。 |
| 2026-11-16 | [DEC-084](DEC-084-nagi-permission-linux-sandbox-enforcement.md)でFoundation #8のB案を確定。Nagi Permission／Capabilityが現在の権限を判断し、Linux保護機構でAPI外の直接アクセスも隔離。最小権限・取消・失効・狭窄、AI／アプリ非特権、許可済み操作の重複確認回避を維持。namespace／seccomp／LSM等の具体採否・Linux targetでの迂回拒否試験・適用版は未決定。 |
| 2026-11-16 | [DEC-083](DEC-083-nagi-identity-linux-credentials-binding.md)を追加。Foundation #7でNagi論理Identity（User／Profile／Session）を正本に、Linux UID／GIDを低レイヤーの保護に活用するB案をCONFIRMED。ログイン・ログアウト・ロック・復帰のNagi管理、Profile別ファイル／Memory／Workspace／Task分離、Owner権限の自動付与禁止を継承・具体化。UID対応・認証手段・Session失効条件・Linux実装／受入・適用版は未決定。 |
| 2026-11-16 | [DEC-082](DEC-082-power-lifecycle-coordination.md)を追加。Foundation #6で電源ライフサイクルのNagi一元管理、サービス横断の保存・停止・復帰準備、失敗に応じた中止・再試行・警告、データと安全性の優先を確定。DEC-067 Power／DEC-068 Sleepと重複させず、具体的な順序・状態機械・強制終了・技術・受入・適用版は未決定。 |
| 2026-11-16 | [DEC-091](DEC-091-nagi-recovery-coordinator-service-owned-recovery.md)を追加。Foundation #15 Crash RecoveryでB案をCONFIRMED。Nagi Recovery Coordinatorが保存証拠／復旧状況を統括し、各Serviceが自分の正本・状態を復旧する。検証済みと結果不明を区別し、中断Task／副作用Actionの無断再実行禁止・User／Profile／Permission／Revision再検証、通常Sleep復帰と区別。具体Journal／Checkpoint／Recovery State Machine・Linux電源断試験／適用版は未決定。 |
| 2026-11-16 | [DEC-090](DEC-090-common-persistence-contract-distributed-authority.md)を追加。Foundation #14 B案をCONFIRMED。Nagiの共通Persistence契約で保存結果・Schema Version／Migration・所有／復旧・削除／忘却を統一し、データ正本はUser／Profile／Memory／Workspace／Task／App／Providerごとに保持。Canonical Memoryと再生成可能なIndex／Cacheを分離し、Workspace復元・本文Backup・Task再実行を区別。単一共通DB強制なし、具体保存技術・durability／transaction・Linux実機受入／版は未決定。 |
| 2026-11-16 | [DEC-089](DEC-089-nagi-volume-manager-linux-mount-hotplug.md)を追加。Foundation #13のB案をCONFIRMED。Linux block／mount／hotplugを利用し、Nagi Volume Managerが接続検出・デバイスパスと別のVolume識別・User／Profile／App権限・安全な取り外し・突然の切断時の状態を管理。書込み完了／unmount未確認の成功表示や不明なActionの自動再実行を禁止。初期対応媒体／FS・自動mount・実際のsafe eject契約・Linux実機受入／適用版は未決定。 |
| 2026-11-16 | [DEC-088](DEC-088-linux-filesystem-nagi-storage-api.md)でFoundation #12のB案をCONFIRMED。Linux標準FilesystemをSystem／User Data／Model Store／Temp等の用途別に選択活用し、Nagi独自Storage／Files API・path／inodeと別のResource ID・分散正本を維持。Filesystem snapshotとNagi Wayback／Backup／Workspace復元を区別。EROFS／ext4／Btrfs等は比較候補で具体FS・暗号／mount・耐障害性・Linux受入・提供版は未決定。B3 Storage／Persistence／Recoveryの検討開始。 |
| 2026-11-16 | [DEC-092](DEC-092-peripherals-usb-storage-hid-staged-support.md)でFoundation #22の方式Bを確定。USBストレージ（USBメモリ・外付SSD）と基本HIDを標準対象、カメラ・プリンター・スキャナーは段階対応。Linux既存driver／OSSを活用しNagi Device Manager／Permission／Privacy／通常UIへ連携する。全Linux対応機器の保証ではなく、具体機種・対応範囲・API・プロトコル・実装版・実機受入は未確定。DEC-088〜091は未マージPR #134で使用済みのため採番を回避。 |
| 2026-11-16 | [DEC-110](DEC-110-audio-system-oss-nagi-service-measured-optimization.md)でFoundation #21の「条件付きB」を確定。音声入出力・mixing／routingにLinux／OSSを活用し、Nagi Audio Serviceで機器切替・優先度・マイク権限・Voice連携を管理。低遅延・省メモリ・省電力は実機測定し不足部のみ最適化／部分置換を検討。ALSA／PipeWire等の採否・実測・対応端末・適用版・受入は未確定。 |
| 2026-11-16 | [DEC-109](DEC-109-input-system-cross-device-linux-foundation.md)でFoundation #20の方式Bを確定。Linux既存入力基盤を活用し、Keyboard／Mouse／TouchをNagi共通Input Serviceで扱う。PC／Tablet／Mobileの入力適応、DEC-060のIME速度優先・学習／秘密入力保護を維持。入力stack・focus・機種・適用版・実機受入は未確定。 |
| 2026-11-16 | [DEC-108](DEC-108-display-gpu-nagi-shell-oss-graphics.md)でFoundation #19のB改良案を確定。Nagi Home／Desktop・Workspace等の独自ShellとPC／Tablet／Mobileの適応契約を維持し、Linux／OSSのGPU・Display・描画基盤を活用する。Wayland／Smithay等の個別技術・Linux実機受入・提供版は未確定。 |
| 2026-11-16 | [DEC-106](DEC-106-device-manager-local-remote-unified-ui.md)でFoundation #17の方式Bを確定。ローカル機器とNagi端末の管理責務を内部で分離し、Settings／DevicesのUIは統合。Linux機器基盤を活用し、詳細構成・対応機器・実機受入は未確定。 |
| 2026-11-16 | [DEC-107](DEC-107-network-devices-ethernet-wifi-bluetooth-staged.md)でFoundation #18の方式Bを確定。Ethernet・Wi-Fiを基本、Bluetoothを段階的に対応。driver／firmware・機種・接続方式・適用版・Linux実機受入は未確定。 |
| 2026-11-16 | [DEC-067](DEC-067-nagi-power-deterministic-resource-management.md)にFoundation #23の方式Bを追記。自動管理を標準とし、省電力／バランス／高性能に相当する簡易モードを選択可能とする。安全・熱保護・前景応答性が優先し、モード名称・制御値・適用端末・実装版は未確定。 |
| 2026-11-16 | [DEC-093](DEC-093-network-service-linux-nagi-policy-offline.md)でFoundation #24の方式Bを追加。Linuxの通信基盤を使い、Nagi Network Serviceで通信設定・接続状態・アプリ権限・Offline対応を統合。全面独自TCP/IP実装を標準採用せず、特殊経路の部分最適化は後続判断。具体stack・強制機構・機器・版・実機受入は未確定。 |
| 2026-11-16 | [DEC-103](DEC-103-testing-release-evidence-baseline-ai-assisted-staged.md)でFoundation #34を記録。Bの段階的・証拠に基づく検証を先行整備し、複数環境の自動テスト・障害注入・テスト不足分析・AI原因調査等のCを将来の正式製品目標とする。AIは仮説・補助、PASS／FAILは決定論的試験と既存Acceptance。DEC-049の速度優先・証拠の正直さ、既存Gate維持、全35項目完了を新Gateにしない。具体試験網・モデル・費用・時期・版・Linux受入は未確定。 |
| 2026-11-16 | [DEC-102](DEC-102-hardware-support-staged-multidevice-linux-drivers.md)でFoundation #33の方式Bを記録。Linuxの既存Driver／低層Hardware機構を活用し、PC／ARM64／Tablet等へ段階的に対応し、機種ごとの実機検証で正式対応を判断する。DEC-052のRaspberry Pi（型番未選定）／PineTab2／Phone未選定と0.2のQEMU・mini-PC・generic ARM64の区別、Desktop firstと既存Gateを維持。具体機種・性能・Driver・適用版・受入は未決定。 |
| 2026-11-16 | [DEC-101](DEC-101-device-connectivity-nagi-protocol-linux-research.md)でFoundation #32を記録。Linux／OSS接続技術とNagi統合Connectivity Serviceを基本に、Share／Handoff等の端末能力・作業状態・Resourceをやり取りするNagi独自Protocolを設計し、必要な独自通信技術は並行研究する方針をCONFIRMED。DEC-066のLAN-first、認証・暗号・Permission／Privacy、無断同期禁止を維持。具体通信／暗号方式、研究成果採用、端末・版・実機受入は未確定。 |
| 2026-11-16 | [DEC-100](DEC-100-system-update-linux-nagi-coordinated-ab.md)でFoundation #31の方式Bを記録。Linuxの更新機構を活用し、Nagi統合Update Serviceが署名・互換検証、A/B切替・失敗時復旧を担う方針をCONFIRMED。DEC-079のread-only System／User Data分離と、App／Model更新の別権限・別契約を維持。Cの差分更新／段階配信・自動最適化、実装・配信・鍵・実機・Versionは未決定。 |
| 2026-11-16 | [DEC-099](DEC-099-installer-first-setup-linux-nagi-gui.md)でFoundation #30のB方式を追加。Linuxの既存低層機構とNagi専用GUI Installerを基本採用し、ローカルLLMの高度判断を要件とせず、Offline-firstとAI不在時の導入、ディスク破壊前の本人確認を重視。DEC-079のOS／Data分離を継承し、技術詳細・暗号化・実機受入・版は未決定。 |
| 2026-11-16 | [DEC-098](DEC-098-diagnostics-linux-nagi-ai-assisted-staged.md)でFoundation #29のB＋段階的C正式目標を追加。Linuxの診断情報をNagi統合Diagnosticsで扱い、将来のAIによる原因候補と修復提案へ段階的に拡張する。検証事実とAI推測の分離、ログ秘匿／権限／Redaction、AI不在時の通常診断、既存0.2 M04とRelease Gateを維持。backend・AI方式・自動修復・版は未確定。 |
| 2026-11-16 | [DEC-097](DEC-097-accessibility-foundation-and-ai-native-long-term-direction.md)でFoundation #28を記録。keyboard／navigation・screen-reader等の通常a11y基盤を先行整備し、AI-nativeで正確なポインティングに依存しない長期方向と両立させる。視線／ジェスチャー／空間入力等の採用・時期、time／locale詳細は未決定。GUI・AI不在時操作・Permission／Policy／confirmationと既存Gateを維持。 |
| 2026-11-16 | [DEC-096](DEC-096-settings-notifications-context-adaptive-staged.md)でFoundation #27を記録。Cの文脈・習慣に合わせた状況適応型Settings／Notificationを最終製品目標とし、Bの統合Settings UI／通知センター／Scope付き設定・通知管理を先行段階として導入する。DEC-008の重大警告・控えめな提案、User明示設定、Permission／Privacy、0.2 M11／M14の契約を維持し、具体技術・自動化詳細・実装版は未決定。 |
| 2026-11-16 | [DEC-095](DEC-095-app-lifecycle-linux-supervisor-runtime-research.md)でFoundation #26の「B基本＋独自Runtime並行研究」を記録。Linuxのプロセス・隔離機構とNagi App Supervisorを製品標準とし、独自Runtimeの隔離・起動速度・RAM・Workspace／AI連携・開発者負担等を比較研究する。採用は実測後に別判断、WASM等の個別技術・版・実機受入は未決定。 |
| 2026-11-16 | [DEC-094](DEC-094-resource-manager-linux-baseline-custom-scheduling-research.md)でFoundation #25のB＋C研究案を記録。製品標準はLinuxのscheduler／資源制御＋Nagi Resource Manager、独自CPUスケジューリングの研究・比較を行い、有効性と安全性を実機で確認できた部分だけ後続採用を判断する。cgroup v2／PSI／sched_extは候補、性能・機器・Releaseは未確定。 |
| 2026-11-16 | [DEC-081](DEC-081-nagi-owned-userland-selective-oss-components.md)でFoundation #5の基本ユーザーランド方針を追加。Nagi固有のサービス・API・アプリ環境は独自設計、libc・基本ライブラリ・基礎ツールはOSSを選択活用、既存distributionのユーザーランド一式は製品基盤にしない。具体libc・ABI・rootfs・採用版は未決定。B1 #1〜#5の方針検討は一巡したが、#4のA/B正式採用やLinux target受入は未完了。 |
| 2026-11-16 | [DEC-080](DEC-080-init-service-manager-ab-evaluation.md)でFoundation #4の選定プロセスを確定。軽量なNagi独自PID 1＋独自Service ManagerのA案を第一候補とし、軽量OSS Init＋Nagi Service ManagerのB案と短期の実装比較後に最終判断。両案の正式採用・具体期間・試験条件・結果は未決定。 |
| 2026-11-16 | [DEC-079](DEC-079-system-image-readonly-ab-data-separation.md)を追加。Foundation #3についてread-only System、User Dataの更新後保持を目的とする分離、A/B更新と復旧、AIモデルの論理的分離をCONFIRMEDとした。partition数・filesystem・モデルの物理配置・Linuxの実装／受入・適用版は未確定。 |
| 2026-11-16 | [DEC-078](DEC-078-boot-platform-entry-and-common-nagi-startup.md)でFoundation #2のBoot方式を記録。UEFI対応PCはUEFI基本、Raspberry Piやタブレット等は機器固有の起動方式を許容し、Linux起動後のNagi処理を共通化する。bootloader・信頼／復旧方式・対応機種・実機試験・Releaseは未確定。 |
| 2026-11-16 | [DEC-077](DEC-077-linux-kernel-lts-stable-policy.md)でFoundation #1のLinuxカーネル系列方針を追加。製品標準はLTS、開発・先行検証では最新Stableを許容。具体的な版・更新運用・適用版とLinux target受入は未確定。 |
| 2026-11-16 | DEC-069と35項目の台帳を追加。Linux移行差異と並行PRを明示し、管理ルール・実装契約テンプレートを保存。個別技術採用・Release Gate・実装状況は変更しない。 |

## 将来ロードマップ(2026-11-04保存、2026-11-15公開)

- [0.3 Revised — Personal Agent](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md) — 2026-10-25改訂、2026-11-04 Memory Engineの未確定方式を明記、Draft / Forward Architecture。
- [0.4+ Multi-Agent Organization](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md) — Vision proposal。Implementation Specではない。
- [0.5–1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md) — DEC-021に基づく段階案。継承する製品方針と暫定の版割当てを区別し、Implementation SpecやRelease commitmentではない。

前の2ファイルはユーザーが提出したMD原本のGit blob SHAを照合して保存したもの(2026-11-15に公開リポジトリへ移管)。0.5–1.0 Master Roadmapはその後の製品決定と承認済み方向を整理した新規提案である。いずれも公開リポジトリの既存Roadmapやrelease gateは変更しない。


DEC-013の詳細設計案: [Memory Resource Governor](MEMORY-RESOURCE-GOVERNOR-DESIGN.md)（PROVISIONAL契約。閾値・技術選択は未確定、0.3実装許可ではない）。


## 後続の開発開始承認（2026-11-12）

[DEC-044 — Linux移行開発の開始承認](DEC-044-linux-migration-development-authorization.md) は、上記DEC-042/043時点の「方針だけでは実装開始を意味しない」に続く、Private Linux移行開発の明示承認を記録する。限定適用の[移行ADR](../decisions/2026-11-12-linux-migration-development-scope.md)とAGENTS / 0.1実装仕様の同日優先注記を参照。旧本文・既存release Gateは保持し、実装開始・試験成功・正式移行済みとは別に検証する。


## カーネル移行に伴う仕様・ロードマップ整合（2026-11-13）

[影響表・実装確認記録](KERNEL-TRANSITION-ALIGNMENT-2026-11-13.md)に、DEC-042/043の確定製品方針、DEC-044の限定開発承認、未確定構成、確認できた旧基盤実装と証拠限界を分けて記録した。新しい製品決定ではない。

- [0.1実装仕様](../Nagi_OS_0.1_Codex_Implementation_Spec.md): 旧基盤本文と2026-11-12限定優先注記を保持し、解釈・影響表を追記。
- [0.2仕様のLinux移行補足](../Nagi_OS_0.2_Linux_Migration_Addendum.md): 0.2仕様に追記したLinux移行の補足。公開版の全文複製・release仕様の全面置換ではない。
- [ロードマップ索引](roadmaps/README.md): 0.3 Revised・0.4+・0.5–1.0へ日付付きLinux適用注記を追加。旧M30順序を承認済み移行開発の全面停止とせず、既存release Gateと版境界を維持。

本文中の「ロードマップ自体は実装開始許可ではない」という原則と、後続DEC-044の明示承認を区別する。Publicへの転載、実開始・target PASS・正式切替・互換保証は本整合作業から推定しない。

## Linux移行レーンの作業場所・統合先（2026-11-14）

[DEC-047 — Linux移行レーンの作業場所・操作許可・統合先](DEC-047-linux-migration-workspaces-and-integration.md) は、DEC-044と移行ADRに記録した初回「NOT ASSIGNED」およびworktree/Git/build/VM未開放のsnapshotに対する、2026-11-14本人の後続承認を記録する。laneごとのSSD worktree、worktree内の明示操作、統合先 codex/linux-integration-20261114 と継続する本人指定の保全・禁止事項は同DECを参照する。DEC-042/043の製品方針を重複決定せず、既存release acceptance gateをLinux開発開始条件へ拡張しない。


## DeepSeekレビューへの継続承認（2026-11-14）

[DEC-048 — DeepSeekによるNagiコード等レビューへの継続承認](DEC-048-deepseek-review-authorization.md) は、すべてのlane・payloadについて、非公開を含むNagiコード・差分・分析資料をDeepSeekレビューへ送る継続承認を記録する。payloadごとの再確認は不要。認証にはKeychainに保存済みの既存credentialを使用できるが、credentialそのものは送信しない。DEC-002に続く、DeepSeekによるNagiレビュー送信の具体的な承認である。

## 8GBでの実用性を目指す性能方針の変更履歴（2026-11-16）

同日追加決定: [DEC-020](DEC-020-ai-os-interaction-path.md)へ、標準・軽量など複数の許可済みモデルをストレージに保持し、RAM常駐を必要最小限にして選択・切替する方針を追記。資源逼迫時の自動切替を目指すが、技術的・品質上難しければ手動切替を確保。ユーザーのモデル固定設定と承認済みTask境界を守り、シームレス性や実装版・数値は未確定。

[DEC-020](DEC-020-ai-os-interaction-path.md)に、8GB級環境で純正アプリの日常操作を軽快に保ちながら軽量量子化LLMを使ったアプリ連携を成立させる目標を追加。モデル交換・低負荷時制御・実測評価と既存のOS優先順位を保持し、定量的な合格値やハードウェア対応保証を新たに確定していない。

## 学習履歴の可視化と既存Memory仕様の整合（2026-11-16）

[DEC-059](DEC-059-learning-history-insights.md)を新設し、[DEC-003](DEC-003-memory-classification.md)と[DEC-004](DEC-004-memory-learning-forgetting.md)に学習の履歴・根拠・実用上の反映を説明できるユーザー体験を補記した。確定情報と推定の違い、元SourceとPermission、ユーザーによる訂正・削除・忘却を維持する。実装は未着手・未確認であり、本記録だけで完了扱いしない。

## 中核コンセプトの再確認（2026-11-16）

[DEC-057](DEC-057-growing-personal-agent-os.md)を新設し、0.3 Roadmapに既存のMemory／Learning／Profile／Model Portabilityを備えたPersonal Agent OSが、今後もNagi OS全体の製品コンセプトであることを確定記録。[VISION-001](VISION-001-human-adaptive-os.md)にも反映した。既存の記憶・忘却・Permissionの決定とRelease Gateを維持する。

## 習慣学習を主軸としたTeach Modeの位置付け（2026-11-16）

[DEC-058](DEC-058-habit-first-learning-and-optional-teach-mode.md)を新設してHabit-firstを確定し、[DEC-004](DEC-004-memory-learning-forgetting.md)、[DEC-028](DEC-028-deterministic-workflows-from-habits.md)、[DEC-033](DEC-033-natural-language-workflow-authoring.md)の優先順位・補助的なTeach作成経路と変更履歴を整合した。自動記憶・自然忘却、ユーザーの明示指示優先、Workflow保存／有効化の承認、実行権限の分離は維持する。

## 仕様・構想の公開リポジトリへの移管(2026-11-15)

[DEC-050 — 仕様・構想の正本を公開リポジトリへ移す](DEC-050-public-specification-repository.md) により、本フォルダを非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移し、今後の仕様決めもここに記録する。旧名称「Jev」時代に置き換え済みだったDEC-015・016・018・019・022の旧ファイル5本は移していない(内容は現行の`builder`版と[DEC-024](DEC-024-external-jev-reference-and-builder-label-correction.md)にある)。

## 決定変更履歴（2026-11-16）

- [DEC-068 — Nagi Sleep（仮称）](DEC-068-nagi-sleep-resume-recovery.md)を新設。DEC-014／022の保存・選択式復旧を継承し、RAM保持の通常復帰と障害後の復旧を区別する。既存決定の置換や実装開始・Release割当て・Gate変更ではない。
- 番号は並行中PRのDEC-053〜066および[Nagi PowerのDEC-067（PR #128）](https://github.com/Roy-To/NagiOS/pull/128)との競合を避けて選定した。これらのmainへのマージを本記録で表明しない。

## Nagi Powerと低レイヤー基盤の整備（2026-11-16）


変更履歴: 2026-11-16 — DEC-020／013を補足するNagi Powerの製品決定を追加。全サブシステムの一括採用・実装開始・Release変更は確定しない。

## 決定履歴 — Nagi Handoff（2026-11-16）

[DEC-065](DEC-065-nagi-handoff-work-continuation.md)でNagi Handoff（仮称）を、PC・タブレット・スマートフォン間で文書・Resource・編集箇所・意味的な作業状態を継続する純正OS機能として採用した。既存Continuity、Homeの `home.continue_from_device`、Device Capability、DEC-014の選択式Workspace移行／同期を最大限再利用し、受信先の画面・能力に応じてUIを再構築する。現行User／Profile／Owner／Permission／Privacy・Source有効性／Revisionを再検証し、実体がなければユーザー許可に基づく取得／転送経路または代替を提示する。無断のファイル・個人データ転送／クラウド同期・競合上書きをせず、中断AI Task／副作用Actionを自動再開・再実行しない。名称・UI・復元範囲・経路・共有方式・クラウド利用・瞬時性・対応版はPROVISIONAL。全アプリ・全状態の完全移行、常時オフライン成立、自動同期は保証しない。単発ファイル転送のNagi Shareとは別の概念である。既存Desktop first、Release／Activation／Acceptance Gate、版割当てを変更せず、実装済みを示さない。

先行するNagi ShareのDEC-064は[PR #125](https://github.com/Roy-To/NagiOS/pull/125)にあり、本記録時点でmain未マージ。今回のHandoffは後続の独立した採用決定として記録する。

## 変更履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | [DEC-063](DEC-063-phone-contacts-messaging.md)を追加。電話・連絡先・SMSの純正採用と端末横断原則を記録し、通信能力への依存と未確定事項を分離。並行するPDF・ユーティリティ決定のDEC-061／062との番号重複を回避。 |

## 統合変更履歴（IME・PDF・ユーティリティ）

| 日付 | 内容 |
|---|---|
| 2026-11-16 | [DEC-062](DEC-062-first-party-utilities.md)を追加。純正ユーティリティ4分野すべての採用とAI非必須の基本機能優先を記録し、構成・連携詳細・版等の未確定事項を分離。PDFの別決定で使用中のDEC-061とのID重複を回避。 |

| 2026-11-16 | [DEC-061](DEC-061-first-party-pdf-reader-editor.md)を追加。基本PDF体験を先に整え、高度編集へ段階的に拡張する採用方針を記録。既存DEC・純正アプリ仕様・ロードマップへ参照を追加。 |
| 2026-11-16 | [DEC-060](DEC-060-responsive-ime-ai-learning.md)を追加。IMEの応答速度を優先し、許可されたデータからAIで学習精度を高める方針と入力データの安全境界を記録。具体方式・性能値・適用版は未確定。DEC-057はPR #116の保存内容を参照し、main未統合を区別。 |

## Projectフォルダを中心とした純正アプリ横断体験（2026-11-16）

[DEC-055 — Projectフォルダ中心の純正アプリ横断AI体験](DEC-055-project-folder-cross-app-context.md)で、既存フォルダを利用者のProjectに指定し、Writer／Sheets／Slides等のResourceと元の保存先を残す参照を束ね、許可されたAI横断Task、関係資料の更新提案、Projectごとの作業文脈・継続へつなげる長期方向をCONFIRMEDとした。Projectは資料・目的・文脈、WorkspaceはUI・状態復元の別単位である。名前、具体UI・モデル、提供版はPROVISIONAL。DEC-023・025および既存Permission・Memoryルールとrelease gateは維持する。

同日補足：1 Projectを複数Workspaceから扱え、新専用ファイル形式を強制しない。Project所属・参照登録は本文取得・外部送信・自動変更の許可にならず、DEC-025の通常提案／事前承認済み限定Taskの例外を維持する。DEC-003／004の所有・機密分類・忘却は確定済みのまま、詳細のローカル保存方式はPROVISIONAL。DEC-012／014とbuilder開発ProjectのDEC-040へ区別を追記し、[ロードマップ索引](roadmaps/README.md)にも版未割当てを明記した。


## 人間が管理する階層型AI組織（2026-11-16）

[DEC-074 — 人間が管理する階層型AI組織](DEC-074-human-led-agent-organization.md)は、人間が唯一のManagerとなり、複数の統括AIとその配下の専門Agentが働く長期製品方向をCONFIRMEDとして記録する。組織の規模・階層を端末資源に応じて拡張する方向も確認済み。具体的な構造、技術、版割当は未確定であり、初期0.4のNagi一体によるbuilder Role方式や既存の権限境界を変更しない。

**同日後続確認:** 当面はAI組み込み体験型OSの確立を優先し、階層型組織は非優先の長期ビジョンとする。DEC-074、VISION-001、DEC-021と0.3／0.4+／0.5–1.0 Roadmapへ日付付きで反映した。提供版・階層数・実装順序は未確定であり、既存Gate・builder Roleを変更しない。

## AI Task開始前のWayback復元可能性表示（2026-11-15）

[DEC-026](DEC-026-ai-change-verification-and-recovery.md)に、AI Task開始前にWaybackで復元可能な範囲を示し、部分復元・復元不能・事前判定できない可能性を隠さない方針をCONFIRMEDとして記録した。画面、事前判定の対象範囲、未対応／不明時の具体的な説明・操作導線、数値、適用版はPROVISIONAL。表示は完全なUndoを保証せず、既存の承認・PermissionとRelease Gateも変更しない。DEC-011からTask UIへの接続を追記した。


## Nagi Home・Task進捗・複数端末UI（2026-11-15）

[DEC-009](DEC-009-nagi-first-home-desktop.md)ではNagiに委任したTaskの作業中は原則Nagi Homeを維持する方針を追加し、[DEC-011](DEC-011-task-approval-progress-ui.md)では依頼直後の会話内カードからコンパクトなTask表示へ移り、選択時に詳細を展開する方式Cを確定した。[DEC-010](DEC-010-intent-first-adaptive-ui.md)ではPC・タブレット・モバイルを基本製品目標に含め、端末に応じたUI設計を確認した。PC／Desktop firstは維持し、具体的な対応機器・モバイル提供時期・実装保証は未確定。

## PR #107〜#129の仕様統合（2026-11-16）

[統合先・DEC改番・重複範囲の対応表](PR-107-129-INTEGRATION-2026-11-16.md)を参照。DEC-052は実機候補、DEC-069はFoundation Checklistの正本として維持する。異なる仕様の番号を分離し、採用・暫定・実装・受入を区別する。過去の「main未マージ」は記録当時の履歴であり、現在の仕様は本索引の各DECを参照する。

## #16 Backup / Restoreの決定・変更履歴（2026-11-16）

[DEC-105](DEC-105-user-environment-backup-and-wayback.md)でB方式（ユーザー環境全体のバックアップ）を基本とし、Waybackと分担・連携し、必要に応じC方式を追加選択肢とする方向を記録した。Cの内容・対象、保存方式・鍵・保持・復元詳細・提供版は未確定。DEC-014のWorkspace保存／移行、DEC-026の実際の復元可能性、DEC-004の明示的忘却を維持し、台帳#16と関連ロードマップへ参照を追加。仕様記録であり実装・受入済みではない。

## Foundation #35：Build / Supply Chain（2026-11-16）

[DEC-104](DEC-104-build-supply-chain-baseline-independent-verification-staged.md)で「Bから始めてCを将来の正式目標にする」を記録。Linux kernel／rootfs／Nagi userland／apps等の入力・依存固定、source pin／patch、ライセンス／NOTICE／SBOM、Provenance、再現性、署名・改ざん検証、成果物整合性をBとして先行し、隔離／独立検証・複数Architectureの継続検証・より厳格な改ざん検出／セキュリティ自動化Cへ段階導入する。SBOMによるライセンス適法性の自動認定、LLM推測による証拠・署名検証の代替をしない。具体ツール・規模・予算・日程・閾値・必須Release・Linux target実証はPROVISIONAL。既存Release／Activation／Acceptance Gateは維持する。

**変更履歴:** 2026-11-16 — DEC-104、Foundation #35とロードマップ索引を追加・更新。DEC-100／103等の[PR #139](https://github.com/Roy-To/NagiOS/pull/139)の記録はmain未統合として区別し、同ブランチは変更しない。#35まで35項目の方針検討は一巡した（別PR記録を含む）が、全件main統合・技術詳細・未確定点の解消・実装／Release受入は未完。次段階の優先候補はDEC-104のREVIEW NOTEを参照。

## Entire履歴・CI分離の変更履歴（2026-11-16）

DEC-050の索引・概要を更新し、[実設定と検証の運用記録](../development/entire-private-history.md)を追加した。保存先の方針と、実際のagent記録・送信・取得の実証範囲を区別する。

同日後続確認：既存履歴2件は候補の文脈審査・原本保全後に移行し、本番の自動送信を再開した。実NagiOS worktreeのCodex CLIで記録→Private自動送信→別環境での取得を実証した。Desktop・他agent等の未検証範囲は[運用記録](../development/entire-private-history.md)を参照する。

## 小さな用事への静かな準備・変更履歴（2026-11-16）

**後続決定（2026-11-16／CONFIRMED）:** 期限超過を含む未完了の小さな用事は、AIが許可された情報に基づいて重要度を推定し、ユーザーの明示指定と許可された反応・修正から優先順位・提示方法を学習する。期限超過だけで一律に重大・高優先とせず、重要度が高くても繰り返しの催促や強い割り込みに直結させない。本人の停止・不要・忘却・訂正を優先し、重要度のScore・重み・通知間隔／上限・UI・実装版はPROVISIONAL。詳細は[DEC-008](DEC-008-proactive-behavior.md)。



**追加決定（2026-11-16／CONFIRMED）:** 小さな用事の完了判定はB「確実なら自動完了、曖昧なら確認」。対象と結果が一致する許可済みProvider／Actionの確実な成功結果、または本人の明示的な完了申告に基づき用事を完了にできる。本人申告と実行結果確認は区別し、部分成功・結果不明は完了と誤表示しない。不要／取消／延期と完了を区別し、誤判定は訂正可能にする。完了・不要な用事は再催促しない。判定基準・UI・Schema・実装版はPROVISIONALで、閲覧／監視／再実行の追加権限を生まない。詳細は[DEC-008](DEC-008-proactive-behavior.md)。



**後続決定（2026-11-16／CONFIRMED）:** リマインド方式は**C「状況中心＋期限による優先制御」**を採用。通常の用事は、関連する会話・作業状況や自然な区切りで控えめに思い出させ、明確な期限のある用事は期限を考慮して通知の優先順位を調整する。本人の明示設定と許可された学習結果を優先し、通知がAction実行や追加データ収集の権限にはならない。通知頻度・時刻・UI・判定方式・リリース版はPROVISIONAL。記録先は[DEC-008](DEC-008-proactive-behavior.md)。



**追加決定（2026-11-16）:** 明確な小さな用事は許可されたNagi会話から静かに一時記録、曖昧なら本人に確認、願望や雑談は勝手にToDo化しない。個別の登録コマンド・毎回の確認を基本要件にせず、誤登録や不要な用事は簡単に訂正・削除できる。具体判定・保存方式・タイミング・実装版はPROVISIONAL。記録はメール送信・購入等の実行許可ではない。

[DEC-008](DEC-008-proactive-behavior.md)へ、未処理の雑務・着手しづらいメール・忘れやすい一時的な用事について、B（静かに準備し、区切りに控えめに提案）を既定とする範囲限定の補足を追加した。積極性・タイミング等の設定調整と、許可されたMemory／習慣／採用・無視・拒否等からの個人適応をCONFIRMEDとし、明示設定・拒否・停止・忘却を優先する。草案・情報整理・一時メモで着手負担を下げる方針であり、閲覧・保存・送信・購入・監視の新規権限は生じない。一時的Task／Reminderと長期Canonical Memoryを区別可能にすることを考慮し、具体的な保存・学習・通知・自動生成条件・提供版はPROVISIONAL／OPEN。

既存DEC-008を補足するため新番号は採番せず、[DEC-058](DEC-058-habit-first-learning-and-optional-teach-mode.md)と[DEC-073](DEC-073-proactive-friction-discovery.md)、関連ロードマップへ参照を追加した。PR #107〜#129統合とFoundation Checklist（PR #130）の内容・履歴を保持し、既存Gate・35項目の状態を変更しない。仕様の記録であり、実装・受入完了ではない。

## 2026-11-25 — 純正Officeアプリの標準保存形式

[DEC-126](DEC-126-office-native-format-and-interchange.md)を追加。Writer／Sheets／Slidesは公開Native形式を標準とし、DOCX／XLSX／PPTXの入出力に対応する方向BをCONFIRMED。拡張子・物理形式・Schema・互換範囲・提供版はPROVISIONAL。分散正本と既存Gateを維持。

[DEC-126](DEC-126-office-native-format-and-interchange.md)追補: 共通コンテナによる再利用と用途別内部表現の方向。ZIPを基本候補としJSON・バイナリ等の採用／内部構造／履歴分担はPROVISIONAL。

[DEC-126](DEC-126-office-native-format-and-interchange.md)追補: Office互換は既存OSSを活用し、Nagi独自の開発リソースを編集機能・アプリ間連携へ重点投入する方針をCONFIRMED。個別ライブラリ・採用版・ライセンス／互換性／性能評価・実装順はPROVISIONAL。
