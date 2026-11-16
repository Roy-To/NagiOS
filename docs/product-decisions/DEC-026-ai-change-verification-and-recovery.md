# DEC-026 — AIによる変更の検証可能性と復元可能性

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（方式A：検証・復元可能な変更を優先、リスク別の保護、承認範囲内の自律実行、AI Task開始前のWayback復元可能性の可視化）／PROVISIONAL（リスク閾値・表示／UI・事前判定範囲・未対応／不明時の扱い・数値・実装方式・適用版）  
**最終合意日:** 2026-11-15  
**対象:** Nagi／Agentが扱うデータ変更・アプリ横断Actionの製品安全原則（実装時期は未確定）  
**関連:** [DEC-001](DEC-001-agent-autonomy-actions.md)、[DEC-011](DEC-011-task-approval-progress-ui.md)、[DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-016](DEC-016-builder-autonomy-risk-based-completion.md)、[DEC-018](DEC-018-builder-isolated-edit-conditional-apply.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-023](DEC-023-work-data-centric-os.md)、[DEC-025](DEC-025-cross-app-data-reference-and-updates.md)、[DEC-036](DEC-036-builder-app-trial-and-installation.md)、[Public 0.2 Platform仕様](https://github.com/Roy-To/NagiOS/blob/main/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)、[0.3改訂ロードマップ](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)

## 目的

AIが複数の資料やアプリのデータを変更するとき、実行可能であることだけでなく、どの対象をどう変更したかを検証し、可能な範囲で元の状態に戻れるようにする。すべての操作に一律の重い保護処理や都度承認を強制せず、リスクと可逆性に応じた安全性と使いやすさを両立する。

## 確定した方針（思想⑨・選択A）

1. **検証可能な変更を優先:** AIの提案・成功申告だけを成功の証拠にしない。利用可能な正規Public Action／Provider契約に基づき、変更前の対象、現在のPermission／Policy、Resource／Object／Revision、承認済みTask Scopeを確認し、実行後には適用結果や実際に確認できる状態を検証する。
2. **復元できる変更を優先:** 変更内容・差分、実行前の状態、Checkpoint／Revision／Wayback等の利用可能な保護・復旧経路を把握し、操作の性質とリスクに応じて適用前の原本保護を行う。Workspaceの画面配置の保存をファイル本文の保護と同一視しない。
3. **リスクに応じた検証・保護:** 低リスクで可逆な操作には軽量な検証・保護を利用し、影響が大きい、不可逆、不確実、他アプリに波及する変更ではより厳格な検証・保護、必要な承認へ切り替える。具体的なRisk taxonomyや閾値は別途決める。
4. **承認済みTask内の自律実行:** ユーザーが目的、対象、変更可能なAction、範囲を事前承認し、現行のOS Permission／Capability／Policyが許す低リスクの下位操作については、人間に都度確認せず進められる。既存の重要操作に要求される画面承認や外部送信の許可を免除するものではない。
5. **競合・不明・失敗の扱い:** 元のRevisionが変わった、意図した変更の検証ができない、保護に失敗した、部分的にしか適用できなかった場合は、必要な再確認や保留・停止・回復へ移す。古い状態でユーザーの新しい編集を無断上書きしない。失敗や未検証を成功として表示しない。
6. **結果と根拠の記録:** Activity／Task状態には、実行したAction、対象、確認可能な差分や結果、検証状態、保護・復旧の可否、部分成功・失敗を正直に記録・表示する。記録自体も既存の情報分類・Permission・Retentionに従い、秘密情報の無用な複製を避ける。
7. **不可逆操作の正直な扱い:** 外部送信、送信済みメール、遠隔環境の副作用等は元に戻せないことがある。Undoや完全復旧を保証せず、既存の追加承認・Scope・Policyに従う。複数アプリにまたがるTaskの部分失敗を、存在しない全体Atomicityで隠さない。
8. **AI Task開始前の復元可能性表示:** Taskを開始する前に、計画された変更についてWaybackでどこまで復元できるかをユーザーに示す。目的はAIの判断精度や完全復旧を保証することではなく、復元可能・部分復元・復元不能・事前判定できない可能性を隠さず伝え、安心して作業を委ねる判断材料を提供することにある。表示は既存のPermission／Policy・承認条件・Task Scopeを変更せず、Task内のすべての操作がUndoできることも意味しない。事前評価は復元成功の保証ではなく、実行後は実際のAction・Provider結果・Revisionや副作用を確認する。

## 既存仕様の優先と禁止

- DEC-018のBuilderによるコード修正は、隔離編集、反映前の元ファイル内容の保護、Revision／競合検査、検証成功など、**既に確定した厳しい反映条件を維持**する。本DECの一般的な「リスク別保護」を根拠に緩和しない。プログラミングRoleは`builder`で、初期はNagi自身がRole切替で実行する方式Cが確定済み（DEC-015）。将来の別Agent配置と実装詳細は未確定。
- 検証できたこと、保護済みであること、AIが実行したことを理由にPermissionを拡張したり、未承認のTaskを開始／再実行したりしない。
- 本決定は0.1 M30、0.2 M17／M18／M21、0.3 Implementation Spec／Activation、Workstream owner／Acceptanceの既存ゲートを変更せず、実装完了・実装開始を意味しない。未公開の仕様はPublicリポジトリに転載しない。

### `builder`製アプリの初回試用・導入と正式更新（DEC-035／036）

未導入の新規アプリの試用・採用・初回正式登録は[DEC-036](DEC-036-builder-app-trial-and-installation.md)、正式導入後の保守・更新は[DEC-035](DEC-035-builder-created-app-maintenance-updates.md)に従う。一般的なリスク別の自律実行・検証を根拠に、いずれかのユーザー承認、正規Package検証、現行Permission／Policy確認を省略しない。

### `builder`製アプリの正式更新・復旧（DEC-035）

正式導入後のアプリ修正は[DEC-035](DEC-035-builder-created-app-maintenance-updates.md)の方式Cに従い、Nagiが提案し、必要な権限・Scopeで隔離修正と検証を実施し、**ユーザーが更新内容を確認・承認した場合に限って**正式Versionを更新する。アプリのデータ・設定は可能な範囲で保護し、互換性・Migration・現行Permission／Revisionを評価する。旧アプリBinaryの復元とデータ変更・外部副作用の完全Undoを混同せず、復旧可能性が不明ならその旨を示して保留する。DEC-018の条件付きコード反映は正式アプリ更新の包括承認ではない。本DECでRollbackの全成功やTarget Runtime受入を保証しない。

## 未確定事項（PROVISIONAL）

- 操作・データ種別別のRisk taxonomy、可逆性判定、確認UI、保護・検証を強化する閾値と例外。
- Provider別の変更前スナップショット、差分・Revision検証、Transaction／Checkpoint／Wayback連携、Rollback／Compensationや重複実行防止の詳細。
- 検証証拠の形式と合格基準、部分成功・失敗時の回復・報告、保護データの容量・保持期間、性能コスト。
- Task開始前に復元可能性を評価できるAction／Resource／Provider／副作用の範囲、その根拠と評価の鮮度。
- 完全復元・部分復元・復元不能・判定不能を分ける具体的な条件、表示名・UI・詳細度、および未対応／不明時の説明と操作導線。未対応／不明を復元可能と扱わない原則はCONFIRMEDだが、その具体的な扱いは未確定。
- 比率・件数・スコア等を表示するか、表示する場合の算定方法と閾値。数値は現時点で確定しない。
- 適用バージョン、実装担当、Target Runtime Acceptance。

## Roadmapとの関連

- **0.2:** 型付きPublic Action、Resource／Revision、Permission、Activity／Transaction／Checkpoint／Waybackの既存基盤を利用する。
- **0.3:** Personal Agentが許可されたTaskの範囲で複数アプリを操作する際の共通製品原則とする。既存Gateを維持する。
- **0.4+:** 追加の作業担当にも同じ安全原則を適用する。Builder専用のDEC-016／018を変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-05 | 思想⑨の選択Aを確定。AIによる変更は検証可能・復元可能な方式を優先し、リスクに応じて保護と検証を調整する。承認済みの低リスク操作は都度確認なく実行でき、既存の権限・不可逆操作・Builderの厳格な反映条件を維持する。 |
| 2026-11-15 | AI Task開始前にWaybackで復元可能な範囲を示し、復元可能・部分復元・復元不能・事前判定できない可能性を隠さず、作業委任の判断材料とする方針を確認。UI、事前判定範囲、未対応／不明時の具体的扱い、数値、適用版は未確定。すべての操作のUndo保証や既存の承認・Permission変更は含まない。 |
