# Nagi OS Product & Experience Checklist — 優先15項目

**作成:** 2026-11-17  
**状態:** 検討テーマ・順序はREVIEW NOTE／PROVISIONAL。PX-15の1.0実用OS B＋将来総合OS Cは高水準完成方針CONFIRMED（詳細PROVISIONAL、進捗PARTIALLY_RECORDED）。他PXの独立した決定／未統合PRの内容は一括変更しない。  
**正本:** [製品仕様索引](README.md)・[運用規則](OPERATING_RULES.md)  
**関連台帳:** [Foundation Checklist（35項目）](NAGI_OS_FOUNDATION_CHECKLIST.md)

## 1. 目的と境界

Foundationの35項目はLinux基盤の起動・保存・保護・デバイス・OS共通サービス・配布などを扱う。本書の15項目は、その上に成立する**利用体験・AIの振る舞い・Memory・純正アプリ・開発者体験・1.0提供範囲**の追加仕様を別チャットで検討するための入口。番号PX-01〜PX-15は既存のFoundation 1〜35とDEC-XXXとは独立する。

**重要:** 既存DECのCONFIRMEDは維持する。本一覧の各論点・優先順はユーザーが確定した新しい製品仕様ではなく、OPEN / PROVISIONALの相談題材。必要な技術選択、実装・Acceptance、Linux実機のPASS、適用版、開始承認は自動的に確定しない。全15件の仕様確定を開発停止条件にしない。AIの権限、Profileの所有、Privacy、Wayback、Release / Activation Gate、既存Workstream境界を緩めない。

## 2. 検討項目

### 第1群：基本操作・ユーザー体験

| ID | 検討テーマ | 追加で決めること | 既存方針の参照 |
|---|---|---|---|
| PX-01 | **Nagi Home / Desktopの詳細仕様** | ログイン直後の画面、会話・アプリ・Taskの入口、Home/Desktopの遷移、AI停止時の操作 | [関連DEC 1](DEC-009-nagi-first-home-desktop.md)・[関連DEC 2](DEC-011-task-approval-progress-ui.md) |
| PX-02 | **Intent-first UI・適応型画面** | 目的に応じたカードやパネル、選択・却下・固定の方法、既存Workspaceの手動配置保護 | [関連DEC 1](DEC-010-intent-first-adaptive-ui.md)・[関連DEC 2](DEC-008-proactive-behavior.md) |
| PX-03 | **Project / Workspaceの統合体験** | Project指定UI、複数Workspaceとの関係、資料・Task・作業状態の再発見と再開 | [関連DEC 1](DEC-055-project-folder-cross-app-context.md)・[関連DEC 2](DEC-014-workspace-state-checkpoints.md) |

### 第2群：AI・Personal Agentの振る舞い

| ID | 検討テーマ | 追加で決めること | 既存方針の参照 |
|---|---|---|---|
| PX-04 | **音声・テキスト対話の統合** | 音声開始・停止、割り込み・誤認識、テキストとの引継ぎ、録音と文字起こしの管理 | [関連DEC 1](DEC-008-proactive-behavior.md)・[関連DEC 2](DEC-011-task-approval-progress-ui.md) |
| PX-05 | **AIの自律実行・承認レベル** | 具体的操作ごとのリスク区分、画面最終確認、未判定時の停止・確認・復元可能性表示 | [関連DEC 1](DEC-001-agent-autonomy-actions.md)・[関連DEC 2](DEC-026-ai-change-verification-and-recovery.md) |
| PX-06 | **長時間Task・並行作業の管理** | Task一覧、Queue・優先順位、割り込み、待機と停止の区別、再開時の確認 | [関連DEC 1](DEC-011-task-approval-progress-ui.md)・[関連DEC 2](DEC-022-builder-interrupted-task-recovery.md) |
| PX-07 | **先回り提案・自発的な働きかけ** | 提案の表示条件・頻度、作業中の抑制、却下・再提示制御、重大警告との区別 | [関連DEC 1](DEC-008-proactive-behavior.md)・[関連DEC 2](DEC-073-proactive-friction-discovery.md) |

### 第3群：Memory・学習・Personalization

| ID | 検討テーマ | 追加で決めること | 既存方針の参照 |
|---|---|---|---|
| PX-08 | **Memoryの自動学習範囲** | 記憶する情報の具体分類、事実と推測の区別、確認が必要な情報、除外条件 | [関連DEC 1](DEC-004-memory-learning-forgetting.md)・[関連DEC 2](DEC-058-habit-first-learning-and-optional-teach-mode.md) |
| PX-09 | **Memoryの保持・忘却・管理体験** | 保持・減衰・Archive・削除の条件とUI、訂正、派生Memory、記憶の履歴 | [関連DEC 1](DEC-004-memory-learning-forgetting.md)・[関連DEC 2](DEC-059-learning-history-insights.md) |
| PX-10 | **Nagiの個人化・対話スタイル** | 口調や説明量の調整、ユーザー設定と学習結果の優先、Profile分離、モデル交換後の継続 | [関連DEC 1](DEC-057-growing-personal-agent-os.md)・[関連DEC 2](DEC-003-memory-classification.md) |

### 第4群：純正アプリ・アプリ横断機能

| ID | 検討テーマ | 追加で決めること | 既存方針の参照 |
|---|---|---|---|
| PX-11 | **純正アプリの標準搭載範囲** | 1.0までに提供する純正アプリと最小機能、必須・任意・後続機能の区別 | [関連DEC 1](DEC-061-first-party-pdf-reader-editor.md)・[関連DEC 2](DEC-062-first-party-utilities.md) |
| PX-12 | **アプリ横断操作・Smart Pasteの詳細** | 初期に対応するアプリ間操作、参照とコピーの選択、出典・更新提案、未対応時のfallback | [関連DEC 1](DEC-025-cross-app-data-reference-and-updates.md)・[関連DEC 2](DEC-056-smart-paste-semantic-transfer.md) |

### 第5群：開発者・配布・1.0完成条件

| ID | 検討テーマ | 追加で決めること | 既存方針の参照 |
|---|---|---|---|
| PX-13 | **Nagiによるアプリ制作体験** | 自然言語による依頼、草案・試用・修正、Memory参照の許可、正式導入の流れ | [関連DEC 1](DEC-071-nagi-native-app-creation-experience.md)・[関連DEC 2](DEC-075-builder-app-trial-and-installation.md) |
| PX-14 | **第三者アプリ開発・移植・配布環境** | 初期SDK・言語・UI部品、移植対象、検証・署名・配布のユーザー/開発者体験 | [関連DEC 1](DEC-017-third-party-developer-ecosystem.md)・[関連DEC 2](DEC-045-nagi-porting-toolkit.md) |
| PX-15 | **Nagi OS 1.0の製品完成基準** | **B＋将来Cの高水準完成方針CONFIRMED:** 正式対応するPC／Target・アプリ／機能の範囲で、日常の代表作業をAIと通常GUIで安全・安定に完結・継続できる実用OS Bを1.0の基準とし、将来は対応端末・アプリ・利用形態を広げる総合OS Cへ。具体的な代表作業・1.0必須アプリ・機器・数値閾値・公開判定・日程はPROVISIONAL | [関連DEC 1](DEC-021-roadmap-0.5-1.0.md)・[関連DEC 2](DEC-074-human-led-agent-organization.md)・[関連DEC 3](DEC-017-third-party-developer-ecosystem.md) |

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

各PXの検討進捗は以下から選ぶ：**OPEN**（未検討）／**IN_DISCUSSION**（検討中）／**PARTIALLY_RECORDED**（部分的にDEC反映）／**RECORDED**（合意範囲をDECに反映）。これは既存DECでの製品採用状態とは別。**本分岐ではPX-15がPARTIALLY_RECORDED（高水準の1.0実用OS B＋将来総合OS CをCONFIRMED、具体的完成条件・版はPROVISIONAL）。** 他PXの決定・未統合PRを一律OPEN／CONFIRMEDに書き換えない。

| 日付 | 変更 | 状態 |
|---|---|---|
| 2026-11-17 | Foundation 35項目から独立した製品体験15項目、既存DEC参照、別チャットでの検討手順を起票 | REVIEW NOTE / PROVISIONAL。新たな15件の製品詳細は未確定 |
| 2026-11-17 | PX-15として「正式対応Targetで日常の代表作業を完了・継続できる実用OS B」を1.0の高水準完成方針とし、将来は総合OS Cへ発展する方向を採用。一般公開品質・公開SDK／配布目標、AI組み込みOS優先と階層組織後順位、通常GUI・Privacy／Permission・Target Acceptanceを保持。 | PARTIALLY_RECORDED：高水準CONFIRMED、代表作業・アプリ・機器／Hardware・性能／品質数値・公開判定・日程はPROVISIONAL |
