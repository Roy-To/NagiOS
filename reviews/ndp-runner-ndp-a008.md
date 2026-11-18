# NDP-A008 自動レビュー記録

## Round 1 — claude review of 9c7c786da802

NDP-A008 のレビュー結果です。P1・P2 の指摘はありません。

実装は目標と受入条件を満たしていると判断しました。runner が実行した `validation` 配下の unittest 80 件は OK でした。実装 AI は自分でテストを実行できていませんが、runner 側の結果で確認できています。

- **`overview`**: `build_overview` が `total`、4 ステータスの件数、`not_passed`（name・status・reason を check の順に並べたもの）を返します。`version` と既存の `checks`・`evaluation` キーは変わっていません。終了コードに関わる箇所も変更されていません。
- **Markdown**: 冒頭に `## Overview` を出し、その下に `### Passed` と `### Unsupported or not run (reason)` の節があります。既存の表は `## Checks` の下に移っています。
- **理由コードの説明**: `REASON_DESCRIPTIONS` に載っているコードは説明付きで、載っていないコードは `` `code` `` だけで表示されます。表のコードは report.py 内で使われている reason と整合しているように見えます。
- **README**: Migration report 節に、PROVISIONAL・`validation/` 内に閉じる・契約ではない、という注記が追記されています。
- **テスト**: 4 fixture すべてで、件数の合計と、各ステータスの件数が checks の実数と一致することを確認しています。`not_passed` が passed 以外の全件と一致することも確認しており、期待値は checks から独立に組み直しています。未知 reason（`made_up_reason_code`）がコードのみで出ること、既知 reason に説明が付くことも確認しています。
- **範囲**: 変更は `validation/` 内だけで、外部 project の build script の実行などはありません。

**P3（軽微）**
- `report.py:316-322`: `build_overview` は未知のステータスがあれば `counts` にキーを増やします（`counts.get` による）。実害はなく、現状はステータスが 4 種に限られています。
- `test_report.py` の新規 2 件のうち、Markdown 側のテストは `windows` fixture 1 つしか使っていません。受入条件は「冒頭に節が出る」ことで足りているので、対応は任意です。

NDP-REVIEW: PASS

## Round 2 — claude review of 80f9900b66dc

**レビュー結果: P1/P2 なし。**

diff（`report.py`、`test_report.py`、`README.md`）を読んで確認しました。再実行はしていません。runner の結果（validation 80 件 OK）を前提にしています。

目標と受入条件は満たしています。
- **overview の中身**: `build_overview` は passed / failed / not_run / unsupported の件数、`total`、非 passed の `name` / `status` / `reason` の一覧を持ちます。件数は `CHECK_STATUSES` で 0 初期化しているので合計が `total` と一致します。一覧は checks の順序を保ちます。
- **既存キーと version**: `version` 1、既存の checks / evaluation キー、終了コードは変えていません。`overview` を追加しただけです。
- **Markdown**: 冒頭の `## Overview` に Passed と「Unsupported or not run (reason)」の節が出ます。既知の reason は説明付きで、表にない reason はコードのみです（`describe_reason`）。
- **範囲**: `contracts/` には触れていません。外部 project の build script を実行する変更や、外部 resource の取得もありません。
- **README**: Migration report 節に PROVISIONAL の注記があります。
- **テスト**: 4 fixture すべてで、件数の合計、ステータスごとの件数、非 passed 一覧の完全一致を、checks から独立に再計算して検証しています。未知 reason のテストは、reason を差し替えて「コードのみ」で表示されることを確認しており、決め打ちではありません。

**P3（軽微）**
1. `report.py` の `overview_lines`: 見出し「Unsupported or not run (reason)」の下に `failed` の check も入ります。各行に status を付けているので誤読はしにくいですが、見出しが実態より狭いです。見出しを「Not passed (reason)」にするか、failed を別の節に分けると正確になります。
2. `test_report.py`: 4 fixture すべてで「既知 reason に説明が付く」ことを確認するテストはありません。Markdown の説明表示は windows の 1 fixture と合成した reason でのみ検証しています。`REASON_DESCRIPTIONS` に載っていない reason が実運用で出ても、コードのみ表示になるだけで害はありません。気になる場合は、4 fixture の実 reason が表に載っていることを確認するテストを足すとよいです。

疑問はありません。

NDP-REVIEW: PASS

