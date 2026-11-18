# NDP-A001 自動レビュー記録

## Round 2 — claude review of d93fa5fd3ce4

P1/P2 はありません。`BASE..HEAD` の diff で変更された 4 ファイル（`diagnose.py`、`rules.json`、`test_windows.py`、`README.md`）を読み、テストは再実行していません。

**目標との整合**
- `<sub/windows.h>` などの path 付き include は `WIN-HEADER` の `lexical_candidate` にならず、README に PROVISIONAL として記載されています。
- `diagnose.py` は `/` と `\` を区切りトークンとして残し、`windows.h` の前後が区切りに隣接する場合を不一致にします。
- この制限は `rules.json` の `basename_only` で `WIN-HEADER` だけに適用され、他 rule の alias 照合は変わりません。
- `api/common` の共有ロジックには触れておらず、二重実装もありません。担当 path 外の変更もありません。
- README の「未確定のまま」の記述は、確定した挙動に合わせて書き換えられています。

**テストの妥当性**
- `test_windows_header_match_is_limited_to_bare_basename` は、`<sub/windows.h>`、`\` 区切り、`"../windows.h"`、`</windows.h>` などの各ケースの期待結果を明示的に検証しています。
- `windowsx.h` と `mywindows.h` が別名のままであること、同じ断片内の素の `windows.h` が 1 件だけ候補になることも検証しています。
- 区切り文字だけの入力が `no_code_evidence` になることも検証しています。
- `basename_only` を `False` にすると `<sub/windows.h>` が `WIN-HEADER` に一致するため、実装が実際に効いていることを確認できます。
- 既存の大文字小文字・引用符 include のテストは回帰していません。

**P3（軽微）**
1. `README.md:80-86`: 判定は偽陰性側の仮定です（README に記載済み）。include search path 上の `sub/windows.h` が実際は SDK の Windows header の場合は検出できません。追加対応は不要です。
2. `README.md:73-74`: `#include /* c */ "windows.h"` のような comment を挟む引用符形式は未確定のままです。これは本 task の範囲外で、README にも明記されています。
3. `test_windows.py:15-22`: `sys.path` を書き換える方式です。runner 起点の discover で `common` を import するために必要で、担当 path 内に収まっています。

**疑問**
なし。

host Python 3.9.6 での結果であることは README に明記されており、Nagi target での受入とは混同されていません。

NDP-REVIEW: PASS

