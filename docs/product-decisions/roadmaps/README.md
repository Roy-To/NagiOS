# NAGI Future Roadmaps

**保存先:** `Roy-To/NagiOS` / `docs/product-decisions/roadmaps/`
**更新日:** 2026-11-15([DEC-050](../DEC-050-public-specification-repository.md)により非公開リポジトリから公開リポジトリへ移管)

| ファイル | 位置付け |
|---|---|
| [Nagi_OS_0.3_Architecture_Roadmap_Revised.md](Nagi_OS_0.3_Architecture_Roadmap_Revised.md) | 0.3 Architecture Draft、2026-10-25 Vision addendum、2026-11-04 Memory Engineの未確定方式を明記。`docs/Nagi_OS_0.3_Architecture_Roadmap.md` の改訂版 |
| [Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md](Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md) | 0.4+ Vision proposal。Implementation Specificationではない |
| [Nagi_OS_0.5-1.0_Master_Roadmap.md](Nagi_OS_0.5-1.0_Master_Roadmap.md) | DEC-021に基づく段階案。継承方針と暫定の版割当てを区別し、Implementation Specification / Release commitmentではない |

0.3と0.4+の文書は製品構想、0.5–1.0 Master Roadmapは製品決定と承認済み方向を整理した提案である。どれも実装済みであることやreleaseの確定を意味しない。

## 当面の優先順位（2026-11-16）

[DEC-074](../DEC-074-human-led-agent-organization.md)の後続確認により、当面はAI組み込み体験型OSの確立を優先し、階層型Agent Organizationは非優先の長期ビジョンとする。各Roadmapに日付付き注記を反映した。具体的な提供版・階層数・実装順序は未確定であり、0.3の単一Personal Agent、初期0.4の単一Nagi builder Role、既存Gate・Acceptanceは維持する。

## Linux方針との整合(2026-11-13)

各ロードマップにDEC-042/043/044の後続関係を追記した。[仕様・実装影響表](../KERNEL-TRANSITION-ALIGNMENT-2026-11-13.md)を参照。

## Nagi Projectとの整合（2026-11-16）

[DEC-055](../DEC-055-project-folder-cross-app-context.md)の通常フォルダをProjectに指定する製品仕様はCONFIRMED。Projectは仕事の対象・関連資料・長期文脈、Workspaceは作業画面・復元状態であり、1 Projectを複数Workspaceから扱える。DEC-023／025の分散正本・許可された共通参照と更新提案、DEC-003／004のMemory規則を継承する。

0.2のResource／Public Action／Permission基盤と0.3のPersonal Agent／Memory／アプリ横断Taskの方向に接続するが、0.3提供を確約せず、0.4+の複数Agentを前提にしない。0.5–1.0の版別範囲・順序も引き続きPROVISIONAL。Projectの技術、UI、リンク形式、詳細のローカル保存方式、版割当てを本追記で確定せず、既存のImplementation Spec、release／activation gate、Acceptance、担当範囲を変更しない。仕様記録は実装・動作検証済みを意味しない。

## OS基盤の検討と進捗（2026-11-16）

[Nagi OS Foundation Checklist](../NAGI_OS_FOUNDATION_CHECKLIST.md)で35項目と7検討束を恒久管理する（[DEC-069](../DEC-069-foundation-checklist-management.md)）。台帳化はCONFIRMED、優先度・検討順序はREVIEW NOTE / PROVISIONAL。各ロードマップの版割当て・Gate・担当・実装開始条件を変更せず、全35項目の完了を新しいrelease条件にはしない。

## 実機テスト候補（2026-11-16）

[DEC-052](../DEC-052-physical-test-device-candidates.md)で、PC＝Raspberry Pi（型番未指定）、タブレット＝PineTab2、スマートフォン／モバイル＝未選定という候補選定方針を記録した。CONFIRMEDは候補選定のみ。起動実績・正式対応・購入・実装時期・対応版は確定せず、各ロードマップの版境界と既存Gateは変更しない。[実機・複数端末の旧展望](../../roadmap/multi-device-outlook.md)の機種選定は同DECの範囲で置き換える。
