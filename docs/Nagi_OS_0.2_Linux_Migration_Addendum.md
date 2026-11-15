# Nagi OS 0.2 — Private Linux移行補足仕様

> **2026-11-15 公開移管:** [DEC-050](product-decisions/DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリへ移した。本文中の「Private」「Publicへ転載しない」などの記述は移管前のもので、DEC-050が優先する。

> この補足は、0.2実装仕様の本文([docs/Nagi_OS_0.2_Codex_Implementation_Spec.md](Nagi_OS_0.2_Codex_Implementation_Spec.md))に対するLinux移行の差分を記録する。


**更新日:** 2026-11-13  
**状態:** Scoped documentation addendum / 既存release仕様の全面置換ではない  
**適用範囲:** DEC-044により承認されたPrivate Linux移行開発と、既存0.2契約の適用関係  
**実装状態:** 本書の保存は実開始・Linux image acceptance・release activationを示さない

## Linux migration lane execution approval (2026-11-14; DEC-047)

The initial assignment snapshot in section 4 (NOT ASSIGNED) and its statement that checkout/worktree/build/VM operations are not authorized are superseded for the later user-approved Linux migration lanes. Use [DEC-047](product-decisions/DEC-047-linux-migration-workspaces-and-integration.md) for the lane worktree location, permitted in-worktree operations, integration branch and the retained listed limits. The Public 0.2 section 2.1/M00 release-runtime activation boundary remains the separate 0.2 release gate already described in this addendum.
## 1. 正本と優先関係

0.2 Platform Foundationの既存実装仕様・AcceptanceはPublic main `48b4eeb12a4a6674badd0a530149570ff4c30d30` の次の両pathを基準とする。確認時点では同一内容である。

- [docs/Nagi_OS_0.2_Codex_Implementation_Spec.md](https://github.com/Roy-To/NagiOS/blob/48b4eeb12a4a6674badd0a530149570ff4c30d30/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md)
- [docs/0.2/Nagi_OS_0.2_Codex_Implementation_Spec.md](https://github.com/Roy-To/NagiOS/blob/48b4eeb12a4a6674badd0a530149570ff4c30d30/docs/0.2/Nagi_OS_0.2_Codex_Implementation_Spec.md)

本Private補足は公開版を複製した新しい正本ではなく、後続決定による限定差分を明記する。Private [AGENTS](../AGENTS.md)、[DEC-042](product-decisions/DEC-042-linux-kernel-transition.md)、[DEC-043](product-decisions/DEC-043-nagi-userland-independent-os.md)、[DEC-044](product-decisions/DEC-044-linux-migration-development-authorization.md)、[移行ADR](decisions/2026-11-12-linux-migration-development-scope.md)を併読する。今回の詳細な照合・証拠限界は[影響表](product-decisions/KERNEL-TRANSITION-ALIGNMENT-2026-11-13.md)に記録する。

## 2. 確定した方針と限定開発

- Linux kernel + Nagi専用userlandはCONFIRMED。既存distributionのrootfs/service/package体系を製品OS基盤として採用しない。個別OSSの選択的再利用は可能。
- DEC-044のPrivate移行設計・実装・検証は承認済み。個別に許可された場所・path・担当・資源での作業に限り、旧独自kernelの完成や旧kernel固有Milestoneを一律の着手前提としない。
- 公開版§2.1、§32 M00の旧M30順序文は既存0.2 release activationの境界として保持する。限定開発への例外を、0.2全体のruntime開始許可・M30 PASS・release Gate解除へ拡張しない。
- kernel revision、init/libc/display/security backend、package形式、互換方式、正式適用版・切替時期は未確定。担当はDEC-044の技術判断委任内で、可逆的な開発構成を根拠と固定入力付きで選べる。

## 3. 再接続が必要な境界

| 公開版の範囲 | 移行時の適用 |
|---|---|
| §5 Reference Machine | 旧0.2参照構成を保持。恒久的Linux targetの決定へ読み替えない。開発targetは個別記録する |
| §6 High-Level Architecture | Nagi Loader → Nagi Kernel → nagi-initは旧基盤の起動連鎖。Linux boot → Nagi userland → Nagi serviceへ実際に接続する。既存loader/initが無変更で使えるとしない |
| §7–27 platform contracts | Identity/Session、Permission、IPC/IDL、Jobs、Settings、Resource/Activity、Model、Update、Notification、SDK等の製品契約を継承。OS依存部の再利用・adapter・再設計を記録する |
| 0.2-M21 SDK/Public App contract | Rust SDK/C ABI、manifest、package検証、out-of-tree sample等の既存Deliverable/Acceptanceを維持。Linux採用をsource/binary/toolkit互換保証とせず、SDKを0.3へ延期しない |
| §29 Ownership | 既存owner/branch/path、shared-pathとIntegration Owner調整を維持。本書は実装Workstream登録やowner変更ではない |
| §30 Status | 歴史的snapshot。実装状況は対象refのimplementation_statusとworkstream state・実コード・受入証拠で確認する |
| §31 Evidence、§35–37 integration | portable core、Linux同一本体adapter、Nagi Linux image内の統合を別証拠で記録。過去kernel/host PASSを移さず、旧成果・branch・未統合差分を保護する |

## 4. 安全・受入・未確定事項

AIは非特権・未信頼のまま。型付きPublic Action、決定論的認可、Permission/Capability、取消・失効、Object ID/Resource revision、データ所有と復旧を維持する。Linux UID/root/capabilitiesだけでNagi権限の同等性を認めない。実際の機構とThreat Modelは対象ADR・同一本体で検証する。

既存0.2 release runtimeはM30 PASS・明示的release境界・対象WorkstreamのIntegration Owner checkpointを維持する。限定移行開発の最初の統合は一つの開発targetでboot → userland → service、その次に既存一appの通常UI操作・実保存・再起動後の再読込を目指す。これはDEC-044の統合目標であり、新しいrelease番号や0.2完了宣言ではない。

実装branch/path/担当/検証環境は、DEC-044/ADRの初回保存ではNOT ASSIGNED。後続割当を確認してから対象作業を行い、全設計の完成待ちを新たな一律Gateにしない。SSD/Git制限、Files保全、拒否・結果不明、共有runner・資源条件は引き継ぐ。未許可のcheckout/clone/worktree、環境新設、install、VM/build実行は本補足で許可されない。

正式製品切替・配布/公開・利用者data移行・互換保証・既存release Gate変更は別判断。Private本文をPublicの仕様、PR、issue、CI artifactへ転載しない。

## 変更履歴

- 2026-11-13: DEC-042/043/044の後続関係を既存0.2仕様へ対応付けるPrivate補足を追加。Public本体・Workstream・release statusは変更していない。
