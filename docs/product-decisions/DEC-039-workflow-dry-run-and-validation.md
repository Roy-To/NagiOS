# DEC-039 — WorkflowのDry Run・事前シミュレーション・段階的検証

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（思想㉕A：本番前の副作用を起こさないDry Run／シミュレーションと段階的検証、未検証の明示、本番Actionの独立した認可）／PROVISIONAL（隔離環境・モック、判定基準、UI、実装・適用版）  
**最終合意日:** 2026-11-05  
**対象:** Nagi Workflowの実行前検査、Dry Run、シミュレーション、段階的な検証。実装版は未確定  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-002](DEC-002-cloud-ai-data-sharing.md)、[DEC-003](DEC-003-memory-classification.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-026](DEC-026-ai-change-verification-and-recovery.md)、[DEC-027](DEC-027-public-action-and-scoped-gui-fallback.md)、[DEC-028](DEC-028-deterministic-workflows-from-habits.md)、[DEC-029](DEC-029-scheduled-event-triggered-workflows.md)、[DEC-030](DEC-030-workflow-template-portability-and-sharing.md)、[DEC-031](DEC-031-external-workflow-validation-and-version-updates.md)、[DEC-032](DEC-032-workflow-safe-retry-and-recovery.md)、[DEC-033](DEC-033-natural-language-workflow-authoring.md)、[DEC-034](DEC-034-human-in-the-loop-workflow-steps.md)、[DEC-037](DEC-037-human-step-revision-deadlines-reminders.md)、[DEC-038](DEC-038-ai-judgment-step-and-model-portability.md)、[Private 0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[Public 0.2基盤仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)

## 目的とユーザー体験

自然言語で生成したWorkflow、ユーザーがGUIで作成したWorkflow、外部から取り込んだWorkflow、改訂済みまたはモデル切替後のWorkflowについて、本番でデータ変更・送信・不可逆操作を実行する前に、実行予定の手順・権限・分岐・依存関係・AI判断の条件を可能な限り試運転できるようにする。シミュレーションは実際の成功を保証するものではない。

## 確定した仕様（思想㉕・選択A）

1. **Dry Runを利用可能にする:** Workflow本番実行前の検査、Dry Run／シミュレーション、必要に応じた段階的な検証を利用できる方針とする。ユーザーは、実際の変更・送信を伴わずに、どのStepや分岐・Actionが想定されるか、何が検査済みか、不確実な箇所はどこかを確認できるようにする。
2. **Dry Runは副作用を発生させない:** Dry Run中には本番のResource変更・削除、外部送信、メール配信・公開、実端末設定変更、不可逆操作、未許可のScheduled／Event実行登録等の副作用を行わない。副作用なしの保証が難しいActionは実行せず、静的検査や明示的なモック・隔離した検査に切り替え、実施できなかった部分を未検証として示す。未知のActionを「無害」と推測して動かさない。
3. **静的検査と試運転を区別:** 手順・型付きAction／Provider、必要なPermission／Capability、Task Scope、対象Resource／Revision、起動条件、分岐、外部通信先、Human Step、入力・出力Schema、AI判断Stepのモデル能力などを可能な範囲で検査する。静的な契約検査で確認したことと、実際のProvider・Target Runtimeで検証済みであることは別々に表示する。
4. **模擬環境の限界を表示:** モック、テストデータ、隔離Provider、シミュレーション結果は本番Resourceの現在状態や外部サービス側の挙動を完全には再現できない。予測された分岐やAI回答、想定送信先、取消・復旧可能性を実際に成立した結果として表示しない。検証可能・未検証・不適合・失敗・実行不能の範囲と根拠を区別する。
5. **AI判断Stepの検査:** DEC-038のモデル非依存の判断能力と型付き入出力契約、分岐の許容範囲、モデル切替後の互換性・品質・安全性を可能な範囲で検証する。検証用のAI推論も、使用するInput・Providerに応じて現行PermissionとCloud送信許可を要する。実際の外部送信や課金等の副作用がある検査は、無害なDry Runの一部と偽らず、別途明示された検証条件・許可を要する。
6. **段階的な本番検証も別承認:** Dry Runで実行しない処理の実環境テストが必要な場合は、対象、実際の副作用、使用データ、実行環境、送信先、必要な権限、保護・復旧可能性を事前に示し、正規のAction／Provider承認を独立に取得する。外部送信や不可逆操作の既存承認を省略しない。検査用の隔離データと本番ユーザーデータを混同しない。
7. **本番開始への自動昇格は禁止:** Dry Run合格、Workflow保存、外部テンプレートImport、モデル切替、テスト用権限の存在だけでは、本番のTask実行・自動起動・外部送信・重要操作の許可にならない。本番実行時には現在のIdentity、Permission／Capability／Policy、承認済みTask Scope、Resource／Revision、Provider／Model状態、停止・取消状態、Action別承認を改めて評価する。
8. **結果の記録:** Workflow／候補Version、検査対象・条件、実施済みの検査、実施していない処理、成功・失敗・未検証、利用したModel・Provider、必要な再確認を、既存Privacy・Retentionに従って記録／表示する。根拠のない「安全」「完全復旧可能」「本番成功」を主張せず、秘密情報や個人Memoryを無用に複製しない。

## 既存決定・実装Gateとの整合

- **DEC-031:** 外部Workflowの未信頼検査・可能な隔離事前検証を補完するが、外部入力の信頼・許可の独立性は維持する。本DECはユーザー作成・改訂・モデル切替後のWorkflowにも事前検証の考え方を広げる。
- **DEC-028／029／033／038:** Draft、正式なWorkflow手順、モデル・判断契約、Scheduled／Event起動の各承認と検査を区別する。検査通過による正式改訂や無人起動の自動許可はしない。
- **DEC-001／011／026／032／034／037:** 実行時のPermission、Human Step、有限Retry／Cancel、変更前保護、結果不明・部分成功・不可逆操作の制約を維持する。検査用の操作を通常Task Scope拡大の口実にしない。
- **実装境界:** 0.1 M30、0.2のAction／Job／Permission／Activity等、0.3以降のImplementation／Activation、AGENTS.md、各WorkstreamとAcceptanceは変更しない。本決定は実装開始・完成の証拠ではない。未公開仕様をPublic repositoryへ転載しない。

## 未確定事項（PROVISIONAL）

- Workflow Schemaと検証器、Mock／Sandbox／Stagingの構成、副作用の技術的遮断とFail-closed検出、Test DataやSnapshotの生成・削除。
- 静的検査・Mock・隔離・Target Runtimeの各段階の受入条件、Coverage、許容不可のエラーや安全性判定、AI判断品質の評価指標。
- Dry RunのUI、各Stepの検証済み／未検証表示、実環境Testの承認フロー、証拠保持・監査Schema／Retention、対応Provider・Release割当。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | 思想㉕の選択Aを確定。Workflow本番実行前にDry Run・シミュレーション・段階的検証を可能とする。副作用を防ぎ、検証不能部分は未検証とし、実際の外部送信・不可逆操作などは独立した既存承認を要する。 |
