# DEC-053 — AI Task開始前のWayback復元可能性表示

**状態:** CONFIRMED（AI Task開始前に、Waybackで復元できる範囲をユーザーへ示す高水準の方向）／PROVISIONAL（表示分類・UI・判定根拠・実装方式・適用版）
**最終合意日:** 2026-11-15
**対象:** NagiのAI Taskの事前体験。特定Version、Task種別ごとの詳細な適用条件は未確定
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-071（初心者向けNagi Native App制作）](DEC-071-nagi-native-app-creation-experience.md)

## 目的

AIに作業を任せる前に、失敗した場合にWaybackでどこまで戻れるのかをユーザーが把握できるようにする。AIの成功申告や一般的な安全表示で済ませず、Taskに関連する変更と実際に利用可能な保護・復旧手段を説明する。

## 確定した方向

1. **AI Task開始前に復元可能性を表示する。** Taskを開始する前に、そのTaskでWaybackを使ってどこまで復元できるかをユーザーが見られるようにする。
2. **表示は実際の復旧可能性に基づく。** Nagiは、保護・復元できると確認できた範囲と、部分的・不明・復元不能な範囲を誤認させない。表示は保証やOS操作の許可ではなく、復元が不確実な場合は不確実性を示す。
3. **既存のTask承認と権限境界を維持する。** 事前表示、Waybackの有無、復元できる範囲の広さは、Task開始、変更、外部送信、Permission付与の承認を置き換えない。実行はDEC-001、Taskと変更の検証・保護はDEC-026、既存のAction別承認と現行Policyに従う。
4. **復元不能な外部作用を正直に扱う。** 外部送信や遠隔サービスへの変更など、実際に取り消せない操作を復元済み・安全に戻せると表示しない。複数Actionの一部だけが戻せる場合、Task全体を一括して戻せると誤認させない。

## 未確定・暫定事項

- 「復元可能」「一部のみ」「不明」等の表示分類・名称、Task要約と操作単位のどちらで見せるか、UI配置・詳細度は未確定。三分類は検討候補であり、本決定で確定しない。
- 実行前にTaskの変更対象・副作用とProtectionの成立状態をどう把握するか、Provider別のRecovery契約、Checkpointの作成要否・作成時点・保持範囲は未確定。Workspaceの配置・アプリ状態のCheckpointを、ファイル本文の保護とみなさない（DEC-014／026）。
- 事前表示後にResource Revision、Permission、Task Scope、Provider可用性が変わった場合の再評価と状態遷移は既存契約を維持しつつ、具体的なUXとAcceptanceを別途決める。
- 対象Version、適用可能なTask／Action、性能・保持コスト、実装Owner、Acceptance条件は未確定。

## 例外・禁止・安全境界

- Waybackが利用可能でない、保護の成立を確認できない、または復旧経路が不明な項目を「復元可能」と表示しない。未対応の状態は不明・未保護・復元不能等、実態に合う説明へ分ける。表示文言の分類はPROVISIONAL。
- 事前表示を見たことをTask承認と見なさず、表示を理由に承認・Permission・Policy・Task Scope・Resource Revisionの確認を省略しない。
- Wayback／Checkpointのプレビューは、その後の実行で同じ結果・全量復旧・Atomicityが保証されることを意味しない。実行後に確認できた復元状態だけを復元済みと報告する。
- 読み取り・復元・Activity記録と履歴表示は現行のUser／Profile／Resource Permission、Privacy、Retentionに従う。権限外の資料・変更内容をプレビューで漏らさない。
- 本決定はDEC-014のWorkspace履歴Preview、DEC-026のリスク別保護と検証、DEC-018のBuilder原本保護要件を弱めず、実装開始、Release assignment、0.3 Acceptance、0.4 Entry Gate／Integration Owner checkpointを変更しない。

## 既存決定との関係

- **DEC-014:** Workspace、Checkpoint、アプリ状態、画面配置等の復元Previewを定める。本DECはその情報をファイル本文の保護や任意のAI Taskの復旧保証に読み替えない。
- **DEC-026:** AI変更の検証可能性、保護、Risk別の復旧可能性と不確実性の報告を定める。本DECは実行開始前にその実際の復旧可能性を説明するユーザー向け方向を追加する。
- **DEC-018:** builderによるソース変更では隔離編集、Revision／競合確認、反映前の元ファイル保護等の既存条件を維持する。事前表示で保護・反映条件を緩和しない。
- **DEC-071:** 初心者向けアプリ制作体験は、この事前表示を利用する候補である。コード／Package生成、Build／Test、試用・導入の各段階で何を示すかは本DECの表示目的と既存の各App lifecycleに従い、実装詳細は未確定。

## Roadmapとの関連

AIへ変更Taskを委任する場合の将来UX方針を記録する。0.3 Personal Agentや0.4+ builder Role workflowへの具体的なRelease／Acceptance割当ては行わない。既存の0.2／0.3／0.4 Gateと実装仕様は変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-15 | ユーザーがAI Task開始前にWaybackでどこまで復元できるかを表示する方向を確認。3状態の表示案、UI、判定基準、TaskごとのProtection実装は暫定として分離。 |
2026-11-16 統合注記: PR #108のDEC-011／026とPR #110の本DECは同じ事前表示方針を記録する。基本原則はDEC-026、事前表示の詳細な境界は本DECを参照し、分類・UIを確定へ昇格させない。
