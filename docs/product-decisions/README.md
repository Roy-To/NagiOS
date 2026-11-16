# NAGI Product Decisions — 決定済み仕様の保管庫

**目的:** チャット上で合意したNAGIの製品挙動・AIポリシー・ユーザー体験を、消えないMarkdownとして継続蓄積する。

**作成日:** 2026-10-25  
**管理場所:** `docs/product-decisions/`  
**対象:** 0.2以降の製品判断（実装前の要求を含む）

## 正本と記録のルール(2026-11-15、[DEC-050](DEC-050-public-specification-repository.md))

**相談はチャット、決定済み仕様の正本はこの公開リポジトリ `Roy-To/NagiOS`。** 詳細は [OPERATING_RULES.md](OPERATING_RULES.md)。2026-11-15に、それまで非公開リポジトリ `Roy-To/NagiOS-internal` に保存していた仕様・構想をここへ移した。各文書に残る「Privateに保存」「Publicへ転載しない」は移管前の記述で、DEC-050が優先する。

- [ChatGPTプロジェクト指示(登録用)](CHATGPT_PROJECT_INSTRUCTIONS.md) — ChatGPTのNagi_chatプロジェクトの指示へコピーして適用。**ファイル保存だけでは設定は変わらない。**
- [Codex開始・引き継ぎ規則](CODEX_START_HERE.md) — このリポジトリで動くCodexが決定事項を参照・記録する手順。
- [0.3改訂・0.4+構想・0.5〜1.0 Master Roadmap](roadmaps/README.md) — ロードマップ案。

ChatGPT・Codexとの会話で仕様が決まったら、`CONFIRMED`か`PROVISIONAL`かを分け、該当DECファイルと索引を更新して**このリポジトリにcommitし、確認できた場合だけ「保存済み」**と報告する。認証情報・個人情報・ユーザー名入りのパス・チャット識別子は書かない。

## 8GB RAM級での軽快な純正アプリと実用的ローカルAI連携（2026-11-16）

[DEC-020 — AIのOS操作経路と計算資源](DEC-020-ai-os-interaction-path.md)へ、RAM 8GB級の限られた端末でも純正アプリの日常操作と必要十分な軽量・量子化ローカルLLMによるアプリ間AI連携が**実用的に共存することを目指す**製品目標を追記。単なる起動確認ではなく前景UIの応答性とAI Taskの品質を実測する方針を再確認した。モデル選択・入替えとNagiのMemory継続、AI不在時の通常操作、Permissionと既存Release／Activation境界を維持する。8GBでの快適動作は**未実証**で、代表端末・実測数値・モデル構成・適用版はPROVISIONAL。本記録は性能保証でも実装完了でもない。

複数の量子化モデルをストレージに保存し、必要時にのみメモリへロードして、利用可能な標準・軽量モデルを切り替えられる方向も**CONFIRMED**。負荷・Taskの必要能力に応じた**自動的な軽量モデルへの切替を目指し、初期に難しい場合も手動切替を確保する**。ユーザーが明示固定したモデル・自動切替の無効設定、Task品質、権限・明示停止、Nagi Memoryの継続を尊重する。自動化の採用時期、RAM閾値、切替中断や再開、完全に切れ目のない応答、適用版は**PROVISIONAL**であり、8GBでの達成性能は未実証。

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
| DEC-001 | [AIの自律性とAction](DEC-001-agent-autonomy-actions.md) | CONFIRMED | 2026-10-25 |
| DEC-002 | [クラウドAI・外部情報共有](DEC-002-cloud-ai-data-sharing.md) | CONFIRMED | 2026-10-25 |
| DEC-003 | [Memoryの機密分類と所有範囲](DEC-003-memory-classification.md) | CONFIRMED | 2026-10-25 |
| DEC-004 | [Memory保存・学習・自然忘却](DEC-004-memory-learning-forgetting.md) | CONFIRMED（一部数値は暫定） | 2026-10-25 |
| DEC-005 | [Files・AlbertとMemoryの連動](DEC-005-files-albert-memory-lifecycle.md) | CONFIRMED（一部パラメータは暫定） | 2026-10-25 |
| DEC-006 | [日常利用シナリオ](DEC-006-daily-use-scenarios.md) | CONFIRMED | 2026-10-25 |
| DEC-007 | [ロードマップ・版境界との整合](DEC-007-roadmap-alignment.md) | REVIEW NOTE | 2026-10-25 |
| DEC-008 | [標準的な働きかけと通知](DEC-008-proactive-behavior.md) | CONFIRMED | 2026-10-29 |
| DEC-009 | [Nagi-first操作思想とNagi Home / Desktop](DEC-009-nagi-first-home-desktop.md) | CONFIRMED | 2026-11-15 |
| DEC-010 | [Intent-first UIと作業別レイアウト提案](DEC-010-intent-first-adaptive-ui.md) | CONFIRMED（複数端末を見据えたUI方針／細部未確定） | 2026-11-15 |
| DEC-011 | [Task承認・進捗UI、`builder`のRole切替・別会話・追加指示表示](DEC-011-task-approval-progress-ui.md) | CONFIRMED（承認UI／Task開始前のWayback復元可能性表示／Role切替C／前景会話優先C／追加指示分類表示C／委任Task進捗C）／PROVISIONAL（UI細部） | 2026-11-15 |
| DEC-012 | [Workspaceの自動命名・整理と自然言語検索](DEC-012-workspace-naming-organization.md) | CONFIRMED（一部詳細未確定） | 2026-11-04 |
| DEC-013 | [Hardware-Adaptive Memory Engineの先行設計と性能評価](DEC-013-lightweight-memory-engine-benchmark.md) | CONFIRMED（適応方針・評価方針）／profile・技術選択PROVISIONAL | 2026-11-05 |
| DEC-014 | [Workspace保存・チェックポイント・横断検索・選択式端末移行／同期・競合／中断復旧・暗号化Export / Importと鍵／データ管理](DEC-014-workspace-state-checkpoints.md) | CONFIRMED（詳細未確定） | 2026-11-05 |
| DEC-015 | [`builder` Roleの初期Nagi実行・切替・別会話継続・追加依頼分類](DEC-015-builder-role-review.md) | CONFIRMED（Role名／初期実行C／自動切替C／会話継続C／追加・別件依頼C）／PROVISIONAL（Runtime・Queue詳細） | 2026-11-05 |
| DEC-016 | [`builder`のScope内自律実行・リスク別完了・追加指示の承認境界](DEC-016-builder-autonomy-risk-based-completion.md) | CONFIRMED（自律B／完了判定C／再試行C／追加指示分類Cと承認・Scope制限）／PROVISIONAL（実装詳細） | 2026-11-05 |
| DEC-017 | [第三者アプリ開発・参入環境と既存アプリ移植](DEC-017-third-party-developer-ecosystem.md) | CONFIRMED（Nagi Native Apps専用・移植支援：PC Windows／Linux重点、将来Mobile Android優先検討、Apple系後順位・段階的統合）／PROVISIONAL（具体技術・対応範囲・実装時期・工数） | 2026-11-13 |
| DEC-018 | [`builder`の隔離編集と条件付き自動反映](DEC-018-builder-isolated-edit-conditional-apply.md) | CONFIRMED（隔離編集C、正式アプリ更新は別承認・DEC-035）／PROVISIONAL（実装詳細） | 2026-11-05 |
| DEC-019 | [`builder`作業データの状態別整理](DEC-019-builder-work-data-cleanup.md) | CONFIRMED（整理C・初期Nagi Role切替）／PROVISIONAL（保持期間等） | 2026-11-05 |
| DEC-020 | [AIのOS操作経路・適応型推論と人間優先の資源配分](DEC-020-ai-os-interaction-path.md) | CONFIRMED（Context／Event駆動、前景優先・適応推論、8GB級での日常操作＋ローカルAI連携の実用性目標、複数モデル導入／軽量モデルへの適応切替と手動選択、`builder` Resource待機継続C）／PROVISIONAL（モデル・実装・性能値・適用版） | 2026-11-16 |
| DEC-021 | [0.5〜1.0ロードマップ境界と段階案](DEC-021-roadmap-0.5-1.0.md) | CONFIRMED（継承方針・改訂0.3の1.0目標・DEC-017の1.0目標）／PROVISIONAL（版別範囲・順序・実装詳細） | 2026-11-04 |
| DEC-022 | [`builder`の中断・再起動後の作業復旧](DEC-022-builder-interrupted-task-recovery.md) | CONFIRMED（復旧C、資源待機とクラッシュ復旧の区別、初期Nagi Role切替）／PROVISIONAL（技術詳細） | 2026-11-05 |
| DEC-023 | [作業・データ中心のOS基本思想](DEC-023-work-data-centric-os.md) | CONFIRMED（作業中心・従来操作・Permissionの維持）／PROVISIONAL（連携方式・適用版） | 2026-11-05 |
| DEC-024 | [TypeSafe AI Jev名称の訂正と`builder` Role・DecisionProvider方針](DEC-024-external-jev-reference-and-builder-label-correction.md) | CONFIRMED（名称訂正、Role切替C、DecisionProvider優先）／PROVISIONAL（将来Agent・専用判断モデル） | 2026-11-05 |
| DEC-025 | [アプリ横断の共通参照と関連データ更新提案](DEC-025-cross-app-data-reference-and-updates.md) | CONFIRMED（共通参照・分散正本・変更検知と提案・承認済みTask内の自動更新）／PROVISIONAL（実装・適用版） | 2026-11-05 |
| DEC-026 | [AIによる変更の検証可能性と復元可能性](DEC-026-ai-change-verification-and-recovery.md) | CONFIRMED（リスク別検証・保護、AI Task開始前のWayback復元可能性表示、`builder`製アプリ正式更新Cとの整合）／PROVISIONAL（UI・事前判定範囲・未対応／不明時の扱い・数値・実装・適用版） | 2026-11-15 |
| DEC-027 | [正規Public Action APIと限定的なGUI fallback](DEC-027-public-action-and-scoped-gui-fallback.md) | CONFIRMED（Public Action優先・明示許可下の限定fallback）／PROVISIONAL（具体方式・制御範囲・適用版） | 2026-11-05 |
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
| DEC-045 | [Nagi Porting Toolkit — PT-01〜06 診断・生成・移植・検証・継続評価](DEC-045-nagi-porting-toolkit.md) | CONFIRMED（Windows／Linux重点、PT-01〜06設計方針、AI非必須／開発手段を選択可能、PT-06は手動対Toolkit等の実移植比較と改善循環）／PROVISIONAL（実装・サンプル・指標／受入・担当・版・16GB性能評価） | 2026-11-13 |
| DEC-046 | [共同開発者の役割と担当領域の自律性](DEC-046-co-developer-governance.md)／[PT-02〜PT-06担当案](../development/CO_DEVELOPER_ALLOCATION_PROPOSAL.md) | CONFIRMED（役割分担）／PROVISIONAL・未決定（個別担当・path・権限・着手） | 2026-11-15 |
| DEC-047 | [Linux移行レーンの作業場所・操作許可・統合先](DEC-047-linux-migration-workspaces-and-integration.md) | CONFIRMED（lane別worktree・操作範囲・統合先） | 2026-11-14 |
| DEC-048 | [DeepSeekによるNagiコード等レビューへの継続承認](DEC-048-deepseek-review-authorization.md) | CONFIRMED（全lane・全payloadのDeepSeekレビュー承認） | 2026-11-14 |
| DEC-049 | [速度優先の開発手続き](DEC-049-development-process-speed-priority.md) | CONFIRMED（開発手続きの速度優先・継続制約） | 2026-11-14 |
| DEC-050 | [仕様・構想の正本を公開リポジトリへ移す](DEC-050-public-specification-repository.md) | CONFIRMED | 2026-11-15 |
| DEC-051 | [レビュー方式A/B](DEC-051-review-methods.md) | CONFIRMED（現在は方式A） | 2026-11-15 |
| DEC-052 | [実機テスト端末の候補選定方針](DEC-052-physical-test-device-candidates.md) | CONFIRMED（候補選定のみ：PC＝Raspberry Pi・型番未指定／タブレット＝PineTab2／スマートフォン・モバイル＝未選定）／対応・購入・時期は未確定 | 2026-11-16 |
| DEC-053 | [AI Task開始前のWayback復元可能性表示](DEC-053-ai-task-recovery-preview.md) | CONFIRMED（Task開始前に復元可能性を表示する高水準の方向）／PROVISIONAL（分類・UI・判定基準・実装・適用版） | 2026-11-15 |
| DEC-055 | [Projectフォルダ中心の純正アプリ横断AI体験](DEC-055-project-folder-cross-app-context.md) | CONFIRMED（Project指定・横断Task・Workspace分離・Project文脈）／PROVISIONAL（名称・技術・実装版） | 2026-11-16 |
| DEC-056 | [Smart Paste — 意味付きコピー／ドラッグでの純正アプリ横断体験](DEC-056-smart-paste-semantic-transfer.md) | CONFIRMED（従来操作維持・意味付き転送・出典／参照保持・選択式提案）／PROVISIONAL（名称・UI・形式・対応アプリ・実装時期） | 2026-11-16 |
| DEC-057 | [学習・適応・継続によって成長するPersonal Agent OS](DEC-057-growing-personal-agent-os.md) | CONFIRMED（OSの中核コンセプト・モデル非依存の継続性・ユーザー主導）／PROVISIONAL（方式・効果測定・適用版） | 2026-11-16 |
| DEC-058 | [習慣学習を主軸に、Teach Modeを任意の補助機能とする](DEC-058-habit-first-learning-and-optional-teach-mode.md) | CONFIRMED（Habit-first・Teach補助・既存の保存／実行承認維持）／PROVISIONAL（Teach UI・記録技術・実装版） | 2026-11-16 |
| DEC-059 | [学習履歴を分かりやすく閲覧するNagi Insights（仮称）](DEC-059-learning-history-insights.md) | CONFIRMED（学習の内容・経緯・根拠・支援への反映を確認できる体験）／PROVISIONAL（UI・履歴Schema・評価指標・適用版） | 2026-11-16 |
| DEC-060 | [応答速度を優先するIMEとAIを活用した学習](DEC-060-responsive-ime-ai-learning.md) | CONFIRMED（入力応答優先・許可されたAI学習・秘密入力除外と忘却・Cloud許可・AI不可時の通常IME・モデル非依存の継続性を目指す方針）／PROVISIONAL（方式・数値・版） | 2026-11-16 |
| DEC-061 | [純正PDF Reader兼Editorと段階的な高度編集](DEC-061-first-party-pdf-reader-editor.md) | CONFIRMED（採用・段階的拡張・AI非必須・公開API連携）／PROVISIONAL（版・高度機能の順序・技術・合格値） | 2026-11-16 |
| DEC-062 | [純正ユーティリティ4分野の採用](DEC-062-first-party-utilities.md) | CONFIRMED（画像・スクリーンショット／文書スキャン・圧縮／解凍・動画／音楽の全分野採用、AI非必須の高速・軽量な基本機能優先）／PROVISIONAL（アプリ構成・対応範囲・連携詳細・版・受入） | 2026-11-16 |
| DEC-063 | [電話・連絡先・SMSの端末横断採用](DEC-063-phone-contacts-messaging.md) | CONFIRMED（純正採用・共通モデルと適応UI・能力／認可依存・People統合原則）／PROVISIONAL（アプリ構成・通信／端末間連携・学習・版） | 2026-11-16 |
| DEC-064 | [Nagi Share（仮称）：近距離の端末間ファイル共有](DEC-064-nagi-share-nearby-file-transfer.md) | CONFIRMED（純正OS機能・端末横断・ローカル直接転送・受信許可・暗号化／認証・LLM非必須）／PROVISIONAL（名称・技術・UI詳細・対応版／端末・性能） | 2026-11-16 |
| DEC-065 | [Nagi Handoff（仮称）：別端末での作業継続](DEC-065-nagi-handoff-work-continuation.md) | CONFIRMED（純正採用・意味的継続・安全境界）／PROVISIONAL（名称・UI・復元範囲・経路・クラウド・性能・適用版） | 2026-11-16 |
| DEC-066 | [同一LANから始める共通Device Connectivity／Trust／Transport](DEC-066-device-connectivity-lan-first.md) | CONFIRMED（初期LAN・共通抽象・将来遠隔拡張・安全境界）／PROVISIONAL（具体方式・運用・時期・性能） | 2026-11-16 |
| DEC-067 | [Nagi PowerとOS全体の決定論的な電力・熱・資源管理](DEC-067-nagi-power-deterministic-resource-management.md) | CONFIRMED（仮称で採用・AI非必須・安全／前景優先・低レイヤー基盤整備の方向）／PROVISIONAL（具体方式・端末・効果・実装版・Gate） | 2026-11-16 |
| DEC-068 | [Nagi Sleep（仮称）— スリープ・休止状態・復帰の管理](DEC-068-nagi-sleep-resume-recovery.md) | CONFIRMED（非LLM管理・状態保存と再検証・通常復帰と障害復旧の区別・段階的Hibernate）／PROVISIONAL（名称・方式・端末・時期・Gate） | 2026-11-16 |
| DEC-069 | [35項目のFoundation Checklist管理](DEC-069-foundation-checklist-management.md) | CONFIRMED（台帳管理）／PROVISIONAL（推奨優先度・検討順序） | 2026-11-16 |
| DEC-070 | [Memoryを活用した個人最適化Nagiアプリ制作](DEC-070-memory-informed-native-app-creation.md) | CONFIRMED（製品方向）／PROVISIONAL（Memory許可・UI・実装・時期） | 2026-11-15 |
| DEC-071 | [初心者向けNagi Native App制作体験](DEC-071-nagi-native-app-creation-experience.md) | CONFIRMED（純正の初心者向け・自然言語／ローカルLLM支援、小規模日常アプリ、試用／導入／共有方向）／PROVISIONAL（製品形態・技術・モデル・版・配布方式） | 2026-11-15 |
| DEC-072 | [Follow／Evolution — Memoryからの操作支援とアプリ改善ビジョン](DEC-072-follow-evolution.md) | CONFIRMED（概念区分・改善試作を伴う提案の長期方向）／PROVISIONAL（実装・受容性・版割当て） | 2026-11-15 |
| DEC-073 | [許可された利用傾向からの潜在的な不便の発見と改善提案](DEC-073-proactive-friction-discovery.md) | CONFIRMED（高水準の提案方針）／PROVISIONAL（データ・診断・Score・UI・自動適用・実装版） | 2026-11-15 |
| DEC-074 | [AI組み込み体験の優先順位と階層型AI組織](DEC-074-human-led-agent-organization.md) | CONFIRMED（当面の体験優先・組織は非優先の長期ビジョン）／PROVISIONAL（構造・技術・提供版・実装順序） | 2026-11-16 |
| DEC-075 | [`builder`が作成した新規アプリの試用・正式導入](DEC-075-builder-app-trial-and-installation.md) | CONFIRMED（方式C：安全な試用・機能／証拠／権限の説明・採用判断後の別途承認付き正式導入）／PROVISIONAL（実装方式・適用版） | 2026-11-05 |
| DEC-076 | [`builder`製アプリのNagi連携・Action API提案／生成](DEC-076-builder-generated-action-api-integration.md) | CONFIRMED（方式C：用途別Public Action候補の自動提案／承認済みScope内実装検証／通常GUI両立）／PROVISIONAL（生成方式・Schema・適用版） | 2026-11-05 |
| DEC-077 | [LinuxカーネルLTS標準採用とStable先行利用](DEC-077-linux-kernel-lts-stable-policy.md) | CONFIRMED（LTS標準・Stable先行利用許可）／PROVISIONAL（具体版・更新保守・patch／config管理・適用版） | 2026-11-16 |
| DEC-078 | [端末別Boot方式と共通Nagi起動処理](DEC-078-boot-platform-entry-and-common-nagi-startup.md) | CONFIRMED（PCのUEFI基本・機器別方式許容・Linux起動後の共通Nagi処理）／PROVISIONAL（bootloader・復旧／信頼・適用版） | 2026-11-16 |
| DEC-079 | [System Imageの読み取り専用化・A/B更新／復旧・User Data／AIモデル分離](DEC-079-system-image-readonly-ab-data-separation.md) | CONFIRMED（read-only System・User Data分離・A/B更新／復旧・モデル論理分離）／PROVISIONAL（partition・filesystem・更新／復旧実装・適用版） | 2026-11-16 |
| DEC-080 | [Init／Service Manager：A案第一候補・B案との短期実装比較](DEC-080-init-service-manager-ab-evaluation.md) | CONFIRMED（A第一候補・短期A/B実装比較後に最終選択）／PROVISIONAL（A/B正式採用・試験詳細・期間・適用版） | 2026-11-16 |
| DEC-081 | [Nagi主体の基本ユーザーランドと汎用OSSの選択的活用](DEC-081-nagi-owned-userland-selective-oss-components.md) | CONFIRMED（Nagi固有サービス／API／アプリ環境の独自設計、libc・基礎ライブラリ・基礎ツールのOSS活用、既存distributionユーザーランド一式非流用）／PROVISIONAL（具体部品・ABI・適用版） | 2026-11-16 |
| DEC-094 | [Resource Manager：Linux標準＋Nagi独自管理、スケジューリングの並行研究](DEC-094-resource-manager-linux-baseline-custom-scheduling-research.md) | CONFIRMED（#25 B＋C研究案：Linux標準を製品基本、Nagi Resource Managerの統合配分と独自schedulerの比較研究・標準方式への退避）／PROVISIONAL（cgroup／sched_ext等、アルゴリズム・性能・実機・採用版） | 2026-11-16 |
| DEC-095 | [App Lifecycle：Linux基盤＋Nagi App Supervisorと独自Runtimeの並行研究](DEC-095-app-lifecycle-linux-supervisor-runtime-research.md) | CONFIRMED（#26 B基本：Linux Native Process＋Nagi Supervisor、独自Runtimeの比較研究・標準経路保持）／PROVISIONAL（WASM等の採否・性能・対応機器・実装／版） | 2026-11-16 |
| DEC-096 | [Settings / Notification：Cの状況適応を最終目標に、Bの統合基盤を段階導入](DEC-096-settings-notifications-context-adaptive-staged.md) | CONFIRMED（#27 Cが最終目標、Bの統合Settings／Notificationを段階導入。明示設定・重大警告・Privacy／Permission維持）／PROVISIONAL（状況判定・習慣連携・自動調整詳細・版／受入） | 2026-11-16 |
| DEC-097 | [通常a11y基盤の先行整備とAI-native操作の長期方向](DEC-097-accessibility-foundation-and-ai-native-long-term-direction.md) | CONFIRMED（#28 通常a11y先行、長期方向との両立、GUI・AI不在時操作・認可維持）／PROVISIONAL（具体入力技術・時期・time／locale詳細・受入） | 2026-11-16 |
| DEC-098 | [Diagnostics / Logging：Nagi統合診断B＋段階的AI支援診断C](DEC-098-diagnostics-linux-nagi-ai-assisted-staged.md) | CONFIRMED（#29 Bを製品基本、CのAI原因分析・修復提案を将来の正式目標として段階導入）／PROVISIONAL（Linux backend・AIモデル・個別解析・保持／export・版／受入・自動修復） | 2026-11-16 |
| DEC-099 | [Installer / First Setup：Linux基盤＋Nagi専用GUI（方式B）](DEC-099-installer-first-setup-linux-nagi-gui.md) | CONFIRMED（#30 Linuxの既存disk等を活用したNagi GUI Installer、AI非依存の安全なOffline-first導入、破壊前確認）／PROVISIONAL（Linux部品・GUI・partition／暗号化・端末・実機受入・適用版） | 2026-11-16 |
| DEC-100 | [System Update：Linux更新基盤＋Nagi統合Update Service（B）](DEC-100-system-update-linux-nagi-coordinated-ab.md) | CONFIRMED（#31 B：Linuxの更新機構とNagiの署名・互換検証／A/B切替・失敗時復旧・System／App／Model別管理）／PROVISIONAL（Linux採用部品・配信／鍵・更新単位・UI／自動適用・実機／版） | 2026-11-16 |


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
