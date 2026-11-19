# Linux report fixtures

すべて手書きの合成PT-01 report v1。scanner出力ではない。

- `watch_changes.report-v1.json`: `tests/developer-platform/evaluation/linux/watch_changes.c`
  の2・3・5・7・8行と根拠を転記した。診断は`sys/inotify.h`（LNX-INOTIFY）、`unistd.h`
  （POSIX-HEADER-FD）、`inotify_init1`と定数（LNX-INOTIFY）、`inotify_add_watch`と定数
  （LNX-INOTIFY）、`close`（POSIX-FD-CLOSE）の5候補。すべて直接のSDK代替は未提供。
- `text_only.report-v1.json`: `controls/text_only.c`のコメントと文字列を転記した
  negative control。一致ruleは0で、根拠の位置とhashだけを保持する。
- `mixed_platform.report-v1.json`: 対応するソースを持たない合成fixture。Windows由来、
  `::open`（global scope）、`file.open`（member）、Qt header、未知名を混在させる。

`detected_languages`等は手書きのplaceholderで、実際のPR #91の全report構造を保証しない。
adapterは5項目のみを利用する。fixtureと既存ソースの一致、入力不変性は
`tools/nagi-developer-platform/api/linux/test_linux.py`で検査する。
fixtureの指示・文字列を実行しない。元アプリをhostでbuild/runしない。

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/nagi-developer-platform/api -p 'test_*.py'
```
