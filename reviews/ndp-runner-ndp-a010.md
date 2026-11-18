# NDP-A010 自動レビュー記録

## Round 1 — claude review of 2ed6b9190773

P1/P2 の指摘はありません。テストは実際の `build_report` 出力を検証していて、受入条件の 5 項目を満たしています。差分は `validation/` 内の 3 ファイルだけで、範囲外の変更や二重実装はありません。テストは再実行していません。runner 実行済みの結果（validation 89 件 OK）を前提にしています。

**確認できたこと**
- **網羅テスト:** `test_report.py:332` は 4 fixture すべてに対し、通常、validator 欠如、clang 欠如の 3 通りで実 `build_report` を実行します。check、files、steps から reason を集め、`undescribed_reasons` で表と許容リストに無いものを検出します。
- **`not_run` の担保:** `clang_unavailable`、`validator_unavailable`、`package_and_signature_checks_not_connected` が実際に出ていることも `assertIn` で確認しています。そのため、clang や nagi-pkg が無い環境でも通ります。
- **失敗検出:** 未登録の reason を check、file、step に仕込むと、3 件とも列挙されて失敗します。許容リストに入れた分は除外されます。
- **problems 行:** 既知の reason には説明が付き、未知の reason はコードのみのままです。passed の項目は出ません。
- **表に無い reason の扱い:** 既存の「未知コードは推測せずコードのみ」のテストは維持されています。
- **追加した説明:** `compiled` と `test_passed` の説明は `stage2.py:238` と `stage2.py:252` の実際の意味と一致していて、推測はありません。
- **README:** PROVISIONAL 節に方針が追記されています。

**P3（軽微）**
- `report.py` の `compiled` の説明は "portable fixture" と書いていますが、`stage2.py` の compile step は target 汎用です。現状 portable でしか出ないので実害はありません。将来 target が増えるなら文言を一般化してください。
- `collect_reasons` と `undescribed_reasons` は `reasons` が `["a:b"]` のような動的な接尾辞付きコードを持つ場合、そのコードを別物として扱います。README に `invalid_template_input:app_id` はコードのみのままと記載済みなので、対応不要です。

NDP-REVIEW: PASS

