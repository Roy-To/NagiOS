# NDP-A026 自動レビュー記録

## Round 1 — codex review of 946e534be857

- **P2** `tools/nagi-developer-platform/validation/test_report_output_limit.py:182,197` — 表に `ui-invalid` のようなハイフンを含む余分な行が追加されても、正規表現がその行を読み飛ばすため、fixture 名の集合が一致したと判定します。実際の README にこの行を追加して、検査結果が空リストになることを再現しました。最小修正は、表のヘッダーと区切り行に続くデータ行をすべて検査し、未知の名前や解析できない行をエラーにすることです。

現行の表には全9 fixtureと `all` があり、同期テストは通過しました。`ui_invalid` 行を除いたメモリ上のコピーでは `missing row: ui_invalid` を確認しました。ディスク上の一時コピーを使うテストは、読み取り専用環境で一時ディレクトリを作成できず、検査まで到達しませんでした。

実 `report.main()` の再測定では、この環境で `all` が終了コード1を返したため、README に記載された終了コード0での測定値は独立に再現できませんでした。差分は担当範囲内の README とテストの2ファイルに限られ、`report.py`、`contracts/`、既存の余裕テストの assertion、PROVISIONAL および host 限定の注記は維持されています。

NDP-REVIEW: FIX

