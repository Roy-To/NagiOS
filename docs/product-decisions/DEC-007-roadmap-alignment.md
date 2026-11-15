# DEC-007 — 0.1〜0.4+ Roadmapとの整合

**状態:** REVIEW NOTE（ユーザー決定事項を変更しない）  
**作成日:** 2026-10-25  
**対象:** 0.1 / 0.2 / 0.3 / 0.4+  
**関連:** DEC-001〜006、DEC-016、DEC-018

## 参照資料

- `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`: 0.1の受入・実装判断。
- `docs/Nagi_OS_0.2_Codex_Implementation_Spec.md`: 0.2 Platform Foundation・Workstream・Activation。
- `docs/Nagi_OS_0.3_Architecture_Roadmap.md`: GitHub main上の0.3 Architecture。
- 2026-10-25にNagi_chatへ提出された `Nagi_OS_0.3_Architecture_Roadmap_Revised.md` と `Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md`: 将来像の参照。新ファイルは当PRでは本体をリポジトリへ反映しない。

## 整合する点

- 0.3 MemoryはNagi Profileに紐づき、Modelから独立して保持する。モデルはMemoryの正本ではない。
- 0.3は**単一の秘書Agent Nagi**による正規Public Action経由の順次クロスアプリ実行、Memory/Preference/Habit、音声・Text fallbackを目指す。
- 0.4+はJev / Research / Review等、範囲付き委譲・証拠付き成果・Stop伝播を備えた**Agent Team**。0.3の動作・安全原則を継承する。
- Team ProfileとTeam Agentは異なる概念。個人Memoryは無条件にTeamへ共有されない。
- Local-first、無許可の特権昇格禁止、結果不明や不可逆な副作用の誠実な提示を維持する。

## 実装仕様へ反映すべき差分

1. **自律性:** スコープ内の可逆な下位Actionと、すでに明示指示された送信の重複承認を不要とする詳細条件。
2. **Cloud:** 許可済みprovider・情報区分・目的内の自動送信、および非機密Memoryの広めのContext共有。
3. **機密分類:** 自動ラベル＋ユーザー修正、元データの保護ラベル優先、unknownの送信禁止。
4. **Memory:** Active / Decay / Archive / Delete、Protected Memory、自然忘却と明示的忘却の差、再学習抑制。日数と回数は未確定。
5. **Files / Albert:** Files削除による検索・Memoryへの反映、Albert一般ページの本文Index、プライベート閲覧と容量の除外・整理。
6. **シナリオ:** DEC-006にある5つを将来のUser Acceptance候補として具体化する。

## 矛盾・未決定の可能性

- **Cloudの“明示Opt-in”と“継続的な事前許可”の表現:** Opt-inは一度限りとは限らない。許可済みScope内では再確認しない一方、許可範囲外や疑義は停止する、という意味をImplementation Specで明文化する。
- **Agentによる学習とUser許可の境界:** Nagiが通常のMemoryを自動作成することと、OSが未許可の行動監視・全データ収集を行うことを同一視しない。収集対象の公開設定・許可・削除操作が必要。
- **記憶と監査のRetentionの違い:** Memoryを忘れてもTask Log、Activity、Wayback、元ファイル、Backupはそれぞれ独立したPolicyの対象。削除済みを再取込みしない仕組みが必要。
- **日常シナリオの実装順:** 0.2で基盤を作ることと、0.3で全シナリオが完全自律動作することを混同しない。最小のAcceptance対象を0.3 Specで選ぶ。

## Gateと優先順位

0.1のM30および0.2 activation条件を変更しない。0.3の実装開始・正式Implementation Spec化は改訂0.3 RoadmapのGateに従う。0.4+は0.3 Acceptanceと別途Activation後に検討する。Jevの自律実行Bと隔離編集の条件はDEC-016／018を参照し、本REVIEW NOTEは既存Gateを変更しない。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-10-25 | 製品決定事項と改訂ロードマップの差分をレビューとして整理 |
| 2026-11-04 | Jevの自律実行と隔離編集に関するDEC-016／018への参照を追加。REVIEW NOTEとGateは維持 |
