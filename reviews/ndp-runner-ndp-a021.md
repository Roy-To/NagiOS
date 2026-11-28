# NDP-A021 自動レビュー記録

## Round 1 — codex review of e2f4b67ea611

- **P2 — 既存8 fixture の JSON が BASE から変わってもテストが通ります。** `all` の各レポートを同じ HEAD の `build_report()` と比較しているため、既存 fixture の理由コードやフィールドが両方の出力で同じように変わると検出できません。順序は固定リストで検証されています。最小修正は、同一環境で BASE から取得した既存8件の JSON を、`recorded_at` を除いて比較する回帰テストを追加することです。[test_report_ui_invalid.py:180](tools/nagi-developer-platform/validation/test_report_ui_invalid.py:180)
- **P2 — evaluation README とレポート fixture 件数の整合がテストされていません。** README には `ui_invalid` の行が追加されていますが、レポート用 fixture が全9件という記述がありません。既存の `ReadmeCountSyncTests` が照合するのは host-check の unittest 65件です。今後レポート用 fixture の追加が README から漏れても、このテストは通ります。最小修正は、evaluation README にレポート用 fixture の総数を明記し、その値を `len(report.FIXTURES)` と照合することです。[evaluation/README.md:17](tests/developer-platform/evaluation/README.md:17) [test_run_host_checks.py:1097](tests/developer-platform/evaluation/test_run_host_checks.py:1097)
- **P3 — fixture の行数が README と一致しません。** README 3か所は「3行」と記載していますが、`panel_layout.qml` は閉じ括弧を含めて4行です。最小修正は、記述を4行にそろえることです。[panel_layout.qml:4](tests/developer-platform/evaluation/ui_invalid/panel_layout.qml:4)

差分は担当パス内に収まっています。新規テストは実 CLI を呼び出して `invalid_xml`、終了コード 1、独立チェックの継続、Markdown の理由説明を検証しています。レビューではテストを再実行していません。提示された runner ログでは evaluation の65件が成功しており、validation スイートの実行結果は確認できませんでした。

NDP-REVIEW: FIX

## Round 2 — codex review of d59b74bbaf9d

**P2 — 出力上限が fixture 件数に追従することを回帰テストで確認できません。** テストは9件に固定して `MAX_REPORT_BYTES` だけを変えるため、実装を `MAX_REPORT_BYTES * 9` に置き換えても通ります。その状態で fixture を追加すると、`--fixture all` の出力上限が増えません。最小修正は、同じレポート出力に対して `FIXTURES` の件数を変え、同じバイト上限の設定で出力の許可と拒否が切り替わることを確認することです。[test_report_ui_invalid.py:199](tools/nagi-developer-platform/validation/test_report_ui_invalid.py:199)

前回の3件は差分上で解消を確認しました。新規テストは実 CLI で `invalid_xml`、終了コード1、マッピングのスキップ、独立チェックの継続、Markdown の理由説明を検証しています。担当パス外の変更は前回レビュー記録を追加した別コミットの1ファイルです。テストは再実行しておらず、提示された成功結果は host 検証です。

NDP-REVIEW: FIX

## Round 3 — codex review of 8450d0093212

**P2 — `--fixture all` で失敗が0件の場合の終了コードを検証しなくなっています。** 既存テストは、注入した失敗を取り除くと終了コードが0に戻ることを確認していました。変更後は `ui_invalid` が残るため、注入の有無にかかわらず期待値が1です。このため、`all` が常に1を返す回帰を検出できません。最小修正は、既存8件を `FIXTURES` に一時設定するケースを追加し、実際の `main()` が失敗件数0と終了コード0を返すことを確認することです。9件での終了コード1の検証は維持できます。[test_report.py:287](tools/nagi-developer-platform/validation/test_report.py:287)

前回の出力上限の指摘は解消されています。追加テストは、同じ出力と1件あたりの上限を使い、fixture 件数だけで許可と拒否が切り替わることを確認しています。実CLIによる `invalid_xml`、マッピングの `not_run`、独立チェックの継続、Markdownの理由説明、既存8件のJSONと順序、READMEの件数照合も差分で確認しました。

担当パス外の変更は、過去2ラウンドのレビュー記録を追加した別コミットのみです。レビューではテストを再実行していません。提示された成功結果はホスト検証です。

NDP-REVIEW: FIX

