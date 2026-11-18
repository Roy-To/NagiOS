# NDP-A009 自動レビュー記録

## Round 1 — claude review of de8168fb7240

P1/P2 はありません。受入条件の6項目は、差分を読んだ限りすべて満たしています。テストは再実行せず、runner の結果（validation 85件 OK）を前提にしています。

## 確認した点
- **集計の二重実装なし**: `build_summary`（`report.py:413` 付近）は各 report の `overview.counts` と `overview.not_passed` を合算するだけです。判定は `build_overview` に任せ、reason の説明も `REASON_DESCRIPTIONS` と `describe_reason` を使っています。
- **既存出力は不変**: 単独 fixture の分岐は、上限・render・終了コードとも従来と同じ値です。all の Markdown では、各個別セクションが単独出力と一致することをテストで確認しています（時刻のみ除外）。
- **終了コード**: all では全 fixture の全 check を走査し、1 件でも `failed` があれば 1 になります。テストは `build_report` を patch して `failed` を作り、1 と 0 の両方を確認しています。
- **件数の一致**: テストが `summary.counts` を、各 `overview.counts` の合計と matrix のセル数えの両方と突き合わせています。
- **not_run の扱い**: `clang_unavailable` を patch したテストで、`not_run` が passed に数えられないことを確認しています。
- **reason の表示**: 既知の reason は説明つき、未知の reason は `null`（JSON）またはコードのみ（Markdown）で、テストがあります。
- **範囲と安全**: 変更は `validation/` 内の 3 ファイルだけで、`contracts/` には触れていません。新たな外部実行・走査・書込みもありません。
- **README**: PROVISIONAL 節に、形式、host のみであること、Nagi target 受入ではないこと、contract ではないことが書かれています。

## P3（軽微）
1. `test_report.py` に、`--fixture all` の引数不正（例: `--fixture bogus`）が終了コード 2 になるテストがありません。argparse の `choices` による既存動作なので実害はありませんが、`choices` を `(*FIXTURES, "all")` に変えた箇所のため、1 行のテストで固定できます。
2. `report.py:430` の `names = list(next(iter(reports.values()))["checks"])` は、先頭 fixture の check 名だけを列にします。fixture ごとに check 構成が変わると、`result["checks"][name]` が KeyError になります。現状は全 fixture が同じ構成で、テストで総数を確認しているため実害はありません。
3. 実装 AI の報告にある通り、`nagi-pkg` / clang が実際に無い環境での実機確認はありません。コード上は、patch したテストで `not_run` が維持されることを確認できています。

NDP-REVIEW: PASS

