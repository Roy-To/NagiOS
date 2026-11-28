# NDP-A012 自動レビュー記録

## Round 1 — claude review of 21f179e41bb7

P1/P2 の指摘はありません。windows と linux の実出力から、行番号と API 名つきで unsupported・未解決の行が出ることをコードとテストの両方で確認しました。ファイル参照だけで確認しており、テストの再実行はしていません。Bash が使えず、runner の結果（100 件 OK）に頼っています。

**受入条件の確認**
- **windows / linux の Markdown**: `api_gap_lines`（`report.py:464`）は JSON の `rows` だけを読み、`availability == "unsupported"` または `rule_id is None` の行を `check line N · APIs · note` の形で出します。`test_report.py:540` が両 fixture の実出力で行番号・API 名・rule ID を固定値と照合しています。
- **JSON と既存テスト**: `markdown_report` に 1 行差し込んだだけで、JSON は変わりません。`test_report.py:562` が整形前後の JSON の同一性を確認しています。既存テストは変更されていません。
- **truncated 時の省略表示**: `MAX_API_MARKDOWN_NAMES` と `MAX_API_NAME_CHARS` で 1 行の長さが有限になります。省略メッセージの N は実際の行数です。`test_report.py:587` は `MAX_API_DETAILS` を 2 に差し替えた実出力で、省略行が出ることを確認しています。
- **空の節**: 対象行がなく truncated でもなければ節を出しません。`test_report.py:562` が portable と ui を確認しています。
- **`--fixture all`**: windows と linux の部分にだけ節が出て、全体の上限内に収まることを `test_report.py:598` が確認しています。
- **README の PROVISIONAL 追記**: 追記されています。

**指摘**
- **P3 `report.py:487`**: 目標の「理由説明付き」に対し、表示は rule ID（例: `unsupported (rule WIN-FILE-OPEN)`）と `no rule matched (unresolved)` だけで、説明文はありません。JSON の rows に説明がなく、JSON 構造を変えない制約があるため、実装の判断は妥当です。README にもその旨が書かれているので、そのままで構いません。
- **P3 `report.py:477-488`**: API 名に改行やバッククォートが含まれると、Markdown の 1 行構造や強調が崩れる余地があります。入力は解析済みのソーストークンなので現実の発生確率は低く、必要なら改行の除去かエスケープを足す程度で足ります。
- **疑問**: `markdown_report` の出力に対する 128 KiB の強制は、このコードでは確認できませんでした。テストの上限チェックは `test_report.py:135` と `test_report.py:608` にあります。新しい節は最大でも 2 check × 32 行 × 約 600 文字なので、上限を超える見込みはありません。

NDP-REVIEW: PASS

