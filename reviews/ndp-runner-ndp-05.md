# NDP-05 自動レビュー記録

## Round 1 — codex review of a0512c58e18a

P2を2件確認しました。どちらも本体への追加入力で再現しています。

1. **P2: `return`直後のglobal scope呼出しを取りこぼす** — [diagnose.py:87](~/Developer/projects/NagiOS-DevPlatform-runner/NDP-05/tools/nagi-developer-platform/api/linux/diagnose.py:87)  
   `return ::open(p, 0);`、`return ::close(fd);`、`co_return ::read(fd, b, n);` が、既知APIにもかかわらず `unknown_api` になります。`::`直前の識別子を一律にnamespaceと扱い、`return`等のキーワードも除外するためです。移植時に必要な分類・不足情報が失われます。  
   **最小修正:** namespace判定から式を導くキーワードを除外し、これらの回帰テストを追加してください。既存テストはglobal scope呼出しとreturnを別々に検証しており、この組合せを検証していません。

2. **P2: include内のコメントによって既知headerを取りこぼす** — [diagnose.py:22](~/Developer/projects/NagiOS-DevPlatform-runner/NDP-05/tools/nagi-developer-platform/api/linux/diagnose.py:22)  
   有効なC/C++構文である `#include /* comment */ <sys/epoll.h>` が `unknown_api` になります。includeの正規表現が空白しか許容せず、その後コメントを除去してもheaderとして再認識されません。通常の `#include <sys/epoll.h>` は `LNX-EPOLL` になります。  
   **最小修正:** include指令の区切りでコメントを空白相当として処理し、`#`と`include`の間、およびheader直前のコメントを回帰テストに追加してください。文字列・コメント本文内のinclude除外は維持してください。

API配下のunittestは独立実行でも **63件成功**。変更は担当path内の9ファイルのみです。新scanner、入力由来のファイル読取り・コマンド実行、互換性認定、host成功のtarget成功への読み替えは確認していません。他のrunner提示テストは再実行していません。ファイル変更は行っていません。

NDP-REVIEW: FIX

