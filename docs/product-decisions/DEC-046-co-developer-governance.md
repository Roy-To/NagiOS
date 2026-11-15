# DEC-046 — Nagi OS共同開発者の役割と担当領域の自律性

**状態:** CONFIRMED（全体方針と担当領域の役割）／未決定（個別担当・GitHub権限・運用設定）
**最終合意日:** 2026-11-15
**適用対象:** Nagi OSの人間による共同開発体制
**関連:** [DEC-044 — Linux移行開発の開始承認](DEC-044-linux-migration-development-authorization.md)、[製品決定の運用規則](OPERATING_RULES.md)、[共同開発者の担当案（PROVISIONAL）](../development/CO_DEVELOPER_ALLOCATION_PROPOSAL.md)、[0.3 Architecture Roadmap §16・§34](roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md)、[0.4+ Multi-Agent Organization Roadmap](roadmaps/Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)

## 目的

創設者と共同開発者が、製品全体の責任と担当領域の自律性を保ちながらNagi OSを共同開発できるようにする。

## 確定した仕様

- **創設者:** Nagi OSの製品全体の方向性と最終判断を担う。
- **共同開発者:** 割り当てられた担当領域について、設計と実装を主体的・自由に進める。担当領域内の日常的な設計・実装判断は、都度の創設者承認を前提にしない。
- 製品全体の方向性や担当領域をまたぐ判断は創設者の最終判断に従う。関連領域への影響がある設計は、統合前に関係者間で調整する。
- 自律的な設計・実装は、既存の仕様、セキュリティ境界、workstreamの所有・接続条件、レビュー・統合手順、テスト・受入条件、および該当release Gateの範囲内で行う。

## 例外・禁止・安全境界

- 本決定は、個別のworkstreamやファイルの担当、実装開始、GitHubアカウント権限、branch保護・merge権限、開発環境、clone/worktree、試験資源を新たに割り当てたり許可したりしない。
- DEC-044のLinux移行開発承認と、そこに定める個別の作業場所・担当・接続・資源・検証条件はそのまま適用する。本決定だけでLinux移行作業を新規に開始・割当しない。
- 既存のセキュリティ、コードレビュー、統合、受入、公開・配布条件と各release Gateは変更しない。共同開発者の役割を理由にこれらを迂回しない。
- Nagi OS内の0.3 Team Profileや0.4+ Multi-Agent機能を変更・有効化する決定ではない。ロードマップの順序、Activation Gate、製品機能の実装範囲も変更しない。
- 知的財産、ライセンス、収益、貢献者表記、離任時の取り扱いは本決定の対象外であり、必要な場合は別途合意する。

## 未決定事項

- 共同開発者の具体的な担当領域、workstream owner、integration owner、着手時期。PT-02〜PT-06の案は別文書にPROVISIONALとして記録しており、個別担当・path・開始許可は未確定。
- GitHubの対象repositoryと個別アカウント権限、branch／PR／review運用、利用する開発環境・検証資源。
- 必要に応じた成果物・ライセンス等の取り決め。

## Roadmapとの関連

0.3のTeam Profileは製品内のプロフィール・共有データ境界、0.4+はNagiのAgent組織機能を扱う。これらは人間の共同開発体制とは別の対象であり、本DECによってロードマップや各Gateは変更されない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-11-15 | ユーザーが確定した創設者の全体方針・最終判断と、共同開発者の担当領域における自律的な設計・実装を記録。既存権限・Gateは維持。 |
| 2026-11-15 | PT-02〜PT-06の具体的な担当案への参照を追加。割当・workstream owner・path・着手・GitHub権限はPROVISIONAL／未決定のまま。 |
