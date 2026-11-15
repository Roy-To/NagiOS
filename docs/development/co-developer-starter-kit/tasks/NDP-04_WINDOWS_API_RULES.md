# NDP-04 — Windows由来APIの静的差分rules

状態: PROPOSED / NOT STARTED。PT-03。NDP-01と既存PT-01の正規入力/report interface、担当path・host条件の確認が必要。

## 目的

Windows由来APIの利用候補を根拠付きで分類し、現行Nagi SDKへの移植で何が不足するかを明示する。新しい全体scannerやWindows互換レイヤーを作らない。

## 編集範囲案

`tools/nagi-developer-platform/api/windows/**`とWindows用の独立テスト。共通contractは調整担当が1つに管理し、NDP-05が別schemaを正本にしない。

## 入力・出力

入力は合意済みPT-01 report、またはownerが認めた同契約の限定fixture。ソース位置などを解釈する場合も指定範囲だけを読む。reportのfieldや公開CLIを勝手に変更しない。

出力は元API/分類、相対的な根拠位置、現行SDKで確認できた対応候補、未提供/不明/要調査を区別する。文字列一致だけなら「候補」であり、実際の呼出しを証明したと書かない。

## 最初の実装単位

ファイルI/Oなど1分類の少数の代表例から始める。Win32/.NET等の着手範囲は評価候補であり、包括対応の約束ではない。実在するSDKに対応がなければ「不足」を返すことを正しい結果とする。LLMは必須にしない。

## focused test案

既知候補、未知API、同名識別子、コメント・文字列だけの出現、繰返し、壊れたreport、過大入力、非ASCII/改行差、決定的順序、指定外pathの拒否を試す。入力不変と秘密・不要な絶対パスを出力しないことを確認する。fixture由来の命令を実行しない。

## 完了境界

本体rulesを使ったhostテストと、正規入力を使った小さな接続例。PT-01が未利用ならMockの契約検査までと明示し、本物への接続済みとしない。コード変換、自動適用、アプリ実行は本taskに含めない。

## 接続待ちのとき

現在の公開例と根拠の読み取り調査は進めるが、独自reportやscannerを作って既成事実化しない。調整担当へ接続上の一点を返し、別の割当済み独立taskへ進む。

共通: [規則](../03_COMMON_RULES.md)、[キュー](../05_BACKLOG_AND_PARALLEL.md)。
