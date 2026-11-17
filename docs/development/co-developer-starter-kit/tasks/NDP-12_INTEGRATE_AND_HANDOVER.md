# NDP-12 — 分野内統合・レビュー・引渡し

状態: PROPOSED / NOT STARTED。共通。関係する小成果、指定Integration Owner、実際のbase/merge許可が必要。

## 目的

個別成果を既存のNagi契約に矛盾なくつなぎ、使える範囲を継続的に増やす。12件すべてが終わるまで統合を先送りしない。

## 作業

成果ごとに元branchと固定commit、変更path、利用contract、実行したテスト、残件を確認する。既存PT-01/SDK/CLIと重複した機能がないかを調べる。小さな独立PRごとに受け取り、レビュー・CI・merge境界は指定手順に従う。

共通contractを更新した場合はconsumer側の互換テストを確認する。merge conflictの解消だけで設計が整合したとしない。片方の機能や失敗テストを削除して通さない。影響部分のhost検証を行う。

NDP-03の生成物、04/05の診断結果、06/07のUI報告、09のStage結果、10/11の評価が同じ版・前提で接続されているかを確かめる。使えない工程は未実装/未接続と表示する。

## 実際の権限

Core Developerが分野内統合を担当する場合も、開始記録で認めたbranchだけに限定する。共有Registry、CI、root workspace、SDK正本などの変更は指定ownerが受け取る。main、Linux移行統合branch、正式releaseへの反映を勝手に行わない。

## 最小の引渡し

```text
成果: NDP-xx / 固定commitまたはPR
確認: 実行コマンドと結果、host-onlyかtargetか
接続: 既存contract・owner・未接続点
次: 最小の修正または次task
```

採用証明、段階ごとのJSON、長いhandoffを重ねない。別AIレビューは実テストの代替ではなく、DeepSeek方針の実施状況も別に正直に記録する。

## 完了境界

今回選択した成果が指定統合先へ実際に反映されたか、未mergeのPRかを区別して報告できる。新target上で実行していなければNagi target PASSやrelease完了にしない。未解決はownerと次の作業へ戻し、次の承認済みtaskを継続する。

共通: [規則](../03_COMMON_RULES.md)、[レビュー・再開](../06_REVIEW_INTEGRATION_RESUME.md)。
