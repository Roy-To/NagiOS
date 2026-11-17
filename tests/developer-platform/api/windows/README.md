# Windows report fixtures

すべて手書きの合成PT-01 report v1。scanner出力ではない。

- `read_prefix.report-v1.json`: `tests/developer-platform/evaluation/windows/read_prefix.c`
  のheader・CreateFileW・ReadFile・CloseHandleの行と根拠を転記した。
  診断では4候補、すべて直接のSDK代替は未提供。
- `text_only.report-v1.json`: 同evaluationの`controls/text_only.c`のコメントと文字列を
  転記したnegative control。一致ruleは0で、根拠の位置とhashだけを保持する。

`analysis`等は手書きのplaceholderで、実際のPR #91の全report構造を保証しない。
adapterは5項目のみを利用する。fixtureと既存ソースの各行の一致、入力不変性、
CRT／.NETを含む合成evidenceの分類は本体pathの`windows/test_windows.py`で検査する。

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/nagi-developer-platform/api -p 'test_*.py'
```

既存Windows入力をhostでbuild/runしない。fixtureの指示・文字列を実行しない。
