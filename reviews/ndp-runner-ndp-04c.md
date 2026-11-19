# NDP-04c 自動レビュー記録

## Round 1 — claude review of 0a87ccd67493

P1/P2 はありません。

NDP-04 の P3（`#include <Windows.h>` と `#include "windows.h"` の取りこぼし）は、今回の変更で直っています。回帰テストは `test_windows_header_case_and_quoted_include` です。大文字小文字違いと引用符形式は一致します。コメント内、文字列内、`windowsx.h`、`mywindows.h`、`"windows" ".h"` は一致しません。また `test_case_folding_is_limited_to_rules_that_opt_in` で、case-insensitive な rule が `WIN-HEADER` だけであることと、API 名が引き続き大文字小文字を区別することを確認しています。

NDP-05 の P2（制御文直後の global scope 呼出し）は、実装 AI が既存テスト `test_global_scope_call_after_control_condition` などで解消済みと報告しています。今回の diff にこの部分の変更はなく、自分では再確認していません。

テストは再実行しておらず、runner の 68 件成功の結果に依拠しています。これは host 上の結果で、Nagi target の受入ではありません。

担当 path 外の変更、新 scanner、互換性認定、外部 resource 取得は見当たりません。

**P3**
1. `windows/diagnose.py:88-92` と `windows/README.md`: `<sub/windows.h>` のような path 付きは `windows.h` に一致し得ます。README には限界として記載済みですが、このケースを固定するテストはありません。今のところ許容範囲で、必要なら、限界を固定するテストを足すだけで足ります。
2. `windows/diagnose.py` の `quotedinclude` は `#include "a\"b"` のような特殊な形を厳密には扱いません。ただし誤検出ではなく取りこぼし側に倒れます。

**疑問**
- NDP-05 の P3（`reviews/` 内のローカルパス）は担当 path 外です。別途の整理が必要です。

NDP-REVIEW: PASS

