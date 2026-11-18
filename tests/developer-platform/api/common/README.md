# PR #91 conformance fixtures

`pr91_*.report-v1.json` は、PR #91 `porting.rs` が文書化している出力規則
（category enum、evidence=`matched tokens: …`、review_status固定値、line≥1、
sort順、limitations）から**手で組み立てた**合成reportで、実scannerの出力ではない。
nagi-cliはbuild・実行していない。`bytes_read`等の数値は任意の値。

- `pr91_win32_candidates.report-v1.json`: 架空の`win_worker.c`（ファイルは存在しない）。
  行8は元がコメントでもscannerが行単位の部分一致で報告することを表す。
  scannerにはlexerがなく、evidenceは周辺ソースを含まない。
- `pr91_truncated.report-v1.json`: `analysis.truncated=true`。findingの不在は未使用を意味しない。
- `pr91_no_findings.report-v1.json`: findingsが空で`no-known-boundary-detected`。

既存の`windows/`・`linux/`のfixtureはPR #91が出力し得ない形（独自category、
`needs_review`、ソース断片のevidence）であり、adapter/lexerの入力処理試験用として残す。
適合は`common/test_conformance.py`が、これらがstrict検査で不適合になることも含めて検査する。
