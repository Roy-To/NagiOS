# NAGI Future Roadmaps

**保存先:** `Roy-To/NagiOS` / `docs/product-decisions/roadmaps/`
**更新日:** 2026-11-15([DEC-050](../DEC-050-public-specification-repository.md)により非公開リポジトリから公開リポジトリへ移管)

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

## Nagi Power（2026-11-16）

[DEC-067](../DEC-067-nagi-power-deterministic-resource-management.md)に、AI／LLM推論を必須としないOS全体の決定論的な電力・熱・負荷・背景資源管理を記録した。既存の前景優先・Model／Job／Memory資源制御と接続し、必要な低レイヤー基盤を広く整備する方向を確認した。具体的な全サブシステムの採用・実装着手・対応端末・効果・版割当て・Gateは確定しない。Master Roadmapの0.6配置は引き続きPROVISIONAL。

変更履歴: 2026-11-16 — Nagi Powerの決定を参照追加。各ロードマップの既存版境界とGateは変更しない。
