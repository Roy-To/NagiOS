# NDP作業ボード

2026-11-17。専用worktreeは利用者が作成を許可。共有checkoutのbranchを切り替えず、専用作業場所で1セッションが書き込む。push/上流merge/共有registry変更は未実施。

| Task | 状態 | 結果 | 次・残件 |
|---|---|---|---|
| NDP-01 | PASS | e8071ecの開始記録 | 担当確定済み |
| NDP-02 | PARTIAL | 最小Manifestを実validatorで確認 | SDK外部参照方式とowner合意 |
| NDP-03 | NOT STARTED | なし | SDK参照契約待ち |
| NDP-04 | NOT STARTED | なし | PT-01正規入力の接続待ち |
| NDP-05 | NOT STARTED | なし | PT-01正規入力の接続待ち |
| NDP-06 | PASS | 9b68295、18テスト・再レビュー成功 | Qt XMLのhost pilot限定 |
| NDP-07 | NOT STARTED | なし | UI owner接続待ち |
| NDP-08 | PARTIAL | 正規validatorを実接続 | CLI/Package owner確認 |
| NDP-09 | PARTIAL | Stage 1実接続、11テスト成功 | Stage 2/3と生成物接続 |
| NDP-10 | PASS | 715d308、独立レビュー済み | 後続の比較評価 |
| NDP-11 | PARTIAL | C/UI各3回一致、入力不変 | 手動・LLM比較とtarget未実施 |
| NDP-12 | NOT STARTED | 上流統合なし | 統合・登録担当の指定待ち |

SDKとtoolの読取結果・具体的な不足は[接続調査](contract-audit.md)。PASSは上記の限定作業のみで、Nagi image受入ではない。

確認結果: portable Cの警告エラー付きbuild・境界条件・繰返し一致・入力不変が成功。NDP-06は実際のNDP-10入力とCLIを含む18テストが成功。標準host全体のdoctor、SDK validator、VM/targetは未実施。

Codexレビュー: 初回P2を修正し、9b68295の独立再レビューで未解決P1/P2なし。詳細は[レビュー記録](../../../reviews/ndp-host-pilots.md)。

DeepSeek: 方式Aの追加条件は非該当と独立担当が判断。重要なOS権限等の変更や未解決・見解不一致はなく、今回の追加送信は不要。実送信はしていない。

2026-11-17: `python3 tests/developer-platform/evaluation/run_host_checks.py` 成功。
Cを実build/run、UI CLIを各3回実行。UI中央値約34ms（起動込み、速度比ではない）。
ランナー失敗検知7件と既存UI 18件が成功。詳細・再実行は[評価README](../../../tests/developer-platform/evaluation/README.md)。

独立レビューのP2（同じ誤解析でも一致成功）を修正。部品・tab順・接続を照合し、旧版で5ケース失敗を確認。

NDP-11独立再レビュー: e49fb71でP2解消、未解決P1/P2なし。DeepSeek追加不要。

NDP-02/08/09: 固定nightlyでnagi-pkgをbuild、最小第三者Manifestを受理。
Stage 1 adapter 11件・正規tool 7件成功。Stage 2/3は未実行を返す。

## 自動runner記録

- 11-17 NDP-06b: GTK Builder静的解析と安全性・回帰テストを追加（PR #165、runnerテスト成功、claudeレビューPASS）
- 11-17 NDP-11b: GTK 3/4 を run_host_checks.py の繰返し評価と failure-path テスト（14 件）に（PR #167、runnerテスト成功、codexレビューPASS）
- 11-17 NDP-07: UI対応表・未対応レポートと実fixtureテストを実装（PR #168、runnerテスト成功、claudeレビューPASS）
