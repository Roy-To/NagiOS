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

## IMEの応答性と学習方針（2026-11-16）

[DEC-060](../DEC-060-responsive-ime-ai-learning.md)は、IME／日本語入力のリアルタイム経路で速度と予測可能な応答を優先し、許可されたデータに基づく学習精度向上にAIを活用する製品方針を記録する。秘密入力の除外、明示的な禁止・忘却、機密分類、DEC-002のCloud許可、AI不可時の通常IMEを維持し、モデル非依存の学習状態を目指す。0.3のPersonal Agent／Learning、後続ロードマップの前景応答性・資源管理と整合するが、IME機能の具体方式・数値・版割当ては未確定で、各ロードマップのRelease／Activation Gateを変更しない。

## Nagi Projectとの整合（2026-11-16）

[DEC-055](../DEC-055-project-folder-cross-app-context.md)の通常フォルダをProjectに指定する製品仕様はCONFIRMED。Projectは仕事の対象・関連資料・長期文脈、Workspaceは作業画面・復元状態であり、1 Projectを複数Workspaceから扱える。DEC-023／025の分散正本・許可された共通参照と更新提案、DEC-003／004のMemory規則を継承する。

0.2のResource／Public Action／Permission基盤と0.3のPersonal Agent／Memory／アプリ横断Taskの方向に接続するが、0.3提供を確約せず、0.4+の複数Agentを前提にしない。0.5–1.0の版別範囲・順序も引き続きPROVISIONAL。Projectの技術、UI、リンク形式、詳細のローカル保存方式、版割当てを本追記で確定せず、既存のImplementation Spec、release／activation gate、Acceptance、担当範囲を変更しない。仕様記録は実装・動作検証済みを意味しない。

## OS基盤の検討と進捗（2026-11-16）

[Nagi OS Foundation Checklist](../NAGI_OS_FOUNDATION_CHECKLIST.md)で35項目と7検討束を恒久管理する（[DEC-069](../DEC-069-foundation-checklist-management.md)）。台帳化はCONFIRMED、優先度・検討順序はREVIEW NOTE / PROVISIONAL。各ロードマップの版割当て・Gate・担当・実装開始条件を変更せず、全35項目の完了を新しいrelease条件にはしない。

## 実機テスト候補（2026-11-16）

[DEC-052](../DEC-052-physical-test-device-candidates.md)で、PC＝Raspberry Pi（型番未指定）、タブレット＝PineTab2、スマートフォン／モバイル＝未選定という候補選定方針を記録した。CONFIRMEDは候補選定のみ。起動実績・正式対応・購入・実装時期・対応版は確定せず、各ロードマップの版境界と既存Gateは変更しない。[実機・複数端末の旧展望](../../roadmap/multi-device-outlook.md)の機種選定は同DECの範囲で置き換える。

## 端末横断の電話・連絡先・SMS（2026-11-16）

[DEC-063](../DEC-063-phone-contacts-messaging.md)は、電話・連絡先・SMS/Messagingの純正採用、共通アプリ／モデルと適応UI、可能な限りPeopleへの連絡先統合を確定する。通信・連携の実可用性はハードウェア／OS能力、ユーザー認可、必要なネットワーク・サービス・キャリアに依存し、全端末での通話／SMS送受信を保証しない。各ロードマップのProfile／Permission、単一ユーザーからの段階的拡張方針と整合させ、端末間の遠隔操作・引き継ぎ・同期、アプリ構成や実装版は未確定のままとする。既存の版割当て・Gate・Acceptanceを変更しない。

## PDF Reader / Editorの製品方針（2026-11-16）

[DEC-061](../DEC-061-first-party-pdf-reader-editor.md)で、Nagi純正PDF Reader兼Editorの採用と段階的拡張をCONFIRMEDとして記録した。高速表示・検索・ページ操作・注釈・印刷・フォーム入力などを先に整え、その後、既存本文／画像の高度編集・OCR・元内容の削除と検証を伴う墨消し・署名関連機能を段階的に検討する。通常閲覧・編集はAI非必須とし、Nagi Project / Smart Paste / Writer / Sheets / Slides等と共通Public APIで連携する。AIによる表抽出・要約・比較・引用は利用者の承認と元資料の権限・機密分類の範囲に限る。

具体的なリリース版・高度機能の順序・技術・合格値はPROVISIONAL。本ロードマップの対象版への割当て、Acrobat完全互換、全機能初期実装、実装済み・Acceptance通過を意味せず、既存Gateは変更しない。
