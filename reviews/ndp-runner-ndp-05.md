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

## Round 2 — codex review of 6e2b912dbba5

確認済みの指摘は **P2×1、P3×1** です。

1. **P2 — 制御文直後のglobal scope呼出しを取りこぼす**  
   `tools/nagi-developer-platform/api/linux/diagnose.py:95–99`  
   `if (ready) ::close(fd);`、`while (ready) ::read(fd, b, n);`、`for (;;) ::close(fd);` が、すべて `rule_id=null / unknown_api` になることを本体で再現しました。直前の `)` を一律に修飾名の一部と判断するため、既知APIの分類・不足情報が失われます。  
   **最小修正:** 対応する括弧と制御キーワードを確認し、制御条件末尾の `)` と `decltype(obj)::name` 等を区別してください。上記の回帰テストと、修飾名を引き続き除外するテストを追加してください。

2. **P3 — レビュー記録にユーザー名入りローカルパスが含まれる**  
   `reviews/ndp-runner-ndp-05.md:7`、`:12`  
   BASE..HEADに追加された記録のリンク先が、ユーザー名入りのローカルパスです。共通規則の記載禁止事項に抵触します。  
   **最小修正:** repository相対pathと行番号に置き換えてください。

独立実行したAPIテストは **65件成功**。前回指摘された `return ::name` とinclude内コメントの修正は確認しました。commonの変更、新scanner、入力由来のファイル読取り・コマンド実行、互換性認定は確認していません。担当path外の差分は上記レビュー記録です。

他のrunner提示テストとNagi target試験は再実行していません。ファイル変更は行っていません。

NDP-REVIEW: FIX

## Round 3 — codex review of 75bc9aab0ffd

HEAD `75bc9aab0ffd` を確認しました。**P1/P2はありません。**

common adapterの再利用、データ駆動rules、手書きfixture、誤検出・未知API・決定性のテストを確認しました。前回P2の修正は回帰テストで検証されています。新scanner、入力由来のファイル読取り・コマンド実行、互換性認定、host成功のtarget成功への読み替えは確認していません。

- **P3 — レビュー記録のパス説明が不正確**
  `reviews/ndp-runner-ndp-05.md:28–30`
  「ユーザー名入り」とありますが、実際のリンク（7・11行）は `~/Developer/...` で、ユーザー名は含みません。ただし他環境では参照できないローカル依存リンクです。
  **最小修正:** 指摘文を訂正し、リンクをrepository相対pathと行番号に置き換えてください。担当path外なのでrunner側で対応できます。

独立実行したAPIテストは **66件すべて成功**。他のrunner提示テストとNagi target試験は再実行していません。ファイル変更は行っていません。

NDP-REVIEW: PASS

