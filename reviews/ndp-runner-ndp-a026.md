# NDP-A026 自動レビュー記録

## Round 1 — codex review of 946e534be857

- **P2** `tools/nagi-developer-platform/validation/test_report_output_limit.py:182,197` — 表に `ui-invalid` のようなハイフンを含む余分な行が追加されても、正規表現がその行を読み飛ばすため、fixture 名の集合が一致したと判定します。実際の README にこの行を追加して、検査結果が空リストになることを再現しました。最小修正は、表のヘッダーと区切り行に続くデータ行をすべて検査し、未知の名前や解析できない行をエラーにすることです。

現行の表には全9 fixtureと `all` があり、同期テストは通過しました。`ui_invalid` 行を除いたメモリ上のコピーでは `missing row: ui_invalid` を確認しました。ディスク上の一時コピーを使うテストは、読み取り専用環境で一時ディレクトリを作成できず、検査まで到達しませんでした。

実 `report.main()` の再測定では、この環境で `all` が終了コード1を返したため、README に記載された終了コード0での測定値は独立に再現できませんでした。差分は担当範囲内の README とテストの2ファイルに限られ、`report.py`、`contracts/`、既存の余裕テストの assertion、PROVISIONAL および host 限定の注記は維持されています。

NDP-REVIEW: FIX

## Round 2 — codex review of 317bff928405

**P2 1件、P1 なし。**

- **P2** `tools/nagi-developer-platform/validation/test_report_output_limit.py:213` — `all` の直後に、行頭の `|` を省略した有効な Markdown 表の行 `` `ghost` | 131072 | 1 (0.0%) | 1 (0.0%) | `` を追加しても、検査は問題なしと判定します。README のメモリ上のコピーで再現しました。最小修正は、行頭の `|` を省略したデータ行も解析し、この形式で追加した未知の行を拒否する回帰テストを加えることです。
- **P3** `tools/nagi-developer-platform/validation/test_report_output_limit.py:291` — fixture が8件に戻り、README の上限も正しく更新された場合、誤った上限を作るテストの置換前後が同じ `1048576` になり、テスト自体が失敗します。置換後の値を `all_limit + 1` から計算すれば、fixture 数に依存せず誤値を作れます。

現行表の9 fixture と `all` の名前・上限は同期テストに合格し、`ui_invalid` を除いたメモリ上のコピーでは `missing row: ui_invalid` を確認しました。`report.py`、`contracts/`、既存の余裕テストの assertion、PROVISIONAL と host 限定の注記は維持されています。担当範囲外の差分は、別コミットの前回レビュー記録 `reviews/ndp-runner-ndp-a026.md` です。

**測定値の独立確認**  
実 `report.main()` を両形式で呼び出し、全18回で stderr が空であることを確認しました。この読み取り専用環境では複数のチェックが `io_error` となり、`all` は JSON 76,821 バイト、Markdown 38,512 バイトでした。README の測定値はこの環境では再現できませんでした。ディスク上の一時コピーを使うテストと全体スイートは再実行していません。

NDP-REVIEW: FIX

