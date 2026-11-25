# NDP-A021 自動レビュー記録

## Round 1 — codex review of e2f4b67ea611

- **P2 — 既存8 fixture の JSON が BASE から変わってもテストが通ります。** `all` の各レポートを同じ HEAD の `build_report()` と比較しているため、既存 fixture の理由コードやフィールドが両方の出力で同じように変わると検出できません。順序は固定リストで検証されています。最小修正は、同一環境で BASE から取得した既存8件の JSON を、`recorded_at` を除いて比較する回帰テストを追加することです。[test_report_ui_invalid.py:180](tools/nagi-developer-platform/validation/test_report_ui_invalid.py:180)
- **P2 — evaluation README とレポート fixture 件数の整合がテストされていません。** README には `ui_invalid` の行が追加されていますが、レポート用 fixture が全9件という記述がありません。既存の `ReadmeCountSyncTests` が照合するのは host-check の unittest 65件です。今後レポート用 fixture の追加が README から漏れても、このテストは通ります。最小修正は、evaluation README にレポート用 fixture の総数を明記し、その値を `len(report.FIXTURES)` と照合することです。[evaluation/README.md:17](tests/developer-platform/evaluation/README.md:17) [test_run_host_checks.py:1097](tests/developer-platform/evaluation/test_run_host_checks.py:1097)
- **P3 — fixture の行数が README と一致しません。** README 3か所は「3行」と記載していますが、`panel_layout.qml` は閉じ括弧を含めて4行です。最小修正は、記述を4行にそろえることです。[panel_layout.qml:4](tests/developer-platform/evaluation/ui_invalid/panel_layout.qml:4)

差分は担当パス内に収まっています。新規テストは実 CLI を呼び出して `invalid_xml`、終了コード 1、独立チェックの継続、Markdown の理由説明を検証しています。レビューではテストを再実行していません。提示された runner ログでは evaluation の65件が成功しており、validation スイートの実行結果は確認できませんでした。

NDP-REVIEW: FIX

