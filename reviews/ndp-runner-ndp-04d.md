# NDP-04d 自動レビュー記録

## Round 1 — claude review of b5a05557cab0

PR #91 の `porting.rs` と差分の照合が終わりました。結論は PASS です。

**確認できたこと**
- `pr91_schema.json` は scanner の出力規則と一致していました。確認した項目は次のとおりです。
  - 上位フィールドと `analysis` のフィールド。
  - category 7値、review_status、conclusion の2値、`detected_languages` の3値。
  - evidence の接頭辞 `matched tokens: ` と区切り `, `。
  - `required_limitations` の3文。
  - 各 category の token 一覧。
- scanner の挙動は `pt01_conformance.py` に反映されていました。
  - `findings` は (path, line, category, evidence) の昇順で重複なし。
  - `conclusion` は `truncated || findings || dependencies` から決まる。
  - line は `index + 1` なので 1 以上の整数で、null や 0 は出ない。
- `MAX_FINDINGS` 10,000 は scanner の上限と一致します。これで scanner が正当に出す report が拒否されなくなります。
- 偏差コードは固定文字列で、report の内容を含みません。
- PR #91 のコードは repo にコピーされておらず、schema の事実だけがデータとして記録されています。
- 変更は担当 path 内に収まっています。
- 実 scanner は build・実行されておらず、fixture は手組みと報告されています。この点は README にも明記されています。

**P3（軽微）**
1. `pt01_report.py:109-141` と `pt01_report.py:143-146`: `strict` は既定で `False` です。通常経路は lenient のままで、PR #91 への適合は呼び出し側が `strict=True` を指定したときだけ強制されます。README の仮定節に、既定が lenient であることと、strict への切替条件を明記してください。
2. `pt01_report.py:123-125`: scanner が出し得る `:` `\` `%` や先頭 `~` を含む path が1件でもあると、`relative_path` がそれを拒否し、report 全体が失敗します。fail closed なので安全ですが、実 report では全件拒否になり得ます。README の限界欄に既に記載されているので、その確認だけで足ります。
3. scanner は大文字小文字を区別せずに部分一致を見ます（`to_ascii_lowercase`）。`pt01_conformance.py` の token 検査は evidence 中の元の表記を見ます。scanner は `pattern` 側（schema の token）の表記を evidence に入れるため、今の検査で問題はありません。ただし、この前提が将来変わると壊れます。README に一行補足してください。

疑問が1点あります。実 scanner の出力で適合テストを通した確認は未実施です。fixture が手組みなので、PR #91 の scanner が使える状態になったら、実出力で一度 `check_report` を通す回帰確認をお勧めします。

NDP-REVIEW: PASS

