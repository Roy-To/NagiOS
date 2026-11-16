# NAGI Future Roadmaps

## 近距離ファイル共有の製品方針（2026-11-16）

[DEC-064](../DEC-064-nagi-share-nearby-file-transfer.md)でNagi Share（仮称）をNagiのPC／Tablet／Phone間の純正ファイル共有機能として採用した。共有UIから近くの受信可能端末を発見・選択し、受信側の確認・許可、暗号化・端末認証と既存の認可・情報区分・所有／Profile境界に従ってローカルに直接転送する。Internet・Cloud account・LLMは基本利用に必須としない。

日常の単発ファイル転送はDEC-014のWorkspace移行・同期、Team／Organization／Remote Agentとは別の範囲である。名称・通信方式・互換・保存先・距離・UI／通知／承認詳細・対応版／端末・性能はPROVISIONALで、0.8等の特定版へ割り当てず、Desktop firstと既存Release／Activation Gateを維持する。Nagi Shareからページ／Project／Workspace／作業状態Handoffへ拡張する案はアイデアであり、既存Continuity仕様の撤回でも、その拡張の採用確定でもない。

**保存先:** `Roy-To/NagiOS` / `docs/product-decisions/roadmaps/`
**更新日:** 2026-11-16（公開移管は2026-11-15、[DEC-050](../DEC-050-public-specification-repository.md)を参照）

| ファイル | 位置付け |
|---|---|
| [Nagi_OS_0.3_Architecture_Roadmap_Revised.md](Nagi_OS_0.3_Architecture_Roadmap_Revised.md) | 0.3 Architecture Draft、2026-10-25 Vision addendum、2026-11-04 Memory Engineの未確定方式を明記。`docs/Nagi_OS_0.3_Architecture_Roadmap.md` の改訂版 |
| [Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md](Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md) | 0.4+ Vision proposal。Implementation Specificationではない |
| [Nagi_OS_0.5-1.0_Master_Roadmap.md](Nagi_OS_0.5-1.0_Master_Roadmap.md) | DEC-021に基づく段階案。継承方針と暫定の版割当てを区別し、Implementation Specification / Release commitmentではない |

0.3と0.4+の文書は製品構想、0.5–1.0 Master Roadmapは製品決定と承認済み方向を整理した提案である。どれも実装済みであることやreleaseの確定を意味しない。

## Linux方針との整合(2026-11-13)

各ロードマップにDEC-042/043/044の後続関係を追記した。[仕様・実装影響表](../KERNEL-TRANSITION-ALIGNMENT-2026-11-13.md)を参照。

## 実機テスト候補（2026-11-16）

[DEC-052](../DEC-052-physical-test-device-candidates.md)で、PC＝Raspberry Pi（型番未指定）、タブレット＝PineTab2、スマートフォン／モバイル＝未選定という候補選定方針を記録した。CONFIRMEDは候補選定のみ。起動実績・正式対応・購入・実装時期・対応版は確定せず、各ロードマップの版境界と既存Gateは変更しない。[実機・複数端末の旧展望](../../roadmap/multi-device-outlook.md)の機種選定は同DECの範囲で置き換える。
