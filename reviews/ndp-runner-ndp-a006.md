# NDP-A006 自動レビュー記録

## Round 1 — claude review of bacd95eb60d0

NDP-A006 のレビュー結果です。P1/P2 はありません。

README から旧「40 checks」の記述は消えています。README の件数が現れるのは、14 checks と明記した NDP-11b の履歴 1 箇所と、現在値（65 checks）を書いた 1 箇所だけです。`median_ms` の計測範囲と validator 差し替え競合の限界は、README と contract-audit.md にそれぞれ追記されています。runner のロジックは変更されておらず、変更は担当 path 内に収まっています。

テストは再実行していません。runner の実行結果は、README の説明とも整合しています。

**P3（軽微）**
- `README.md:193-195` — 「The suite grew in this step (...)」の文は、いつの時点の話か分かりにくく、文の途中で長い行になっています。「NDP-11c で増加。現在の件数は NDP-11d 節に記載」のように時点を明記して整えると読みやすくなります。
- `test_run_host_checks.py` の `ReadmeCountSyncTests` — 目標は「runner が返す check 数と比較」でしたが、runner には check 数を返す仕組みがありません。そのため README の件数を unittest の収集数と比べる実装になっています。この定義は README に明記されていて、runner を変更しない制約の下では妥当です。ただし、skip されるテストも件数に含まれる点は README に書かれていません。

**疑問**
- `test_run_host_checks.py` の `collected_count` は `sys.modules[__name__]` に依存します。`python test_run_host_checks.py` の直接実行では `__main__` になり、動くかどうかは未確認です。discover での実行は runner が成功を確認済みです。

NDP-REVIEW: PASS

