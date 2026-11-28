# NDP-A017 自動レビュー記録

## Round 1 — codex review of 0f7dc051641c

指定の BASE と HEAD を比較しました。変更は担当の3パス内に収まり、混在 fixture は既存の診断・描画経路を再利用しています。`contracts/` に差分はありません。

- **P2 — 未解決行の検証に架空の API を使っています。** `tests/developer-platform/evaluation/mixed/read_and_watch.c:9`、`:19`。現行の fixture では `NtFancyFlush` と `fancy_notify_create` が未解決行になります。この入力では、実在する Windows／Linux API が診断ルール未収録の場合に未解決として表示されることを検証できません。各プラットフォームで実在し、現行ルールに未収録の API に置き換え、snapshot とソース照合テストを同期してください。
- **P2 — `--fixture all` の `reasons` に mixed の理由が含まれることをテストが保証していません。** `tools/nagi-developer-platform/validation/test_report_mixed.py:125`。現行実装は全 fixture の理由を集約しますが、`reasons` が空になったり、mixed の理由が集約から漏れたりしても、現在のループは通ります。`summary["reasons"]` のコード集合が、mixed を含む全 `reports` の `overview.not_passed` から求めた集合と一致することを検証してください。

`git diff --check` は通りました。独立実行した mixed の Markdown CLI は、読み取り専用環境で既存の診断経路が一時ファイルを確保できず、両診断が `io_error` になりました。この環境では正常出力を実行確認できていません。提示された runner ログには validation の新規テストの実行結果がなく、Nagi target は `not_run` です。

NDP-REVIEW: FIX

## Round 2 — codex review of 30827802cc73

P1・P2 の指摘はありません。`mixed` は既存の診断経路を使い、Windows と Linux の未対応・未解決行を JSON と Markdown に行番号付きで出します。テストは CLI の実出力、ソースと snapshot の照合、`all` の matrix と reasons の集約を検証しています。既存4 fixture の個別レポート経路に出力を変える差分はなく、`contracts/` にも差分はありません。

- **P3** [evaluation/README.md](tests/developer-platform/evaluation/README.md:14) — fixture 一覧を読むと、実装から置き換え済みの `NtFancyFlush` と `fancy_notify_create` が現在も使われているように見えます。一覧の API 名を `CancelIoEx` と `euidaccess` に更新してください。
- **P3** [ndp-runner-ndp-a017.md](reviews/ndp-runner-ndp-a017.md:13) — BASE–HEAD 全体には担当パス外の前回レビュー記録が含まれ、全体に対する `git diff --check` は末尾の空行を検出します。担当3パスに限ったチェックは通りました。runner 側で記録の末尾の空行を削除し、担当パスの確認ではレビュー記録コミットを区別してください。

テストは提示された runner 結果を参照し、再実行していません。Nagi target の受入は `not_run` です。

NDP-REVIEW: PASS

