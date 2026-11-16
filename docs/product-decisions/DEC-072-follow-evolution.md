# DEC-072 — Follow / Evolution：Memoryによる操作支援からアプリ改善へ

**状態:** CONFIRMED（FollowとEvolutionの役割分担、潜在的な不便の提案、許可された安全な範囲での改善版プレビューを長期目標とする方向）／PROVISIONAL（実装技術、UI、収集・評価方式、提供時期、製品受容）
**決定日:** 2026-11-15
**適用範囲:** NagiのMemory、日常操作支援、将来のNagi Native Apps制作・改善体験。特定バージョンの必須Gateではない。
**関連:** [DEC-003](DEC-003-memory-classification.md)、[DEC-004](DEC-004-memory-learning-forgetting.md)、[DEC-008](DEC-008-proactive-behavior.md)、[DEC-010](DEC-010-intent-first-adaptive-ui.md)、[DEC-015](DEC-015-builder-role-review.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-017](DEC-017-third-party-developer-ecosystem.md)、[DEC-028](DEC-028-deterministic-workflows-from-habits.md)、[DEC-035](DEC-035-builder-created-app-maintenance-updates.md)、[DEC-036](DEC-036-builder-app-sharing-and-publication.md)、[DEC-040](DEC-040-builder-app-development-project-retention.md)、[VISION-001](VISION-001-human-adaptive-os.md)、[0.3 Revised](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[0.5–1.0 Master Roadmap](roadmaps/Nagi_OS_0.5-1.0_Master_Roadmap.md)

## 目的

人が自覚して言語化した要求だけに応える開発支援ではなく、許可されたMemoryと利用文脈から、本人が不便だと意識していない反復操作やUI・手順の改善余地を見つけ、押しつけず改善の選択肢を提案する。人がソフトウェアに合わせるのでなく、ソフトウェアが人に合わせるというVISION-001を、操作補助からアプリ改善へ段階的に発展させる。

## 合意した方向（CONFIRMED）

1. **Follow — 既存アプリの操作を補助する層。** Memory、好み、作業文脈を利用できる範囲で、現在のアプリケーションの操作・入力・手順の負担を軽減する。アプリ本体の変更はFollowの本質的な要件にしない。ユーザーの明示指示、Permission、既存UI操作を優先する。
2. **Evolution — アプリそのものを改善する層。** 許可された利用傾向から、ボタン配置、画面構成、処理の順序、必要な小規模機能などの改善候補を発見し、アプリの改修案をユーザーへ提示する。利用者本人が問題を明確に言葉にすることを提案の前提としない。単なるWorkflowの自動化・既存操作の補助だけに限定しない。
3. **提案前の安全な試作を長期目標とする。** 可能であり、現在のPermission／Policy、対象Resource、承認済みScopeと隔離条件が許す場合は、改善されたUIや動作のプレビュー・試作品を用意してから「試してみますか」と提案する体験を目指す。作成できない場合は仮説や設計案として示し、実際に検証済みのプレビューと混同しない。
4. **人間の好みと選択を尊重する。** 操作回数が少ないことだけを「良いUI」の基準にせず、慣れ、安心感、アクセシビリティ、確認を敢えて残す意図などを考慮する。提案の採用・却下と明示的な設定を尊重し、既存アプリや利用中のUIを推定だけで大きく変更しない。
5. **改善提案・試作・正式適用は別。** 提案やプレビューの提示はアプリ本体の変更・自動アップデート・Permission付与・外部送信を認可しない。正式な改修・導入はDEC-015／016／018／035／036等の隔離編集・検証・ユーザー承認・正規Package経路を維持する。実際に可能な復旧範囲はWayback／Revision等の証拠に基づき、完全Undoを保証しない。
6. **Memoryと利用情報は許可された範囲のみ。** DEC-003／004の機密分類、User／Profile分離、忘却、明示指示優先を守る。潜在課題発見を理由として無断の常時画面監視、全アプリ操作の包括記録、第三者へのMemory共有、権限拡大を認めない。共有アプリPackageに作成者個人のMemoryやCredentialsを含めない（DEC-036）。
7. **段階的な長期ビジョン。** Memoryによる理解を土台とし、Followで操作支援を磨き、その発展型としてEvolutionを検討する。Evolutionの利用者受容性・UX効果は未検証であり、実装完了時期や1.0での必須提供を確約しない。

「Follow」「Evolution」は本検討での概念名。正式な機能名・アプリ名の確定を意味しない。

## 未確定事項（PROVISIONAL）

- 許可されたMemory／イベント／操作傾向から潜在的な非効率をどう検出し、誤推定・偶然の操作と区別するか。計測対象・保持期間・Opt-in／Opt-out・評価手法。
- 改善提案の根拠、本人にとっての利益、負担、好みの優先度、採用・却下の扱い、頻度と通知UI。提案の受容性と実効性の検証。
- 改善プレビューをどの段階で、どの安全な隔離環境・Scopeで生成するか。構造化UI提案、動作する試作品、アプリ本体のPatchの区別。
- Nagi Studio（仮称）／builder／Porting Toolkitとの接続方式、個別UIコンポーネント、LLM・非LLMの役割分担、対応アプリと端末。
- 技術構成、推論能力・性能・資源コスト、受入条件、対応バージョン・着手時期、一般ユーザーへの実際の受容・普及。

## ロードマップとの境界

0.3のModel-independent Memory／Personal Agentの方針を前提とするが、Evolutionを0.3の必須機能に追加しない。0.4の初期`builder` Roleや、0.5–1.0の暫定版割当て・既存Gateを変更しない。Porting Toolkitは既存アプリの移植支援であり、Evolutionの完成と同一視しない。初期に動作するアプリ制作環境が存在する、完成したAI監視／変更システムがある、とも主張しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-15 | Follow＝Memoryを踏まえた既存アプリの操作支援、Evolution＝潜在的な不便を発見してアプリの改善案を試作・提示する長期発展型、と整理。安全で許可された範囲では改善版プレビューを用意して提案する方向に合意。実装時期・受容性・詳細UI・データ分析方式は未確定。 |

| 2026-11-16 | PR #111の旧DEC-052をDEC-072へ改番。実機候補DEC-052との番号競合のみを解消し、原決定・状態・履歴を保持。 |

関連する[DEC-073](DEC-073-proactive-friction-discovery.md)は潜在的不便の発見と根拠付き提案を扱う。発見・提案だけで試作の権限は生じず、本DECのプレビュー目標も既存の承認済みScope・Permission・隔離条件の内側で扱う。
