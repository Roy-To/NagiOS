# DEC-094 — Resource Manager / Scheduler：Linux標準を基本にNagi独自スケジューリングを並行研究

**状態:** CONFIRMED（Foundation #25のB＋C研究案：Linux標準スケジューラー／資源管理を製品基本構成とし、Nagi Resource Managerからアプリ・AI・電力の配分を協調制御する。独自スケジューリングの比較研究を進め、十分な実機検証を満たした対象範囲のみ後続採用判断し、標準方式に戻れる設計を維持）／PROVISIONAL（Linux制御機構・独自アルゴリズム・測定数値・実機・導入版・正式採否）

**最終合意日:** 2026-11-16\
**適用候補:** Nagi OS Foundation #25。特定Releaseへの割当て・実装着手時期は未確定\
**関連:** [Foundation Checklist #25](NAGI_OS_FOUNDATION_CHECKLIST.md#f25)、[DEC-020（前景優先・資源配分）](DEC-020-ai-os-interaction-path.md)、[DEC-067（Nagi Power）](DEC-067-nagi-power-deterministic-resource-management.md)、[DEC-013（Hardware-Adaptive Memory）](DEC-013-lightweight-memory-engine-benchmark.md)、[DEC-022（Task復旧）](DEC-022-builder-interrupted-task-recovery.md)、[DEC-043（Linux／専用ユーザーランド）](DEC-043-nagi-userland-independent-os.md)、[DEC-077（LTS標準／Stable先行検証）](DEC-077-linux-kernel-lts-stable-policy.md)、[Roadmaps](roadmaps/README.md)

## 目的と背景

Nagiでは、通常アプリの前景操作、Nagi Home／Desktop、音声会話、ローカルLLM推論、検索／索引、バックグラウンドJobがCPU／RAM／GPU／NPU／I/O・電力・熱を共有する。一般的なデスクトップの処理に加えて、AIが実行中でも人間の操作を快適に維持することが製品上重要である。既存のDEC-020／067の前景優先・安全・非特権AI・決定論的資源管理を継承し、OS側でどこまで自動配分するかを定める。

## 確定した製品方針（CONFIRMED：B＋C研究）

1. **製品の標準動作はLinux基盤＋Nagi独自管理（B）。** Linuxの標準CPUスケジューラーおよび既存の資源制限・観測機能を基本として利用する。Nagi独自Resource Managerは通常アプリ、前景UI、音声、Model Runtime、Job、Memory処理、Nagi Power等の資源要求・優先度・予算を協調管理する。Linuxの汎用スケジューラー全面置換を標準要件にしない。
2. **Cの価値を狙った並行研究を実施する方針。** 端末・負荷に応じたNagi向けCPUスケジューリング、AIと前景UIの動的優先制御等の研究・試験を進め、Linux標準構成との実測比較によって効果を判断する。カーネル全面改造は研究開始の必須条件ではない。
3. **研究成果の製品組込みは別判断。** 前景応答性、AI Taskの品質・処理時間、メモリ、電力・発熱、音声処理、安定性、公平性・飢餓リスクなどを実機・現実的な負荷で比較し、十分な改善と安全性を確認できた範囲だけ後続の採用候補とする。効果の数値や優位性は未実証であり、現時点で独自スケジューラーを正式採用しない。
4. **標準方式へ戻れることを重視。** 実験・部分最適化の成否や対応kernel／機器の制約によらず、Linuxの標準スケジューリング・資源管理を製品の正常な基本経路として保持する。機能不在や問題発生時の具体的な退避契約は後続設計と実機検証で定める。
5. **既存の資源管理責務を重複させない。** OS Resource Managerは予算・強制・観測の調整役であり、Job SchedulerのTask lifecycle、Model Runtimeの推論／モデル配置、Memory Engineの検索方式選択、Nagi Powerの継続的な電力・熱制御、Permission／Policyの判断を置換しない。AI／Modelにscheduler制御の特権を直接与えない。
6. **安全とユーザー意思が優先。** 前景操作とシステム安全を標準で優先し、明示的なTask優先指定は熱・電力・機能維持・資源上限・Permission／Policyを越えない。資源待機はユーザーのPause／Cancel／Stopと区別し、許可が失効したJobや結果不明のActionを資源回復だけで自動再実行しない。

## 既存の確定仕様との境界

- [DEC-020](DEC-020-ai-os-interaction-path.md)の前景優先・CPU／GPU／NPU等の適応的な資源配分と、AI不在でも通常操作が可能であることを維持する。
- [DEC-067](DEC-067-nagi-power-deterministic-resource-management.md)のLLM非必須のOS全体の電力・熱管理と協調する。Nagi PowerとOS Resource Managerは別系統の競合する管理者にならない。
- [DEC-013](DEC-013-lightweight-memory-engine-benchmark.md)のHardware-Adaptive Memoryとモデル非依存Canonical Memoryを保持し、索引／検索方式選択の正本を別に増やさない。
- [DEC-022](DEC-022-builder-interrupted-task-recovery.md)等の承認・Scope・Pause／Cancel・副作用二重実行防止を維持し、資源最適化をTaskの自動再開・再試行許可と混同しない。
- Linux kernelの製品標準はLTS、Stable先行検証は可能とする[DEC-077](DEC-077-linux-kernel-lts-stable-policy.md)の方針を変更しない。研究候補がLTS等で利用可能と仮定しない。

## 研究候補・未確定事項（PROVISIONAL／OPEN）

- Linux標準構成での具体的な観測／制御方法（cgroup v2、PSI等は**候補**）。採用するcgroup階層、CPU／memory／I/O quota・weight、優先度・OOM・圧迫時制御。
- Linuxの拡張可能なCPUスケジューリングの評価方法（`sched_ext`／eBPF等は**研究候補**）。利用するkernelの機能・APIの安定性、対応端末、kernel更新時の保守負担、fallbackの成立を確認し、採用済み技術とみなさない。
- CPU配分とGPU／NPU制御を同一視しない。GPU／NPUのdriver／runtime、Memory／I/O／熱と連携する具体的な実装・利用可能な信号と限界。
- 何をResource Managerが所有し、何をJobs／Model Runtime／Memory／Powerへ委譲するかという予算／要求API・責任境界、複数ユーザー／Profile・公平性。
- 比較Baseline、ベンチマーク負荷、前景UI遅延・音声割り込み・AI推論品質と所要時間・電力・温度・RAM・安定性・飢餓の指標／閾値／測定装置・結果。
- 研究の実施順・担当・着手条件・実機種、製品への採否、適用バージョンとAcceptance、kernel変更・patchの有無と保守体制。
- 試験が成立しない端末やkernelでの標準方式への退避、切替時の安全確保と副作用の扱い。

## Roadmapと実装の境界

本DECは#25の**製品標準の方針**と、**C方式の一部を研究・比較する方向**をCONFIRMEDとする。コード実装・kernel patch・`sched_ext`・eBPF採用、性能改善、Linux target／実機受入、Release／Activation Gate変更を意味しない。Desktop firstと端末横断の製品目標、0.3 Hardware-Adaptive MemoryのGate、0.5–1.0における資源管理の暫定版配置は維持する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーがFoundation #25の「B＋C研究案」を明示採用。Linux標準scheduler／資源機構とNagi独自Resource Managerを製品の基本にし、独自スケジューリングを並行研究・比較する方針をCONFIRMED。具体的な`cgroup v2`／PSI／`sched_ext`等の採用、改善効果、実機試験結果、製品採用・提供版は未決定。 |
