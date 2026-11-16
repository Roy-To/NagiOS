# DEC-068 — Nagi Sleep（仮称）— スリープ・休止状態・復帰の管理

**状態:** CONFIRMED（採用・製品方針）／PROVISIONAL（正式名称・具体方式・対応端末・実装版・Gate）

**最終合意日:** 2026-11-16

**適用候補:** Linuxカーネル + Nagi専用ユーザーランド。実装時期・Release割当ては未確定

**関連:** [DEC-014](DEC-014-workspace-state-checkpoints.md)、[DEC-022](DEC-022-builder-interrupted-task-recovery.md)、[DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-009](DEC-009-nagi-first-home-desktop.md)、[DEC-042](DEC-042-linux-kernel-transition.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[純正ソフトウェア実装仕様](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[0.2実装仕様](../Nagi_OS_0.2_Codex_Implementation_Spec.md)、[roadmaps](roadmaps/README.md)

## 目的と決定の根拠

作業状態と安全境界を保ちながら端末をスリープさせ、復帰後に利用可能な状態へ戻す「Nagi Sleep（仮称）」を採用する。ユーザーが直前の提案に「OK,それでいこう」と明確に同意し、本記録依頼で指定した範囲を確定事項の根拠とする。取得できない提案本文を補完して機能を増やさない。

Workspace保存とTask復旧の正本はDEC-014／022のままとし、本決定はスリープへの移行と復帰の管理を補足する。並行記録中の[Nagi Power（DEC-067、PR #128）](https://github.com/Roy-To/NagiOS/pull/128)はOS全体の継続的な電力・熱・資源管理を扱う。確認した[Power仕様](https://github.com/Roy-To/NagiOS/blob/5ce421f539d28468bdda04e543850bb647cb7857/docs/product-decisions/DEC-067-nagi-power-deterministic-resource-management.md)との責務を分け、別のAI Schedulerや保存正本を重複採用しない。並行記録の参照はmainへのマージ済みを意味しない。

## 確定した仕様（CONFIRMED）

1. **LLMを使わず管理する。** Linuxカーネルの標準的な電源管理機構と、Nagiの決定論的なシステムサービスでスリープ／復帰を管理する。AIが無効・不在・利用不能でも基本操作を利用できる。Linuxカーネルの採用を理由に既存ディストリビューションの電源管理構成を一括採用しない（DEC-042／043）。決定論的とは推論に依存しない制御・認可を指し、実行時間が常に同一になる保証ではない。
2. **まずスリープ・復帰・画面ロックを整備する。** 画面ロックは既存のIdentity／Sessionと認証・認可に従う。端末の復帰そのものを本人確認や権限付与の代わりにせず、LLMの判断でロックを解除しない。ロックと保存の具体的な処理順序・認証UIは未確定とする。
3. **移行前に必要な状態を保存する。** Workspace、対応アプリ、Taskの状態を、それぞれの正規API・保存Policy・Permission／Privacyに従って、移行前に必要な範囲で保存する。DEC-014の軽量な復元用状態と、アプリ独自の未保存編集の回復情報、ファイル本文のバックアップを区別する。全アプリ・全プロセスの完全保存や、未保存編集が必ず回復することを約束しない。
4. **復帰時にネットワークとデバイスを再検証する。** 接続・利用可能性・現行の権限や信頼状態を確認し、古い接続状態やハンドルをそのまま有効と見なさない。未接続・利用不能・検証未完了を正確に扱い、再検証だけを理由に未承認の通信・Actionを実行しない。確認項目・順序・再試行条件は未確定。
5. **Model Manager／AI Jobを安全に待機・資源調整する。** 通常のModel Runtime、Jobs、OS資源管理契約を通じて移行と復帰に協調させ、復帰直後もDEC-020の人間の操作応答性優先と現行の資源制限を守る。AIに電源管理・Schedulerの特権を直接与えない。モデルの常駐維持、unload／reloadや推論途中の保存方式を本決定で固定しない。
6. **通常復帰と障害後の復旧を区別する。** RAMが保持された通常のスリープ復帰は、保存情報からWorkspace等を再構成する電源断・クラッシュ・再起動後の復旧と別の状態として扱う。通常復帰を「全Task再実行」、障害後の復旧を「何事もなく処理が継続した状態」と表示しない。
7. **障害後に中断Taskや副作用Actionを無断再開・再実行しない。** 保存済み状態を用いた復旧はDEC-014／022に従い、ユーザーが選択する前に中断AI Taskを進めない。再開・やり直しを選んだ後も現行のUser／Profile、Permission／Policy、TaskのScope・期限・Budget・Cancel状態、Resource Revisionと競合、実行済み／未実行／結果不明を再検証する。結果不明のActionを盲目的に再送せず、副作用の二重実行を防ぐ。
8. **休止状態は段階的に追加する。** Hibernationは対応実機での技術検証を経て追加する方針とし、全端末での対応を約束しない。採用する実装技術や対応機種、保存・復帰の成立は未検証であり、スリープの採用だけで休止状態対応済みとはしない。

## 継続・保存・復元の境界

- **RAM保持でもTaskの権限は自動継承しない。** 通常復帰からの継続は、DEC-020で認める有効な承認済みTaskのResource待機と、ユーザーの明示Pause／Cancel／Stop、承認待ち・失効・状態不明・障害復旧待ちを区別する。前者の条件付き継続は既存の再検証規則に従う。すべてのAI Taskがスリープをまたいで自動継続できるという決定ではなく、プロセス・推論・Taskの具体的な継続境界はPROVISIONAL。
- **保存範囲と結果を正直に示す。** 保存できた状態、保存失敗・未対応・不明な状態、復帰後に利用できない資料やデバイス、部分復元・復元不能を区別する。Workspaceの復元だけで本文の巻戻しやAI Taskの実行を行わない。チェックポイントを用いる場合の復元先選択・既存状態の保護もDEC-014を維持する。
- **休止状態と障害復旧を混同しない。** Hibernationの継続可否は後続の設計・実機検証で決める。ストレージを使用することだけで障害後の保存状態復旧と同一視せず、復帰失敗・状態不明の場合も無断のTask／Action再実行を認めない。
- **低レイヤー基盤整備への合意を拡大しない。** 積極的に基盤を整える方向への同意は、全サブシステムの採用・実装着手・担当割当てを一括で認めるものではない。

## 既存仕様との役割分担（REVIEW NOTE）

| 正本 | 維持する責務と本決定の関係 |
|---|---|
| DEC-014／022 | Workspace保存・チェックポイント、権限・Revision再評価、選択式復旧、二重実行防止の正本。本決定は電源状態遷移からこれらを利用する方針を補足する |
| DEC-020／並行中のDEC-067 | 前景応答性、資源待機と明示停止の区別、OS全体の電力・熱・予算管理を維持する。Sleepは移行・復帰の協調を扱う |
| DEC-009／純正アプリ仕様 §54 Home・§65 Devices・§67 Models | Homeの作業復元、Devicesの信頼・接続、ModelsのProvider管理を置換しない。画面復帰・Workspace復元とTask実行を分離する |
| 0.1仕様 §43–45／0.2仕様 §9・13・14・48 | Model Managerの予算・load/unload、Session、アプリLifecycle、JobsのScope・取消・有限永続化、Resource-pressure契約を継承する。背景アプリのsuspendと端末全体のスリープを同一視しない |
| 0.2仕様 §5.2／Linux移行補足／roadmaps | 既存のsuspend/resume非必須という0.2受入範囲を変更しない。将来の製品方向を特定Releaseの必須要件へ昇格させない |

## 未確定・暫定案（PROVISIONAL）

- 正式名称、Linux sleep方式（s2idle／deep等）、hibernate実装技術、関連Linux interface・個別OSSの採否。
- swap／storageの構成・容量・配置、暗号化と鍵の保管・利用・破棄・復旧設計。Workspace Exportの鍵設計をhibernateへそのまま転用すると決めない。
- 対応する実機・firmware・driver・周辺機器とサポート範囲、検証条件。実機候補の選定はSleep／Hibernate対応保証ではない。
- ロック、状態保存、Job待機、デバイス停止、復帰時再検証等の処理順序・timeout、保存失敗・遷移失敗時の通知／中止／復旧手順。
- プロセス・AI Task・推論・外部Jobの継続境界、モデルの保持／unload／reload、待機状態や結果不明の識別・照合方式。
- 消費電力、電池持ち、復帰速度、保存時間、失敗率などの数値目標・測定方法・合格条件。性能改善や完全復元の実績は未取得。
- 実装時期、Release割当て、担当、Acceptance、Release／Activation Gate。既存のLinux移行開発承認を本機能の無条件な着手承認へ拡大しない。

## Roadmap・実装との境界

0.3 RevisedのPersonal Agent、0.4+のTask／Role／Activity、0.5–1.0案の資源管理・Workspace継続性・復旧方針と整合する製品要求として記録する。各版への割当て、順序、実装仕様と受入条件は別途定める。本記録は既存の版境界・Workstream所有権・Acceptance・Gateを変更せず、実装済み、Linux image上の受入成功、実機のスリープ／休止／復帰成功を示さない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーの明確な採用確認を受けNagi Sleep（仮称）を記録。LLM非依存のスリープ／復帰／画面ロック、必要な状態保存と接続再検証、AI Jobの安全な待機・資源調整、通常復帰と障害復旧の区別、既存の選択式復旧・権限・二重実行防止、実機検証後の段階的Hibernate方針をCONFIRMEDとし、技術・数値・端末・時期・GateをPROVISIONALに分離。 |

2026-11-16 統合注記: [DEC-067](DEC-067-nagi-power-deterministic-resource-management.md)のPowerをmainに統合済み。上記の並行記録は作成時点の履歴。Powerの継続的資源管理とSleepの移行・復帰の協調を分離し、DEC-014／022の保存・選択式復旧・二重実行防止を継承する。
