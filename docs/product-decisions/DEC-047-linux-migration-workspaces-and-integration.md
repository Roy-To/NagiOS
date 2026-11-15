# DEC-047 — Linux移行レーンの作業場所・操作許可・統合先

> **2026-11-15 公開移管:** [DEC-050](DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態:** CONFIRMED（lane別worktree、worktree内操作許可、Linux統合先）  
**合意日:** 2026-11-14  
**対象:** Linux移行の設計・実装・検証レーン。DEC-042/043のLinux kernel + Nagi専用userland方針に基づく。  
**関連:** [DEC-042](DEC-042-linux-kernel-transition.md)、[DEC-043](DEC-043-nagi-userland-independent-os.md)、[DEC-044](DEC-044-linux-migration-development-authorization.md)、[移行開発ADR](../decisions/2026-11-12-linux-migration-development-scope.md)

## 決定の根拠と優先関係

2026-11-14のユーザー明示決定を記録する。DEC-044と移行開発ADRの初回記録にある「NOT ASSIGNED」「worktree/Git/build/VMを解放しない」という記述を、本DECの範囲で置き換える。DEC-044/ADRの他の既存内容、DEC-042/043の製品方針、既存安全・認可・データ保護・品質条件は保持する。

## 確定した作業場所と操作許可

1. Nagi実装repository `/Volumes/SSD/NagiOS-Development-20261012/Developer/NagiOS` からlaneごとに新しいworktreeを作成し、公式 `/Volumes/SSD/NagiOS-Development-20261012/nagi-ssd.py new-worktree` を使う。
2. 割り当てられたworktree内では、本人の都度承認なしに次を行える: branch作成、commit、worktree作成、Linux移行統合branchへのmerge、build/test、QEMU/VM起動、kernel source/toolchain/library等の依存取得・install。
3. Linux移行統合branchは **`codex/linux-integration-20261114`** とする。各laneはそこへ統合する。board.mdで確認した基準候補は `codex/integration-next-phase` @ `b5b33491d2d3704ab0f1f765a6b200cfa272518c`。この候補の現行状態は、作業開始時のowner handoffで固定する。

## 継続する保全・禁止事項

この決定で維持する列挙事項:

- Files保全対象の変更をしない。
- 未承認のSSD走査をしない。
- 禁止されたGit操作をしない。
- 拒否・結果不明の操作を迂回しない。
- 強制push・履歴書換えをしない。
- 他laneのworktree/branchを削除しない。
- 保全対象を変更しない。
- 非公開内容をPublic repositoryへ出さない。
- 課金を伴う資源を使わない。

具体的なFiles保全対象、SSD/Git禁止操作、拒否・結果不明対象は別担当が更新する 司令塔Dotの作業フォルダ(`nagi-orchestration`)にある `protected.md` にパスと操作で記録し、`needs-user.md` に載せて本人確認を受ける。一覧にない操作は、割り当てられたworktree内で確認待ちにせず実行する。

## Linux milestoneとの関係

旧M0→M30の全完了はLinux開発開始条件ではない。旧milestoneの状態・受入履歴・M30をPASSにする基準は保持し、Linux targetの新しい証拠へ置き換えない。0.1/0.2その他既存release gateはそれぞれのrelease activationに引き続き適用する。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-14 | ユーザーがNagi SSD repositoryのlane別worktreeとworktree内Git/build/test/VM/依存操作、統合先branch名、継続禁止を明示承認。DEC-044/移行ADRの初回「NOT ASSIGNED / 操作未開放」記述を本範囲で置換。 |

## 後続の履歴・未共有commitの取扱い（2026-11-14）

2026-11-14の本人の後続決定により、本人が作成または割り当てた `/Volumes/SSD/NagiOS-Development-20261012/worktrees` 配下のworktree内では、laneは都度確認なしに編集、commit（自己の未共有commitのamendを含む）、build、test、QEMU実行、依存取得を行える。force-pushは禁止し、共有branch historyの書換えも禁止する。既存の「履歴書換えをしない」は共有済み履歴に適用し、自己の未共有commitのamendは許可する。その他のDEC-047条項は変更しない。
