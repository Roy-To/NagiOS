# DEC-080 — Init / Service Manager：A案優先・B案との短期実装比較

**状態:** CONFIRMED（A案を第一候補とし、B案との短期実装比較後に最終選択する判断プロセス）／PROVISIONAL（A/Bの正式採用、具体的な実装方式・期間・比較結果）
**最終合意日:** 2026-11-16
**対象:** Nagi OS Foundation Checklist #4「Init / Service Manager」（検討束B1）。Linuxカーネル＋Nagi専用ユーザーランドへの移行におけるinit／サービス管理方式の選定
**関連:** [DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-044](DEC-044-linux-migration-development-authorization.md)、[DEC-047](DEC-047-linux-migration-workspaces-and-integration.md)、[DEC-049](DEC-049-development-process-speed-priority.md)、[DEC-069](DEC-069-foundation-checklist-management.md)、[DEC-078](DEC-078-boot-platform-entry-and-common-nagi-startup.md)、[Foundation Checklist](NAGI_OS_FOUNDATION_CHECKLIST.md)、[Roadmaps](roadmaps/README.md)

## 背景と目的

Nagiのユーザーランドを独自設計する長期方針は[DEC-043](DEC-043-nagi-userland-independent-os.md)でCONFIRMED。ただし、Linux上のPID 1とService Managerを全面独自実装するか、既存の軽量Initを使うかは未決定だった。

ユーザーは、独自Initによる制御の一貫性・独立性を重視する一方、Linux起動・サービス統合の開発速度への影響を重視する。予測値だけでA案を採用せず、限定的な実装比較で判断する。

## 確定した判断プロセス（CONFIRMED）

1. **A案を第一候補に置く。** A案は「Nagi独自の最小限のLinux PID 1（Init）と、それとは責務を分けたNagi独自Service Manager」の方向とする。最終的な製品実装方式としてのA案の採用は、まだ確定しない。
2. **B案と短期間の実装比較を行う。** B案は「既存OSSの軽量Init／PID 1を基盤とし、上位はNagi独自のService Managerで管理する」方向とする。評価は机上比較だけでなく、実際のLinux上での限定的な実装と検証によって行う。
3. **比較後に最終決定する。** A案の優先順位は、比較結果にかかわらず無条件でA案を製品採用することを意味しない。開発工数・成立性・安定性等の結果を確認してから、A案かB案かを改めて決める。

「短期間」は限定試行の方向性であり、ユーザーが具体的な日数、期限、作業量、採用条件の閾値を承認したものではない。

## 比較候補の定義と未確定の採用判断

| 事項 | A案（第一候補・未採用） | B案（比較対象・未採用） |
|---|---|---|
| Linux PID 1 / Init | Nagiが最小限の責務を独自実装 | 既存の軽量OSSを評価・活用 |
| サービス管理 | Nagi独自Service Manager | Nagi独自Service Manager |
| 基本設計 | PID 1に複雑なサービス／AI管理を詰め込まず別プロセスへ分離 | 既存InitとNagi管理の責務を分離 |
| 現在の位置付け | 第一候補。**正式採用未決定** | 比較候補。**正式採用未決定** |

上記の候補定義はA/B比較対象を明確にするものであり、既存Initの製品搭載や、Nagi PID 1の恒久的な仕様・採用を確定しない。

## 評価時の確認事項（REVIEW NOTE／試験内容はPROVISIONAL）

開発速度とNagi独自の管理体験の両方を評価するため、可能な範囲で**同じLinux開発target、同じ上位Service Manager契約、同じ失敗シナリオ**を使って比較することを推奨する。比較のための追加作業が長期化し、Linux最小統合を妨げないよう、範囲を限定する。

- **初期工数:** Linux BootからPID 1、Nagi Service Manager、最小Nagi serviceの起動までに必要な変更と検証。工数は実測し、予測値で固定しない。
- **基本成立性:** PID 1の初期化、子・孤児プロセス回収、正常終了、サービスの依存順序とready／healthの扱い。
- **異常系:** Service Managerのクラッシュ・再起動、再起動上限、依存サービスの失敗、障害の観測と診断。PID 1自体の重大障害に対する安全な失敗動作。
- **独自管理との親和性:** Nagi Permission、User／Session所有、将来のNagi Power／Sleep等への接続容易性。AI／LLMなしでも起動・監視・復旧できること。
- **複雑性・保守:** 起動責務の二重化、既存OSS依存、試験可能性、変更リスク、継続保守負担。
- **証拠区分:** hostテスト／Linux上の検証／Nagi Linux imageでの統合試験を混同せず、未実施・未確認を明記する。

比較の実施日、上限日数・時間、機器、選定したOSS、個別担当、評価結果、A/B選択の合格値はこのDECでは確定しない。評価手順を変更可能にし、測定結果と残るリスクを報告した後、ユーザーが製品方針を最終判断する。

## 維持する既存境界

- [DEC-043](DEC-043-nagi-userland-independent-os.md)のLinuxカーネル＋Nagi専用userland、既存ディストリビューション基盤を流用しない方針、個別OSSの選択的利用可能性を維持する。B案の試行はディストリビューション全体の採用を意味しない。
- 既存0.1の`user/nagi-init/`／Supervisorの設計、0.2 Supervisorの起動・依存・健康状態・停止・再起動・権限等の製品契約は移行比較の参照資産であり、旧guestの動作をLinux PID 1動作や受入PASSと扱わない。
- [DEC-044](DEC-044-linux-migration-development-authorization.md)／[DEC-047](DEC-047-linux-migration-workspaces-and-integration.md)の既存限定移行開発許可と実行範囲を継承する。今回の決定だけで、未許可の実行環境・資源・共同作業の所有権を追加承認したり、実装済み・比較開始済み・PASSと報告しない。
- AI非特権・Nagi Permission／Capabilityの既存境界は不変。Linux UID／rootによってNagi権限モデルの同等性を自動保証しない。
- 0.1／0.2／0.3以降のRelease／Activation／Acceptance Gate、特定リリースの採用・切替日は変更しない。
- これをFoundation全体の新しい一律待機Gateとしない。先行可能な他のLinux移行・上位サービス実装は、既存承認範囲で継続できる。

## 次の判断とRoadmapの扱い

比較の実際の証拠（同一条件での起動・障害系・必要工数、実装上の制約）を見て、A案かB案の正式採用を明示的に決め、その後で恒久的な技術ADRと実装契約へ反映する。版割当て・開発開始状況・機器対応・受入結果は未決定である。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-16 | ユーザーが「A案を第一候補のまま、B案と短期間の実装比較を行ってから最終決定する」方針を選択。**比較して判断するプロセスのみCONFIRMED**。A/Bどちらの正式採用も、期間の数値・技術選定・試験結果も未確定。 |
