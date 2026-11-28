# NDP-A011 自動レビュー記録

## Round 1 — claude review of b8b5aef37a90

Markdown の列挙がノイズで埋まり、目標である「どの要素がなぜ未対応か」を満たせないため、P2 の指摘が 1 件あります。テストは再実行していません。

**P2: `report.py` の `gap_row` 呼び出し付近（`inspect_ui`、`gaps = [...]` の行）と `gap_row_lines`**
- **現象:** 絞り込みに行の `status` を使っています。`map_ui.py:154-162` の `_row_status` は、`mapping.status` が `supported` でも `partial` を返します。そのため `status != "supported"` では何も除外されません。実装 AI の報告どおり、63 行すべてが載ります。
- **起きる条件:** どの ui fixture でも起きます。mapping が supported の行も上限 32 行（JSON）と 8 行（Markdown）を占有します。しかも元の行順で切り詰めるため、後方にある `unsupported` / `needs_decision` が truncated で落ちたり、Markdown の「more omitted」に隠れたりします。
- **影響:** 「どの widget・要素がなぜ未対応か」を示す目的が実質的に果たせません。実装 AI は制限として報告していますが、目標の目的に反しています。
- **最小修正案:**
  - `row["mapping"]["status"] != "supported"` の行だけを載せます。
  - もしくは `unsupported`、`needs_decision`、`partial` の順に安定ソートしてから切り詰めます。
  - `mapping.status` も各エントリに足すと、`partial` の意味が読み手に分かります。これは追加キーなので、`status_counts` との整合テストは「行 status ベースの件数 ≥ 載った行数、または truncated」の形に調整してください。
  - あわせて、supported な mapping 行が除外され、`unsupported` が上限超過時にも残るテストを追加してください。

**P3（軽微）**
- `gap_row_lines` の省略件数は `counts`（行 status ベース）から計算しています。上記を直すときは、絞り込み後の総数を `gap_rows_total` のような形で持たせると正確です。
- 不正な値が 1 行でもあるとファイル全体が `invalid_tool_output` になります。厳格で安全側ですが、README に書いてあるので許容範囲です。

NDP-REVIEW: FIX

## Round 2 — claude review of 40da72a50d5e

P1・P2 の指摘はありません。実装は目標を満たしていると判断しました。再実行はしておらず、runner の結果（95 件 OK）と、diff・コードの読解で確認しています。

**確認できた点**
- **map_ui の実出力との一致**: 行の `id` は `kind:index` 形式で、`source_kind` と共に `reason_code` の正規表現を通ります。実装は推測ではなく、`map_ui.py:212,238` の実出力に合わせてあります。
- **絞り込みの基準**: `row["mapping"]["status"] != "supported"` で絞っています。行の `status` は `partial` より良くならないため、そちらで絞ると全行が載ってしまいます。ここは正しく直っています。
- **上限と整合**: `MAX_UI_DETAILS` の 32 件で切り、`gap_rows_total` と `gap_rows_truncated` を出します。既存キーは変えていません。
- **安全性**: 載せるのは `id`・`source_kind`・`status`・`mapping_status`・`reason` のコードだけです。widget 名、本文、絶対パスは含みません。想定外の値は `invalid_tool_output` で拒否します。
- **テスト**: ui fixture の実出力（`analyze.py` と `map_ui.py` の直接実行）と照合しています。truncated、Markdown の省略件数、ツール不在や非 ui fixture での不在、不正値の拒否も検証しています。
- **README**: PROVISIONAL として形式と限界が書かれています。`contracts/` は変更されていません。

**P3（軽微）**
- `report.py:454` の `item.get("gap_rows_total", len(rows))` は、`gap_rows_total` がない場合の代替値です。ツール出力からは常にあるので実害はありません。
- README にある「`partial` rows only when their vocabulary match is not `supported`」は、README の別箇所の「`partial` は `status_counts` 以下」というテストの扱いと並べると分かりにくいです。言い回しの整理を勧めます。

NDP-REVIEW: PASS

