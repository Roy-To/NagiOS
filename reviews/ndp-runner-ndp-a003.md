# NDP-A003 自動レビュー記録

## Round 1 — claude review of ee7d81fe41d5

P1/P2 はありません。指摘は P3 の 1 件だけです。テストは再実行せず、diff と新規テストファイルを読んで確認しました。README の diff は、Bash が拒否されて取得できませんでした。このため README の追記内容は未確認です。

**目標との照合**
- **範囲:** `stage2.py` が追加するのは `summarize_diagnostics` と、結果への `diagnostics` フィールドだけです。status、reason、終了コードの処理は変わっていません。拒否系と unavailable の結果には `diagnostics` が付きません。この点はテスト（`RejectionResultsUnchangedTests`）で固定されています。
- **伏字化:** 出力に入るのは file、line、severity、category の 4 項目だけです。
  - file は fixture 内の既知ファイル名に限った allowlist です。scratch の絶対パスや `./` は取り除かれ、外部パスや `../` を含む行は捨てられます。
  - category は `-W[a-z0-9+-]` の形式に合うものだけを通します。
  - メッセージ本文、ソースの抜粋、環境変数の値は元から出力に含まれません。
- **上限:** `items` は 16 件・1024 文字までです。入力は元々 64KiB で止まる capture なので、走査量にも上限があります。
- **テスト:**
  - 実 clang で、エラー、`-Werror` 警告、header 内の `#error`、警告のみで passed になる場合、passed で空になる場合を確認しています。
  - 漏えいテストは本文の秘密文字列、環境変数の値、絶対パスを実際にコンパイルして確認し、fixture が書き換わっていないことも見ています。
  - 上限テストは実 clang の警告 150 件と、fake の無限出力の両方があります。
  - fake compiler のテストは上限と整形だけを見る位置づけで、そう明記されています。
- **既存テストの変更:** `test_compiler_nonzero_exit_is_kept` は、キー名 `diagnostics` に文字列 "diagnostic" が含まれることへの対応です。このキーだけを除外し、同じ確認を保っています。assertion は弱まっていません。

**P3**
- `stage2.py` の `summarize_diagnostics` 内にある scratch prefix の除去は、出力行の先頭 1 回だけです。clang が絶対パスを途中に含める診断では、その行がそもそも正規表現に一致しないため、パスは出ません。ただし、その診断は件数からも落ちます。README の限界節にこの旨が書かれていれば十分です。
- 件数 `errors` と `warnings` は、clang が診断を出力した分に限られます。clang 自身のエラー上限や 64KiB の capture で頭打ちになります。実装報告によると README には記載済みです。

NDP-REVIEW: PASS

