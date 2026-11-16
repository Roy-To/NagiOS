# DEC-082 — 電源ライフサイクルのNagi一元管理とサービス協調

**状態:** CONFIRMED（Nagiによる電源ライフサイクル全体の一元管理・サービス協調・失敗状況に応じた中止／再試行／警告・ユーザーデータと安全性の優先）／PROVISIONAL（状態機械・手順／順序・Timeout・強制終了・プロセス構成・実機／Release）
**最終合意日:** 2026-11-16
**適用範囲:** Foundation Checklist #6「電源ライフサイクル」（検討束B2）。Linuxカーネル＋Nagi専用ユーザーランド上のshutdown／reboot／sleep／resumeと、将来対応するhibernateとの協調
**関連:** [DEC-067 Nagi Power](DEC-067-nagi-power-deterministic-resource-management.md)、[DEC-068 Nagi Sleep](DEC-068-nagi-sleep-resume-recovery.md)、[DEC-014 Workspace](DEC-014-workspace-state-checkpoints.md)、[DEC-022 Task復旧](DEC-022-builder-interrupted-task-recovery.md)、[DEC-043 Linux＋Nagi userland](DEC-043-nagi-userland-independent-os.md)、[DEC-079 System Image](DEC-079-system-image-readonly-ab-data-separation.md)、[DEC-080 Init選定](DEC-080-init-service-manager-ab-evaluation.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[Roadmaps](roadmaps/README.md)

## 目的と位置付け

スリープ／復帰の個別方針は[DEC-068](DEC-068-nagi-sleep-resume-recovery.md)、継続的な電力・熱・負荷・背景資源管理は[DEC-067](DEC-067-nagi-power-deterministic-resource-management.md)で決定済み。本DECはそれらを置き換えず、シャットダウン／再起動を含む**電源状態の遷移をOS全体でどう統括し、各サービスがどう協調するか**という上位の製品方針を定める。

## 確定した製品方針（CONFIRMED）

1. **Nagiが電源ライフサイクル全体を一元管理する。** シャットダウン、再起動、スリープ、復帰、および段階的に対応する休止状態の移行・復帰について、サービス横断の準備・結果・異常の管理責務をNagi側に置く。カーネル・実機の電源制御にはLinuxの仕組みを活用し、AI／LLM推論を必須の制御経路としない。
2. **各サービスの保存・停止・復帰準備を協調させる。** 電源操作の際、Nagiが必要なサービスへ状態遷移を通知・調整し、アプリ／Workspaceの正規の保存、AI Jobの待機・停止、ストレージ書込みの完了、Session・Deviceの処理、復帰時の再検証等を、各機能の既存契約とともに扱う。全対象の完全保存・常時成功を保証する決定ではない。
3. **失敗に応じて中止・再試行・警告を使い分ける。** 保存・停止・復帰準備等が失敗した場合、可能な条件では電源操作を中止して通常の利用状態を維持または復帰させ、必要に応じて再試行・利用者への警告や対処の選択肢を提供する。無条件に操作を続けたり、成功していない処理を完了扱いにしたりしない。
4. **ユーザーデータと安全性を優先する。** 通常の電源操作では作業内容の保護を図る。ただし、致命的な障害・緊急停止・安全確保が必要な場合まで、操作を無期限に待機させる保証はしない。緊急時の具体的な優先順位、強制停止条件・タイムアウトやUIは別途定める。
5. **Nagiの既存サービス契約と権限境界を維持する。** 復帰後もIdentity／Session、Permission、Device／Networkの現行状態を再検証する。Workspace復元とAI Taskの実行は別であり、障害・再起動後の中断Taskや副作用Actionを無断再開・再実行しない。通常のRAM保持スリープ復帰は障害後の復旧と区別する。

本決定は「Lifecycle Coordinator（仮称）」と呼ぶ**論理的な統括責務**の採用を意味する。独立した常駐プロセス・新デーモン・特定APIを作る決定ではなく、既存Nagi Service Managerへの統合などの構成は未確定。

## 操作別の設計意図（REVIEW NOTE／具体条件はPROVISIONAL）

| 操作 | 協調時に検討する扱い | 別途決めること |
|---|---|---|
| Shutdown | 保存・停止・書込完了を試みる。未保存や失敗を検出したら安全性に応じて中止／警告／対処選択 | 強制停止の条件、データ保護範囲、全操作の順序 |
| Reboot | 終了側はShutdownと同様の準備を行い、再起動後は起動・更新・復旧の既存契約に従う | 更新・A/B slotとの接続、途中失敗時の状態 |
| Sleep | 必要な状態保存・Job待機・Session／Device管理を協調し、安全に遷移できない場合は可能なら通常状態へ戻す | ロック／保存／Device停止の順序、キャンセル条件 |
| Resume | Device／Network／Session／Permission等を再検証し、利用できない機能や結果不明を明示する | 再検証手順、失敗時の部分復帰・通知 |
| Hibernate | DEC-068に従い対応実機で検証して段階的に追加 | 対応機器、ストレージ・暗号化、復帰可否、手順 |

表は技術設計時の確認例であり、未決定の状態遷移や例外処理を新しい確定仕様に昇格させない。

## 既存決定との整合と安全境界

- **DEC-068 Nagi Sleep:** Linux標準機構＋決定論的Nagiサービス、画面ロック・状態保存・接続再検証・AI Job協調、通常スリープ復帰とクラッシュ／電源断後の復旧の区別、実機検証後の段階的hibernateを維持する。全アプリ・全プロセスの完全復元や全端末の休止状態対応は保証しない。
- **DEC-067 Nagi Power:** 電力／熱／負荷・資源配分の**継続的な管理**を担い、本DECの**電源状態遷移の統括**と分ける。重複するAI Schedulerや保存正本を新設しない。
- **DEC-014／022:** 保存と復元の対象を分け、権限・Privacy／Resource Revisionを遵守し、障害後の復旧はユーザーによる選択と再検証を必要とする。結果不明の副作用Actionを盲目的に再送しない。通常復帰で有効なTaskが条件付き継続できる場合も、明示Pause／Cancel／Stopや許可期限を無視しない。
- **DEC-079:** System A/Bの更新／Recovery・User Data分離を維持する。通常rebootを起動imageの成功判定やユーザーデータの完全バックアップと同一視しない。
- **DEC-080:** Init／PID 1のA/B短期実装比較と最終選択は引き続き未決定。本決定によってInit実装方式やLifecycle Coordinatorのプロセス配置を先に固定しない。
- **DEC-043:** Nagi専用ユーザーランドとLinuxカーネルの責務分離を維持し、既存Linuxディストリビューションのサービス・電源管理構成一式を製品基盤に流用すると決めない。AIに電源制御の特権を与えない。
- **Release／実装境界:** 0.2仕様のsuspend／resume非必須範囲、現行のRelease／Activation／Acceptance Gate、担当・実装権限を変更しない。Nagi Linux image・実機の電源遷移や障害試験の成功は未確認。

## 未確定事項（PROVISIONAL／OPEN）

- 電源遷移の状態機械と対象サービス一覧、依存関係・準備／保存／停止／復帰の厳密な順序、通知・取消・完了判定・Health契約。
- 「中止可能」「再試行可能」「警告が必要」の判定条件、警告表示・ユーザー選択UI、強制停止・緊急電源断との境界とタイムアウト／回数。
- Linuxの具体interface、各機種のfirmware／driver／suspend方式、hibernateの対応範囲、暗号化・ストレージ要件。
- Lifecycle Coordinatorの実装場所（Service Manager統合・独立プロセス等）、必要なAPI／認可・ログ・障害診断の方式。
- 実測性能、電源遷移失敗／電源断／復帰不良の検証条件、対応実機、提供版、正式Acceptance・実装担当。

## 技術設計への引渡し（REVIEW NOTE）

単一のLinux開発targetで、通常shutdown／reboot／sleep要求をシミュレーションして、サービスの準備・保存結果・キャンセル／失敗・有限の待機・復帰の状態遷移を検証する。その後、対象機器のdriver／firmwareによる実際の電源遷移を別に評価する。失敗して保存できなかった状態を「保存済み」と扱わず、旧kernelの成功証拠をLinux targetのPASSへ流用しない。

## Roadmapとの関連

0.3や後続ロードマップ上のWorkspace・AI Task継続、資源／電力管理と整合する製品方針として記録する。どの版でいつ実装・提供するかは未決定。既存の0.1／0.2／0.3以降のGateを変更せず、全機能を同時実装する義務を新設しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーが「Nagiが電源ライフサイクル全体を一元管理し、各サービスの保存・停止・復帰準備を協調させる。失敗時は状況に応じて操作を中止・再試行・警告し、ユーザーデータと安全性を優先する」方針を明確に承認。具体順序・時間・強制終了条件・プロセス構成を未確定とした。 |
