# Nagi OS Product & Experience Checklist — 優先15項目

**作成:** 2026-11-17  
**状態:** PX-01〜PX-15のユーザー合意済み高水準方針はCONFIRMED、各PXの具体的UI・技術・性能・適用版等はPROVISIONAL、Checklistの進捗は15項目ともPARTIALLY_RECORDED。検討順序はREVIEW NOTE／PROVISIONAL（実装・Target受入・Release PASSとは別）。  
**正本:** [製品仕様索引](README.md)・[運用規則](OPERATING_RULES.md)  
**関連台帳:** [Foundation Checklist（35項目）](NAGI_OS_FOUNDATION_CHECKLIST.md)

## 1. 目的と境界

Foundationの35項目はLinux基盤の起動・保存・保護・デバイス・OS共通サービス・配布などを扱う。本書の15項目は、その上に成立する**利用体験・AIの振る舞い・Memory・純正アプリ・開発者体験・1.0提供範囲**の追加仕様を別チャットで検討するための入口。番号PX-01〜PX-15は既存のFoundation 1〜35とDEC-XXXとは独立する。

**重要:** 既存DECのCONFIRMEDは維持し、今回のPX-01〜PX-15の採用は明示合意された高水準の製品方向に限定する。対応アプリ・端末、具体UI・選択条件・性能・モデル・技術・実装順序・Releaseへの割当ては、個別DECに別のCONFIRMEDがない限りPROVISIONAL／OPEN。検討テーマの初期順序はREVIEW NOTEであり、全15件の詳細確定を開発停止条件にしない。Linux／Targetの実装PASSや既存Permission・Privacy・Wayback・Release／Activation Gate、Workstream境界は変更しない。

## 2. 検討項目

### 第1群：基本操作・ユーザー体験

| ID | 検討テーマ | 追加で決めること | 既存方針の参照 |
|---|---|---|---|
| PX-01 | **Nagi Home / Desktopの詳細仕様** | **B＋将来Cの高水準方針CONFIRMED:** 会話中心のシンプルな統合Homeを基本とし、会話・アプリ・Task・Workspaceへの入口をつなぐ。将来はProject／Task等に応じた高度なHome適応へ。具体レイアウト・導線・カード数・Nagi Bar位置・AI停止時の詳細UI・技術・提供版はPROVISIONAL | [関連DEC 1](DEC-009-nagi-first-home-desktop.md)・[関連DEC 2](DEC-011-task-approval-progress-ui.md) |
| PX-02 | **Intent-first UI・適応型画面** | **B＋将来Cの高水準方針CONFIRMED:** 作業目的に応じたカード／パネルをユーザーが選択・却下・固定・調整できる適応UI B。将来は許可されたTask／Project文脈・明示設定／採用履歴による高度個人適応C。詳細UI、表示順序・適応条件・学習技術・性能・提供版はPROVISIONAL | [関連DEC 1](DEC-010-intent-first-adaptive-ui.md)・[関連DEC 2](DEC-008-proactive-behavior.md) |
| PX-03 | **Project / Workspaceの統合体験** | **B＋将来Cの高水準方針CONFIRMED:** Project中心に許可された資料・参照・Task・関連Workspaceと作業／再開候補を一元的に把握・操作するBを基本に、将来は文脈・関係・履歴から根拠付きで継続を支援するCへ。統合ビュー、関連付け・一覧・検索・復元導線、技術・性能・版はPROVISIONAL | [関連DEC 1](DEC-055-project-folder-cross-app-context.md)・[関連DEC 2](DEC-014-workspace-state-checkpoints.md) |

### 第2群：AI・Personal Agentの振る舞い

| ID | 検討テーマ | 追加で決めること | 既存方針の参照 |
|---|---|---|---|
| PX-04 | **音声・テキスト対話の統合** | **B＋将来Cの高水準方向CONFIRMED:** 明示操作で始める複数ターンの音声会話、同一対話内の音声／テキスト切替、画面での誤認識訂正。将来はユーザーが明示的に有効化するハンズフリー音声操作へ段階的に拡張。詳細はPROVISIONAL | [関連DEC 1](DEC-008-proactive-behavior.md)・[関連DEC 2](DEC-011-task-approval-progress-ui.md) |
| PX-05 | **AIの自律実行・承認レベル** | **B＋将来Cの高水準方針CONFIRMED:** Action種別・影響範囲・可逆性・検証可能性・承認済みScope等に応じ段階的に確認。将来は許可されたTask文脈・利用者設定・検証状態等に応じ確認体験を高度化。Risk分類・閾値・未判定時の分岐・UI・版はPROVISIONAL | [関連DEC 1](DEC-001-agent-autonomy-actions.md)・[関連DEC 2](DEC-026-ai-change-verification-and-recovery.md) |
| PX-06 | **長時間Task・並行作業の管理** | **B＋将来Cの高水準方針CONFIRMED:** Nagiが管理する長時間・複数Taskの統合Taskセンター型の把握・操作、将来は依存関係・資源・優先度を踏まえる高度な実行調整。Queue／Priority／状態表示の詳細、依存関係、Scheduler、並列度、技術・版はPROVISIONAL | [関連DEC 1](DEC-011-task-approval-progress-ui.md)・[関連DEC 2](DEC-022-builder-interrupted-task-recovery.md) |
| PX-07 | **先回り提案・自発的な働きかけ** | **B＋将来Cの高水準方針CONFIRMED:** 許可された文脈・好みを踏まえ、作業を妨げない自然な提案Bを基本とし、将来は根拠・効果の比較とFeedbackを含む複数候補の高度管理Cへ。頻度・タイミング・抑制・却下／再提示・重要度判定・詳細UI・技術・版はPROVISIONAL | [関連DEC 1](DEC-008-proactive-behavior.md)・[関連DEC 2](DEC-073-proactive-friction-discovery.md) |

### 第3群：Memory・学習・Personalization

| ID | 検討テーマ | 追加で決めること | 既存方針の参照 |
|---|---|---|---|
| PX-08 | **Memoryの自動学習範囲** | **B基本方針CONFIRMED:** 許可された情報を自然に学ぶ／推測は推測として扱う／重要な内容は確認。細分類・確認条件・除外の追加詳細はPROVISIONAL | [関連DEC 1](DEC-004-memory-learning-forgetting.md)・[関連DEC 2](DEC-058-habit-first-learning-and-optional-teach-mode.md) |
| PX-09 | **Memoryの保持・忘却・管理体験** | **B基本方針CONFIRMED:** 自然忘却＋ユーザー管理。将来Cの高度な重複統合・矛盾検出を追加する方向もCONFIRMED。条件・UI・Cの具体方式／版／時期はPROVISIONAL | [関連DEC 1](DEC-004-memory-learning-forgetting.md)・[関連DEC 2](DEC-059-learning-history-insights.md) |
| PX-10 | **Nagiの個人化・対話スタイル** | **B基本方針CONFIRMED:** 利用に応じて口調・説明量・説明方法などが安定的に自然適応。**将来Cの高水準方向CONFIRMED:** Project／Task／作業状況に応じる状況適応へ拡張。UI・モデル／アルゴリズム・判定・評価・提供版／時期はPROVISIONAL | [DEC-057](DEC-057-growing-personal-agent-os.md)・[DEC-003](DEC-003-memory-classification.md)・[DEC-004](DEC-004-memory-learning-forgetting.md) |

### 第4群：純正アプリ・アプリ横断機能

| ID | 検討テーマ | 追加で決めること | 既存方針の参照 |
|---|---|---|---|
| PX-11 | **純正アプリの標準搭載範囲** | **B＋将来Cの高水準方針CONFIRMED:** 日常の主要作業を純正アプリで完結できる実用性・品質を重視する構成Bを基本とし、将来は総合的な純正ソフトウェア環境Cへ。各アプリの1.0必須／任意／後続区分・最小機能・形式・性能・技術・同梱／配布形態・版はPROVISIONAL | [関連DEC 1](DEC-021-roadmap-0.5-1.0.md)・[関連DEC 2](DEC-061-first-party-pdf-reader-editor.md)・[関連DEC 3](DEC-062-first-party-utilities.md) |
| PX-12 | **アプリ横断操作・Smart Pasteの詳細** | **B＋将来Cの高水準方針CONFIRMED:** 通常の転送操作を保ちながら対応方式・制限を分かりやすく案内し、コピー／構造化Object／参照を選べるSmart Paste Bを基本に、将来はProject・Task文脈／変更影響を扱う高度な横断Workflow Cへ。初期対応アプリ・既定方式・詳細UI／Preview・Transfer／fallback制御・技術・性能・提供版はPROVISIONAL | [関連DEC 1](DEC-056-smart-paste-semantic-transfer.md)・[関連DEC 2](DEC-025-cross-app-data-reference-and-updates.md)・[関連DEC 3](DEC-027-public-action-and-scoped-gui-fallback.md) |

### 第5群：開発者・配布・1.0完成条件

| ID | 検討テーマ | 追加で決めること | 既存方針の参照 |
|---|---|---|---|
| PX-13 | **Nagiによるアプリ制作体験** | **B＋将来Cと優先順位を高水準CONFIRMED:** 将来は対話型共同制作Bから個人適応・継続改善Cへ。**当面はローカル完結型IDEを急がず**、ローカルLLMと許可されたCloud開発AI／既存IDE・人間の橋渡し、他OS→Nagi Native移植支援を重視。高度な統合制作環境は1.0計画の最終盤を目安とする低優先の長期方向。具体UI・Memory同意・Cloud Connector・性能・提供版はPROVISIONAL | [関連DEC 1](DEC-071-nagi-native-app-creation-experience.md)・[関連DEC 2](DEC-045-nagi-porting-toolkit.md)・[関連DEC 3](DEC-075-builder-app-trial-and-installation.md) |
| PX-14 | **第三者アプリ開発・移植・配布環境** | **B＋将来Cの高水準方針CONFIRMED:** 既存IDE／公開SDKとPorting Toolkit／任意の許可済みCloud AI・Local LLM橋渡し、Build／Test／Package／独立Target受入を一連の開発者体験Bにまとめ、将来は互換性・継続保守・更新Package／配布準備を支援するCへ。初期言語・Framework／SDK／CLI／Cloud／署名・配布詳細・技術・性能・版はPROVISIONAL | [関連DEC 1](DEC-017-third-party-developer-ecosystem.md)・[関連DEC 2](DEC-045-nagi-porting-toolkit.md)・[関連DEC 3](DEC-021-roadmap-0.5-1.0.md) |
| PX-15 | **Nagi OS 1.0の製品完成基準** | **B＋将来Cの高水準完成方針CONFIRMED:** 正式対応するPC／Target・アプリ／機能の範囲で、日常の代表作業をAIと通常GUIで安全・安定に完結・継続できる実用OS Bを1.0の基準とし、将来は対応端末・アプリ・利用形態を広げる総合OS Cへ。具体的な代表作業・1.0必須アプリ・機器・数値閾値・公開判定・日程はPROVISIONAL | [関連DEC 1](DEC-021-roadmap-0.5-1.0.md)・[関連DEC 2](DEC-074-human-led-agent-organization.md)・[関連DEC 3](DEC-017-third-party-developer-ecosystem.md) |

## 2.1 PX-01〜PX-15の決定記録（2026-11-17）

### PX-01の決定記録（2026-11-17）

- **CONFIRMED:** 対話中心でシンプルな統合Home Bを基本とし、将来はProject・Task等に応じた高度適応Home Cへ発展する高水準方針。正本は[DEC-009](DEC-009-nagi-first-home-desktop.md)のPX-01節であり、[DEC-010](DEC-010-intent-first-adaptive-ui.md)のIntent-firstとWorkspace保護を維持する。
- **既存仕様を維持:** Home標準表示、Homeからアプリ直接起動時のDesktopへの自動切替、委任Task中のHome維持、前回WorkspaceのDesktop復元、会話・Task・アプリ状態保持、AI停止時の通常GUI、Nagi BarからHomeへの復帰、Taskの無断再実行禁止。
- **PROVISIONAL／OPEN:** 具体的なHome／Desktopレイアウト、各導線、カード数、Nagi Bar位置、AI停止時の詳細UI、個人化・技術・適用版・Acceptance。
- **進捗:** PARTIALLY_RECORDED（高水準の採用方針のみDECへ反映）。他PXの並行PRの変更は未統合の可能性があり、今回の分岐で一括上書きしない。

### PX-02の決定記録（2026-11-17）

- **CONFIRMED:** ユーザーが選択・却下・固定・調整できるIntent-first適応型UI Bを基本とし、将来は許可された作業文脈・採用履歴・明示設定による高度な個人適応Cへ発展する高水準の方針。正本は[DEC-010](DEC-010-intent-first-adaptive-ui.md)のPX-02節で、通常の提案の性格は[DEC-008](DEC-008-proactive-behavior.md)を維持する。
- **既存CONFIRMEDを維持:** 新規Workspaceの目的別・複数画面自動配置、既存Workspace・固定ウィンドウ・手動配置の保護、AI不在時の通常GUI、控えめな静かな提案、Action／Permission・Privacy／Memory境界。PX-02のB／将来Cは既存の自動配置の「選択C」「選択B」と別軸。
- **PROVISIONAL／OPEN:** カード／パネルの選択・却下・固定の詳細UI、表示順序・適応／抑制条件、学習／生成の技術、性能・端末・提供Version／時期・Acceptance。
- **進捗:** PARTIALLY_RECORDED（高水準の決定のみDECへ反映）。他PXの独立PRの合意をこのbranchから一括上書きしない。

### PX-03の決定記録（2026-11-17）

- **CONFIRMED:** B＝Project中心の統合作業ビューで、許可された資料・参照・Task・Workspaceと作業／再開候補をまとめて見渡し、選んで操作できる方向。将来C＝許可されたProjectの関係・履歴・状況に基づき、根拠・実際の復元可能性を示す高度な文脈理解／作業継続へ発展する方向。[DEC-055](DEC-055-project-folder-cross-app-context.md)を正本とする。
- **既存仕様の維持:** 普通のフォルダをProjectとして指定可能、別の場所のResourceも元実体を移さず参照可能、Project／Workspaceの別管理と1 Projectから複数Workspaceの利用、DEC-012／014の自動命名・自動保存・名前付きCheckpoint・本人による復元対象／復元先の選択、DEC-065のHandoff・現行権限・同期／本文転送の別承認。Projectの閲覧／Workspace復元による中断Taskの無断再開は禁止。
- **PROVISIONAL／OPEN:** 統合ビューの名称・画面、Project指定・関連付け、資料／Task／Workspace一覧、検索順位・候補・復元／Handoffの導線、UI・Metadata／実装技術・性能・提供Version。
- **進捗:** PARTIALLY_RECORDED（高水準方針の決定をDECに記録。既存のRelease／Activation／Acceptance Gateや実装成否は変更しない）。他PXの独立PRをこの分岐で上書きしない。

### PX-04検討結果（2026-11-17）

- **CONFIRMED（採用済み）:** B「明示操作で始める複数ターンの音声会話と、同じ対話内で音声・テキストを自然に切り替え、誤認識を画面で訂正できるハイブリッド体験」。将来C「ユーザーが明示的に有効化したハンズフリー音声操作へ段階的に拡張する」高水準の方向もCONFIRMED。
- **PROVISIONAL（未決）:** Wake Word採用・常時待受、録音音声／文字起こしの保持・削除、詳細UI、音声本人認証、技術・モデル・Provider、提供版・時期、性能数値。
- **既存境界を維持:** 重要操作の画面上の明示的最終承認、OS Microphone Permission、DEC-002のCloud送信別許可、ユーザー開始会話外での音声割り込み禁止、停止・Text/UI fallback・Task状態保持。音声モードや認識結果はAction権限／本人認証を拡張しない（DEC-001/002/008/011/110）。
- **進捗:** PARTIALLY_RECORDED。基本方針と将来方向をDECに記録し、詳細仕様・実装・提供版の確定を意味しない。

### PX-05の決定記録（2026-11-17）

- **CONFIRMED:** 段階的・リスク対応承認のBを基本とし、将来的に許可された状況・利用者設定・検証状態を踏まえた高度な承認体験Cへ発展する方向。正本は[DEC-001](DEC-001-agent-autonomy-actions.md)のPX-05節。既存のDEC-026「変更の検証・復元可能性を優先する選択A」は別軸として保持する。
- **PROVISIONAL／OPEN:** Risk taxonomy・閾値、未判定時に確認／保留／停止を選ぶ厳密な条件、具体的な画面・判定アルゴリズム・技術・適用Version。
- **進捗:** PARTIALLY_RECORDED（高水準のみDECに記録）。その他のPXの採否、既存Permission／Policy・画面最終承認・Wayback・Workstream Gateをこの変更から確定・変更しない。

### PX-06の決定記録（2026-11-17）

- **CONFIRMED:** Bの統合Taskセンター型を基本とし、将来CのTask依存・資源・優先度・期限に配慮した高度なTaskオーケストレーションへ発展させる高水準方針。正本は[DEC-011](DEC-011-task-approval-progress-ui.md)のPX-06節。既存の前景優先、Task状態と停止・復旧の区別、権限・Gateを維持する。
- **PROVISIONAL／OPEN:** Task一覧・Queue・Priority・状態表示の具体UIと制御、依存関係・Scheduler・並列度／安全検証の詳細、技術・性能・提供版。既存DECでCONFIRMEDの「状態理由を区別する」「明示的に優先度を指定できる」等を撤回する意味ではない。
- **進捗:** PARTIALLY_RECORDED（高水準の採用方針をDECへ記録。機能実装やAcceptanceの完了を意味しない）。他PXの独立PRの内容を今回の分岐で確定・上書きしない。

### PX-07の決定記録（2026-11-17）

- **CONFIRMED:** 許可された文脈・好みに応じた自然な提案Bを基本とし、将来は複数候補・根拠・期待効果・利用者Feedbackを扱う高度な提案管理Cへ発展させる。正本は[DEC-008](DEC-008-proactive-behavior.md)のPX-07節、既存の潜在的不便の発見は[DEC-073](DEC-073-proactive-friction-discovery.md)を維持。
- **PROVISIONAL／OPEN:** 頻度・提示タイミング・作業中の抑制条件、保留・却下／再提示の具体制御、候補重要度・便益の評価、通知／UI、技術・提供版。
- **進捗:** PARTIALLY_RECORDED（基本方針のみDEC反映）。DEC-008の控えめな先回りと重大警告の別扱い、DEC-073の根拠・不確かさ・Memory／Privacy／Permission、Workflow／Actionの個別承認を維持。他PXの並行PRは今回のbranchで上書きせず、統合時に照合する。

### PX-08検討結果（2026-11-17）

- **CONFIRMED（採用済み）:** バランス型B案。許可された情報を普段の利用から自然に学び、観測・事実・推測を混同せず、重要な内容はユーザーに確認する。正本は[DEC-004](DEC-004-memory-learning-forgetting.md)のPX-08節。
- **PROVISIONAL（未決）:** 具体的な記憶対象分類、重要度判定、どの段階で確認するか、除外の追加詳細、状態／確信度、保持・実装・適用版。既存のMemory保護・忘却、Permission／Source・Profile境界、Workflow承認は変更しない。
- **進捗:** PARTIALLY_RECORDED。基本方針をDECに記録するが、追加詳細や実装の完了を意味しない。

### PX-09検討結果（2026-11-17）

- **CONFIRMED（採用済み）:** B案「自然忘却＋ユーザー管理」。Nagiが通常はMemoryを自然に整理し、ユーザーは必要なときに記憶を確認・訂正・保護・忘却できる。将来C案の高度な重複統合・矛盾検出を追加する方向もCONFIRMED。正本は[DEC-004](DEC-004-memory-learning-forgetting.md)のPX-09節。
- **PROVISIONAL（未決）:** 重要度・利用度判定、Decay／Archive／削除条件と閾値、約7／90／180／365日の提案値、管理UI・履歴表示、Cのアルゴリズム・自動適用か提案か・確認条件、実装方式・提供版・時期。既存の明示的忘却、ユーザー訂正、Memory保護、Source／Profile／Permission境界、Release／Activation／Acceptance Gateは変更しない。
- **進捗:** PARTIALLY_RECORDED。基本方針と将来方向を記録し、条件・UI・実装の確定を意味しない。

### PX-10の高水準決定記録（2026-11-17）

- **対象:** **Nagiの個人化・対話スタイル** | **B基本方針CONFIRMED:** 利用に応じて口調・説明量・説明方法などが安定的に自然適応。**将来Cの高水準方向CONFIRMED:** Project／Task／作業状況に応じる状況適応へ拡張。UI・モデル／アルゴリズム・判定・評価・提供版／時期はPROVISIONAL

**詳細:** 関連DECのCONFIRMEDとPROVISIONALを参照し、追加の仕様・実装・Target Acceptance・提供版をこの要約から確定しない。

### PX-11の決定記録（2026-11-17）

- **CONFIRMED:** 高水準の製品体験としてB＝日常の主要作業をOS純正アプリと通常GUIで実用的に完結できる品質重視の構成を基本とし、将来はC＝総合的な純正ソフトウェア環境へ段階的に発展する。正本は[DEC-021](DEC-021-roadmap-0.5-1.0.md)のPX-11節。
- **既存CONFIRMEDを維持:** [純正アプリ実装仕様](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)のPhase 1〜3、[DEC-061](DEC-061-first-party-pdf-reader-editor.md)の純正PDF Reader／Editorと基本機能優先、[DEC-062](DEC-062-first-party-utilities.md)の画像・取り込み・アーカイブ・メディア4分野すべての採用、[DEC-063](DEC-063-phone-contacts-messaging.md)の端末能力に応じる通信機能採用、AI非依存の基本操作、[DEC-017](DEC-017-third-party-developer-ecosystem.md)／DEC-021の1.0公開開発者環境・配布基盤目標を維持。いずれも全アプリの1.0同梱確約に読み替えない。
- **PROVISIONAL／OPEN:** 1.0での各アプリの必須／任意／後続リスト、最小機能・対応形式／機器・性能／品質・具体技術・搭載／配布形態・Version／時期。AI連携や高度編集は既存権限と成熟度に従う。
- **進捗:** PARTIALLY_RECORDED（高水準方向のみDECへ記録し、1.0の最終構成／実装・Acceptance完了は未認定）。他PXの独立PRの変更をこのbranchで一括上書きしない。

### PX-12の決定記録（2026-11-17）

- **CONFIRMED:** B＝対応可能な貼り付け方法と差異・制限を案内し、本人が方式を選べる統合Smart Paste。将来C＝許可されたProject／Task文脈とProvenance・変更影響を踏まえた高度なアプリ横断Workflowへ発展する高水準の方向。正本は[DEC-056](DEC-056-smart-paste-semantic-transfer.md)のPX-12節。
- **既存CONFIRMEDを維持:** DEC-025の共通参照／分散正本Aと変更検知・提案A、Scope内承認済みTaskの条件付き自動更新、DEC-056のSheets→Slides・Albert→Writer・画像→Slidesの意味付き転送構想と従来GUI操作、DEC-027の正規Public Action優先Aと明示許可・限定ScopeのGUI fallback。更新元／先Revision、Permission／Policy、Cloud送信別許可、Task停止を守る。
- **PROVISIONAL／OPEN:** 初期の対応アプリ組合せ、コピー／参照の既定選択、貼り付け方法の具体UI・Preview、Transfer Schema、非対応時のfallback技術／制御詳細、性能・実装／提供版・Acceptance。
- **進捗:** PARTIALLY_RECORDED（高水準方針をDECに記録。アプリ間の具体実装と受入PASSを意味しない）。他PXの未統合PRをこのbranchから上書きしない。

### PX-13の後続決定記録（2026-11-17）

- **CONFIRMED — 将来の体験:** B案＝対話しながら設計案・修正・可能なPreview／安全な試用・採用判断へ進む共同制作体験。将来C案＝本人が許可したMemory・保存済み開発Projectを活かして継続改善を提案できる方向。既存のDEC-070／071・builder Role／試用／正式導入・共有／更新の別承認を維持する。
- **CONFIRMED — 現在の優先度:** Local LLMの独力によるアプリ全面生成を当面の必須成果とせず、既存IDE・人間・許可されたCloud開発AIとNagi SDK／Porting Toolkitの情報を安全に橋渡しし、Windows／Linux等からのNagi Native移植支援を重視する。高度な統合制作環境は1.0計画最終盤を目安にした低優先長期目標で、**1.0の特定版への搭載確約・Release Gateではない**。既存の1.0公開開発者環境／配布基盤目標は変更しない。
- **PROVISIONAL／OPEN:** 制作画面・Preview・修正UI、Memory利用許可UI、Cloud Provider／Connector・選択／送信範囲・モデル／交換方式、初期移植対象・SDK技術・Target検証、性能・提供Version／時期・Acceptance。DEC-002のCloud事前許可、DEC-075の試用／正式導入の別承認は未決定へ戻さない。
- **進捗:** PARTIALLY_RECORDED（高水準方針と優先度のみ関連DECに記録。機能実装・統合テスト完了・Release決定ではない）。他PXの独立PRの合意をこのbranchで上書きしない。

### PX-14の決定記録（2026-11-17）

- **CONFIRMED — B＋将来C:** B＝一般的なIDE／開発ホスト、正規Nagi SDKとPorting Toolkitの診断・段階的移植・検証、Local LLMと任意のCloud開発AIの安全な橋渡し、Build／Test／Package／Nagi Target受入への分かりやすい一貫した開発者体験。将来C＝SDK／依存変更影響・互換性・継続保守・更新Package／配布準備を支援する長期方向。正本は[DEC-017](DEC-017-third-party-developer-ecosystem.md)、Toolkitの境界は[DEC-045](DEC-045-nagi-porting-toolkit.md)。
- **既存CONFIRMEDの維持:** Nagi Native Apps専用、Rust SDK中心・必要な安定C ABI、Level 0〜3、Windows／Linuxの同等重点／Android将来・Apple系後順位、PT-01〜06非AI基盤、PX-13のCloud橋渡し優先・統合型初心者IDEの後順位、DEC-002のCloud送信事前許可、Host試験／Nagi Target Acceptanceの独立、Package署名・インストール・共有・公開の別承認、DEC-017／021の1.0開発者環境／配布基盤という高水準目標を維持。
- **PROVISIONAL／OPEN:** 最初の対象言語・Framework／UI部品、SDK／ABI・CLI／IDE接続の詳細、Cloud Provider／Connector／データ受渡し、PT成果／診断形式、具体的なPackage／署名／配布技術と手順、実測性能・品質・担当／提供Version・Acceptance。
- **進捗:** PARTIALLY_RECORDED（高水準方向のみDECへ反映。Porting Toolkit実装完了、Target受入、1.0公開判断・Release Gateの達成は意味しない）。他PXの未マージPRにある確定判断はこのbranchで一括上書きしない。

### PX-15の決定記録（2026-11-17）

- **CONFIRMED — Bを1.0の高水準基準:** 正式対応を明示したTarget環境・機能で、代表的な日常作業をNagiの適切なAI支援と通常GUIによって安全・安定に完了でき、継続利用・結果確認・障害時の復旧を適切に扱える実用OS。一般公開品質・実証済み対応範囲を機能数より重視する。**将来C**として端末・周辺機器・純正／第三者アプリ・利用形態を拡大する総合OSへ発展する。[DEC-021](DEC-021-roadmap-0.5-1.0.md)のPX-15節を正本とする。
- **維持する既存CONFIRMED:** 改訂0.3と0.5〜1.0 Master Roadmapの一般公開品質／安定性／Security／互換性／Developer experience、[DEC-017](DEC-017-third-party-developer-ecosystem.md)の公開SDK・開発者／配布基盤1.0目標、[DEC-074](DEC-074-human-led-agent-organization.md)のAI組み込みOS優先・階層Agent組織長期後順位、AI不在時通常GUI・a11y基盤・Memory／Workspace・Permission／Privacy、TargetでのAcceptanceをHost-onlyから区別する既存境界。0.1〜0.4と後続版のRelease／Activation／Acceptance GateとWorkstream開始条件は変更しない。
- **PROVISIONAL／OPEN:** 代表作業シナリオと必須純正アプリ／個別機能、正式対応PC・周辺機器とHardware profile、性能・品質／安定性／Security・a11yの数値・許容不具合、具体的な試験マトリクス・公開判定手順／Support policy・Release日程。Master Roadmapのcandidate evidence（§6）はまだ確定チェックリストではない。
- **進捗:** PARTIALLY_RECORDED（高水準完成方針のみDECに保存。実装・Target受入／1.0 Release PASSを認定しない）。他PXの独立PRの合意内容をこのbranchでは上書きしない。

## 3. 既存のCONFIRMEDとの関係（変更しない）

- **UI / Workspace:** Nagi-firstと通常GUIの共存、Home / Desktop、既存Workspaceの手動配置保護、新規Workspaceの意図に応じた配置、ProjectとWorkspaceの区別は既存決定。PX-01〜03は画面・遷移・具体操作の追加検討（DEC-009/010/012/014/055）。
- **Agent / 承認:** 承認済みTask内の限定的な自律性、権限の都度検証、前景会話優先、危険操作の保護、再実行禁止、控えめな先回りは既存決定。PX-04〜07は詳細体験（DEC-001/008/011/016/022/026/073）。
- **Memory:** 重要情報の選択的な自動学習、自然な習慣学習、モデル非依存の継続、訂正・忘却、学習履歴の可視化は既存決定。PX-08〜10は具体分類や体験。DEC-004にある日数・回数の提案値はPROVISIONAL（DEC-003/004/057/058/059）。
- **純正アプリ・連携:** 既存0.2アプリの定義、追加純正ユーティリティ、共通参照、Smart Paste、正規Public Action優先は既存決定。PX-11〜12は正式搭載範囲と代表ワークフローの明確化（DEC-023/025/027/056/061/062）。
- **開発者 / 1.0:** Nagi Native Apps、移植支援、初心者向けアプリ制作、builderの隔離・試用・正式導入の承認、公開SDK/配布環境という方向は既存決定。PX-13〜15は到達範囲・品質・具体操作（DEC-017/021/045/070/071/075/076）。
- **当面の優先:** AI組み込みOS体験を優先し、階層型Agent Organizationは長期・非優先。初期0.4の単一Nagi builder Roleを変更しない（DEC-074/015）。

## 4. 項目別チャットの進め方

**推奨検討順（REVIEW NOTE）:** PX-08〜10（Memory）→ PX-04〜07（Agent）→ PX-01〜03（UI）→ PX-11〜15（アプリ／1.0）。IDは変更しない。

各チャットで、まず最新の[README](README.md)、[OPERATING_RULES](OPERATING_RULES.md)、本Checklistの対象PX、関連DECと[roadmaps索引](roadmaps/README.md)を実際に読む。既存CONFIRMED、具体的に追加で決める論点OPEN、実装依存PROVISIONALを整理し、未決の部分だけ2〜3案を比較する。本人が明確に採用した範囲だけをCONFIRMEDにする。合意後は既存DEC更新または新DECを作成し、README索引・変更履歴と本台帳の状態を更新し、**公開Roy-To/NagiOSへのGit commitを検証した場合だけ「保存済み」**と報告する。読めない・書けない場合は明記して未確認/未保存とする。

### 新規チャットに貼る共通指示

> Nagi OS Product & Experience Checklist の **PX-XX** について仕様を検討したい。公開GitHub Roy-To/NagiOS の docs/product-decisions/README.md、OPERATING_RULES.md、NAGI_OS_PRODUCT_EXPERIENCE_CHECKLIST.md の該当項目、関連する最新DECとroadmapsを実際に確認して。既存CONFIRMEDを変更せず、未決事項だけを抜き出し、具体例とA/B/C案・メリット・デメリットを提示して相談しよう。私が決定したら、その範囲だけをCONFIRMEDとして該当DEC、索引と変更履歴、Checklistを更新し、公開リポジトリへコミットして検証して。未決はOPEN/PROVISIONAL、GitHub反映ができなければ未保存と報告して。既存のRelease/Activation/Acceptance Gateは変更しない。

## 5. 進捗と変更履歴

各PXの検討進捗は以下から選ぶ：**OPEN**（未検討）／**IN_DISCUSSION**（検討中）／**PARTIALLY_RECORDED**（高水準方針のDEC反映、詳細未確定）／**RECORDED**（合意範囲をDECへ反映）。既存DECの製品採用、実装・Acceptanceとは別軸。**PX-01〜PX-15はすべてPARTIALLY_RECORDED（高水準方針CONFIRMED、詳細PROVISIONAL）**。実際の機能完了や新しいRelease Gateは認めない。

| 日付 | 変更 | 状態 |
|---|---|---|
| 2026-11-17 | Foundation 35項目から独立した製品体験15項目、既存DEC参照、別チャットでの検討手順を起票 | REVIEW NOTE / PROVISIONAL。新たな15件の製品詳細は未確定 |
| 2026-11-17 | PX-01の対話中心の統合Home Bを基本とし、将来のProject／Taskに応じた高度適応Home Cへ発展する高水準方針を決定。DEC-009／010の既存CONFIRMEDとAI不在時の通常GUIを維持。 | PARTIALLY_RECORDED：高水準CONFIRMED、具体画面・導線・カード数・Bar位置・AI停止時UI・技術・提供版はPROVISIONAL |
| 2026-11-17 | PX-02のユーザー主導の適応型UI Bを基本、将来Cの高度な個人適応UIへ発展する高水準方針を追加。DEC-010の新規Workspace自動配置／既存配置保護、DEC-008の静かな提案・権限境界を維持。 | PARTIALLY_RECORDED：高水準CONFIRMED、選択／却下／固定UI・表示順序・適応条件・学習技術・性能・提供版はPROVISIONAL |
| 2026-11-17 | PX-03のProject中心の統合作業ビューBと将来の高度な文脈理解・作業継続Cを高水準方針に採用。既存Project／Workspace分離、Resource／Memory・Workspace復元・Handoffの権限境界を維持。 | PARTIALLY_RECORDED：高水準CONFIRMED、一覧・関連付け・検索／復元導線・UI・技術・性能・提供版はPROVISIONAL |
| 2026-11-17 | PX-04のB基本方針と将来Cの高水準方向をDEC-008／011に反映。詳細・版・時期・性能数値はPROVISIONAL。既存の画面最終承認、Microphone Permission、Cloud送信別許可、音声割り込み抑制、停止・Text/UI fallback・Task状態保持を維持 | PARTIALLY_RECORDED：基本方針／将来方向CONFIRMED、詳細PROVISIONAL |
| 2026-11-17 | PX-05でB（段階的・リスク対応承認）を基本、将来C（状況適応型）を製品方向に採用。既存のAction権限・重要操作の画面最終承認・DEC-026の選択A・復元可能性表示を維持。 | PARTIALLY_RECORDED：高水準CONFIRMED、閾値・未判定時の詳細／UI・技術・版はPROVISIONAL |
| 2026-11-17 | PX-06の統合TaskセンターBと将来の高度オーケストレーションCを高水準方針として採用。既存の前景優先・待機と停止／復旧の区別・Scope／Permission・Gateを維持。 | PARTIALLY_RECORDED：高水準CONFIRMED、Queue・Priority・表示・依存関係・Scheduler・並列度・実装・版はPROVISIONAL |
| 2026-11-17 | PX-07のB（文脈・好みに応じ自然に提案）と将来C（候補の整理・根拠／期待効果の比較・Feedback活用）を高水準方針として決定。DEC-008／073の既存CONFIRMEDを維持。 | PARTIALLY_RECORDED：高水準CONFIRMED、頻度・提示条件・抑制・却下／再提示・重要度判定・UI・技術・版はPROVISIONAL |
| 2026-11-17 | PX-08でバランス型B基本方針を採用。許可された情報の自然な学習／推測の明示的な区別／重要な内容の本人確認をDEC-004に反映。既存Memory／Privacy／忘却・Gateを維持 | PARTIALLY_RECORDED：B基本方針CONFIRMED、細部PROVISIONAL |
| 2026-11-17 | PX-09でB「自然忘却＋ユーザー管理」と将来C「高度な重複統合・矛盾検出」の方向をCONFIRMEDとしてDEC-004／059に反映。閾値・UI・Cの具体方式／版／時期はPROVISIONAL、既存のSource／Profile／Permission・忘却・Gateを維持 | PARTIALLY_RECORDED：基本方針／将来方向CONFIRMED、詳細PROVISIONAL |
| 2026-11-17 | PX-10のB基本方針と将来Cの高水準な状況適応方向をDEC-057へ記録 | PARTIALLY_RECORDED。具体的なUI・技術・判定・評価・提供版／時期はPROVISIONAL |
| 2026-11-17 | PX-11で日常の主要作業を純正アプリで実用的に完結できる品質優先Bと将来の総合的な純正環境Cを高水準方針として採用。Phase 1〜3・PDF／ユーティリティ採用・AI非依存・1.0開発者／配布目標を維持。 | PARTIALLY_RECORDED：高水準CONFIRMED、個別1.0必須／任意／後続・最小機能・形式・性能・技術・搭載／配布形態・版はPROVISIONAL |
| 2026-11-17 | PX-12のB（対応可能な貼り付け方式の案内・ユーザー選択）＋将来C（Project／Taskの関係・変更影響を扱う高度なアプリ横断Workflow）を高水準で採用。DEC-025方式A／DEC-027方式AとDEC-056の従前の意味付き転送・通常操作を維持。 | PARTIALLY_RECORDED：高水準CONFIRMED、初期の対象App組合せ・コピー／参照既定・UI／Preview・Transfer／fallback制御・性能・技術・提供版はPROVISIONAL |
| 2026-11-17 | PX-13の対話型共同制作B＋将来の個人適応・継続制作Cを高水準として維持。独立したローカル完結制作環境は後順位とし、ローカルLLMが許可されたCloud開発AI・既存IDE／人間とNagi SDK・移植診断を橋渡しする支援、他OSアプリ移植を当面優先。 | PARTIALLY_RECORDED：高水準方針・優先度CONFIRMED。1.0計画最終盤は目安で確定版ではない。詳細UI・Cloud連携・モデル・性能・技術・版はPROVISIONAL |
| 2026-11-17 | PX-14で既存IDE／公開SDK・Toolkit・Local LLM／許可済みCloud AIの橋渡し、段階的Build／Test／Package／独立Target検証をつなぐBと、将来の互換性・継続保守・更新／配布支援Cを高水準採用。既存Nagi Native・移植対象優先・開発者目標・権限Gateを維持。 | PARTIALLY_RECORDED：高水準CONFIRMED。初期言語・Framework・SDK／CLI・Cloud／Package／署名・配布・性能・版はPROVISIONAL |
| 2026-11-17 | PX-15として「正式対応Targetで日常の代表作業を完了・継続できる実用OS B」を1.0の高水準完成方針とし、将来は総合OS Cへ発展する方向を採用。一般公開品質・公開SDK／配布目標、AI組み込みOS優先と階層組織後順位、通常GUI・Privacy／Permission・Target Acceptanceを保持。 | PARTIALLY_RECORDED：高水準CONFIRMED、代表作業・アプリ・機器／Hardware・性能／品質数値・公開判定・日程はPROVISIONAL |
| 2026-11-17 | PX-01〜PX-15の独立した高水準CONFIRMEDを統合。各元PRのDECの判断を保持し、共通Checklist・READMEの重複変更を整理した。 | 全15項目PARTIALLY_RECORDED／詳細PROVISIONAL。新たな実装・Target／Release PASSなし。 |
