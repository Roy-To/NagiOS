# DEC-006 — Nagiの日常利用シナリオ

**状態:** CONFIRMED  
**最終合意:** 2026-10-25までのNagi_chat議論  
**対象:** 0.2基盤 / 0.3 Personal Agent / 将来の0.4+  
**関連:** DEC-001、DEC-004、DEC-005、DEC-007

## ユーザーが採用した代表シナリオ

1. **資料を探す → 要約する → 文書を作る。** Files、Search、Albert、Notes、Writerなどの共通ContextとPublic Actionを活用する。
2. **ファイルを整理する → 重複をゴミ箱に移す → 必要なら復元する。** 操作対象・権限・Byte一致・復元可能性を確認し、Activity / Waybackへ記録する。
3. **Webで調べる → 閲覧情報を保存する → 後日思い出す。** Albertの履歴・本文IndexとNagi Memoryを区別しながら連携する。
4. **作業を中断する → 後日続きを再開する。** Workspace、Task、Activity、Waybackを使い、状態と現在のRevisionを確認する。
5. **普段の手順を理解する → 次回から作業を簡略化する。** Preference / Habitを自動学習するが、実行権限は増やさない。

## バージョン別の主担当

| 世代 | 主な役割 |
|---|---|
| 0.1 | 起動・基本OS機能、Search / Workspace / Waybackの最初の基盤 |
| 0.2 | Identity、Action、App SDK、Jobs、Settings、Search、Activity / Wayback等の汎用Platform統合 |
| 0.3 | 単一Personal Agentが、許可を受けた複数アプリのActionを順次実行し、MemoryとLearningを活用 |
| 0.4+ | Nagiが専門Agentへ限定Taskを委譲する高度なシナリオ（別のActivationが必要） |

0.2の土台を整えることは0.3 Agentが完成したことを意味しない。0.4+のMulti-Agentは0.3に前倒ししない。

## 未確定事項

代表Acceptance Workflowの具体的な入出力、Taskごとの操作承認の初期値、低スペックでの性能目標、使用アプリ組合せごとの実装順。

## 決定履歴

| 日付 | 内容 |
|---|---|
| 2026-10-25 | 5つの主要日常利用シナリオを合意 |
