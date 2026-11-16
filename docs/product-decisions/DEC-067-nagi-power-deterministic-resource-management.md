# DEC-067 — Nagi PowerとOS全体の決定論的な電力・熱・資源管理

**状態:** CONFIRMED（採用・製品方針）／PROVISIONAL（名称の正式化・具体方式・実装版・Gate）  
**最終合意日:** 2026-11-16  
**適用候補:** Linuxカーネル + Nagi専用ユーザーランド。対応端末・実装版・時期は未確定  
**関連:** [DEC-020](DEC-020-ai-os-interaction-path.md)、[DEC-013](DEC-013-lightweight-memory-engine-benchmark.md)、[DEC-022](DEC-022-builder-interrupted-task-recovery.md)、[DEC-042](DEC-042-linux-kernel-transition.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-052](DEC-052-physical-test-device-candidates.md)、[純正ソフトウェア実装仕様](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md)、[roadmaps](roadmaps/README.md)

## 目的と位置付け

ユーザーが採用に同意した「Nagi Power（仮称）」を、OS全体の電力・熱・負荷・バックグラウンド処理の資源管理として記録する。根拠は本記録依頼で明示された決定内容と、低レイヤーの基盤を積極的に整備する方針への明確な同意である。省略された過去の提案を補完して確定事項を増やさない。

DEC-020の前景応答性優先・AI資源配分を、OS全体の管理の中へ位置付ける補足決定であり、同じAIスケジューラやMemory Engineを別途重複採用する決定ではない。

> **2026-11-16 電源ライフサイクル統括の後続決定:** [DEC-082](DEC-082-power-lifecycle-coordination.md)で、全電源状態遷移のNagi一元管理・各サービスの保存／停止／復帰準備と失敗時対応をCONFIRMEDとした。本DECのNagi PowerはOS全体の継続的な電力・熱・負荷・背景資源管理を扱い、DEC-082は電源状態遷移の協調を扱う。両者の重複するSchedulerや保存正本を新設しない。具体的な実装境界・プロセス構成・Releaseは未決定。

## 確定した仕様（CONFIRMED）

1. **Nagi Power（仮称）を採用する。** LinuxカーネルとNagi専用システムサービスにより、OS全体の電力・熱・負荷・バックグラウンド処理の資源を決定論的に管理する。製品ポリシーと制御の責務をNagi側で持ち、具体的なプロセス構成やLinux interfaceは未確定とする。
2. **安全性と操作応答性を優先する。** バッテリー残量、AC／電池駆動、CPU／GPU負荷、温度／スロットリング、前景操作の状況に応じて制御する。端末ごとの取得可能な信号・センサーと対応範囲は検証待ちであり、すべての端末で測定できると保証しない。
3. **AI／LLM推論を電力管理の必須経路に入れない。** 監視・制御の実行判断は通常のOS／システムサービスで行う。AIが無効・不在・利用不能でも電力・熱・資源管理が動作する。
4. **AI処理もOSが管理する資源利用者とする。** Model Manager／Model Runtime、Job Scheduler、Indexing、Memoryの検索・索引整備等は、通常の資源管理契約に従ってthrottle、queue、suspendされ、必要時にはmodel unloadされる。AIに電力管理やスケジューラの特権を直接与えない。AI専用の迂回経路や無制限の常駐・並列処理を認めない。
5. **ユーザーは安全限界内で優先度を変更できる。** 前景操作の応答性を標準で優先しつつ、特定のAI Taskを優先する等の指定を可能にする。優先度変更も安全限界、OS機能維持、資源上限、Permission／Capability／Policyを越えない。UI・profile・数値は未確定。
6. **自動調整でTaskとActionの意味を変えない。** 承認済みActionを重複実行せず、ユーザーが明示Pause／Cancel／StopしたTaskを自動再開しない。資源待機からの継続はDEC-020に従い、現行の承認・Scope・期限・Permission／Policy・Resource Revision・Budget・取消状態を再検証する。クラッシュ／再起動後の復旧や実行結果不明のActionは資源回復だけで再実行せず、DEC-022等の既存規則に従う。
7. **必要な低レイヤー基盤を今後広く整備する。** これは製品方向の合意であり、具体的な全サブシステムの採用、担当割当て、実装着手、版割当て、Release／Activation Gateの変更を一括で確定するものではない。

## 既存仕様との役割分担（REVIEW NOTE）

| 既存の正本 | 既に扱う範囲と本決定の関係 |
|---|---|
| DEC-020 | 決定論的OS処理、非特権AI、前景優先、ユーザーの優先指定、Resource待機と明示停止の区別を維持する。本決定はOS全体の電力・熱管理の採用を補足する |
| DEC-013／[Memory Resource Governor案](MEMORY-RESOURCE-GOVERNOR-DESIGN.md) | Memory検索経路・索引整備の適応方針を維持する。Memory側の処理選択とOS全体の予算・制限を接続するが、Governor案のAPI・閾値・検索技術を確定しない。Canonical Memory、由来、権限、保護・忘却を資源調整で変更しない |
| DEC-042／043 | Linuxカーネル + Nagi専用ユーザーランドを前提とする。既存ディストリビューションの電源管理構成や個別OSSを一括採用する決定ではない |
| 純正ソフトウェア実装仕様 §65 Devices／§67 Models | Devicesの端末信頼・Continuityと、ModelsのProvider／モデル管理を維持する。Nagi Powerは端末一覧UIやModel Routerそのものを置換しない |
| 0.2実装仕様 §48 Resource-pressure policy | 非活動モデルのunload、背景アプリのsuspend、Shell／前景／音声の保護という既存契約と整合する。電力・熱管理の採用から、既存0.2の実機・suspend/resume対応を完了扱いにしない |
| 0.3 Revised／0.4+／0.5–1.0 roadmap | Model・Jobの予算、Queue／Throttle／Pause／Unload、前景優先の構想を継承する。0.5–1.0案の0.6資源管理配置を本決定で確定しない |

ここでの決定論的管理は、推論に依存しないポリシー・認可・制御の方針を指す。Linuxの実行順序や応答時間が常に同一になるという保証ではない。

## 未確定・暫定案（PROVISIONAL）

- 正式名称、サービス・プロセスの分割、観測・制御API、Linux interface（cpufreq、thermal、powercap、cgroup、eBPF等）の採否。列挙は候補であり必須構成ではない。
- 実機センサー、CPU／GPU等のdriverとの接続、測定不能・古い信号・取得失敗時の具体的な扱い。
- governor、閾値、sampling間隔、ヒステリシス、power profiles、バッテリー保護・充電制御の具体範囲。
- 自動調整・公平性・飢餓回避・待機継続の具体アルゴリズム、model unloadやcheckpointの条件、Task状態の表示・説明。
- バッテリー寿命、省電力・発熱・応答性の改善効果、計測方法、baseline、合格数値。実測前に効果を保証しない。
- 対応端末、実装版、担当、実装着手・統合時期、Acceptance／Release／Activation Gate。DEC-052の実機候補をNagi Power対応済みとしない。
- **将来の任意機能候補:** AIによる電力使用の説明や設定提案。採用・自動適用は未確定であり、基本管理の必須経路にしない。

## Roadmap・実装との境界

本記録は製品仕様の決定であり、実装・実機検証・Linux image上の受入・性能達成の証拠ではない。既存のLinux移行開発承認はその対象範囲で有効だが、本決定だけから新しい全基盤の実装開始を推定しない。既存の0.1／0.2／0.3／0.4+等の版境界、所有権、Acceptance、Release／Activation Gateは変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | Nagi Power（仮称）の採用、AI非必須の決定論的OS全体管理、通常契約によるAI資源制御、安全限界内の優先指定、Task／Action境界、低レイヤー基盤整備の方向をCONFIRMEDとして記録。具体方式・効果・端末・実装版・Gateと任意のAI説明機能は未確定に分離。 |

2026-11-16 責務整合: [DEC-068](DEC-068-nagi-sleep-resume-recovery.md)がSleepを採用する。Powerは継続的な電力・熱・資源管理、Sleepは電源状態の移行・復帰の協調を扱う。Workspace／Task保存・復旧の正本はDEC-014／022であり、別のSchedulerや保存正本を重複採用しない。
