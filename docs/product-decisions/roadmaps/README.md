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

## Nagi Handoff採用・変更履歴（2026-11-16）

[DEC-065](../DEC-065-nagi-handoff-work-continuation.md)でNagi Handoff（仮称）を、PC・タブレット・スマートフォン間で文書・Resource・編集箇所・意味的な作業状態を継続する純正OS機能として採用した。既存Continuity、Homeの `home.continue_from_device`、Device Capability、DEC-014の選択式Workspace移行／同期を最大限再利用し、受信先の画面・能力に応じてUIを再構築する。現行User／Profile／Owner／Permission／Privacy・Source有効性／Revisionを再検証し、実体がなければユーザー許可に基づく取得／転送経路または代替を提示する。無断のファイル・個人データ転送／クラウド同期・競合上書きをせず、中断AI Task／副作用Actionを自動再開・再実行しない。名称・UI・復元範囲・経路・共有方式・クラウド利用・瞬時性・対応版はPROVISIONAL。全アプリ・全状態の完全移行、常時オフライン成立、自動同期は保証しない。単発ファイル転送のNagi Shareとは別の概念である。既存Desktop first、Release／Activation／Acceptance Gate、版割当てを変更せず、実装済みを示さない。 0.3／0.4+／0.5〜1.0のどのVersionへ提供するかは本記録では確定しない。
