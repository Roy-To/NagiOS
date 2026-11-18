# NDP-A016 自動レビュー記録

## Round 1 — claude review of 54b56896bc83

P1/P2 の指摘はありません。再実行はしておらず、変更箇所のコードを読んだ範囲で判断しています。`git diff` を取った後、Bash が don't ask mode で拒否されたため、追加確認は Grep で行いました。

**目標・受入条件の充足**
- **`ui_mapping` の `axis_gaps` と合計**
  - `report.py:259-286` の `axis_gaps()` が、各ファイル結果へ軸名 → status 別の整数件数を加えています（`report.py:320-325`）。
  - 件数の合計が行数と一致するかどうかは、実装が直接検証するのではなく `summary.axis_status_counts` との一致で担保しています。`map_ui.py` 側のテストが、その summary の合計が行数と等しいことを確認しています（`ui/mapping/test_mapping.py:226-227`）。
- **backend 軸と Markdown**
  - `ui/mapping/test_mapping.py:228` で全行が `unsupported` であることが確認されています。
  - Markdown は `report.py:563-574` で、`unsupported` と `needs_decision` の件数を軸ごとに1行で出します。
- **軸定義の二重実装なし**: 軸名は `map_ui.py` の `summary.axis_status_counts` から取っており、固定リストは持ちません。
- **未知の軸・status・不一致**: 次のいずれかで `CheckError("invalid_tool_output")` になります。
  - 行の軸集合がツール自身の summary と異なる。
  - status が `MAPPING_STATUSES` にない。
  - 集計が summary と一致しない。
  - 軸名が `reason_code` の形式に合わない。
- **widget 名・文言・絶対パスの非複製**: 保持するのは軸名と整数だけです。
- **失敗時の挙動**: `axis_gaps` は `analyze` の成功パス内で作られるため、解析失敗・ツール未準備・UI 以外の fixture には出ません。version 1 の既存キーは変更されていません。
- **README の PROVISIONAL 記述**: 仮定と限界（host 静的解析のみ、実描画・実 a11y なし、Nagi target 受入ではない）が追記されています。
- **担当 path**: 変更は `validation/` 内だけで、`contracts/` と `ui/` は変更されていません。

**P3（軽微）**
- `README.md` の「Markdown は `unsupported` と `needs_decision` のみ表示し、`partial` は JSON 参照」という記載は適切です。ただし、focus・a11y・localization が "at best partial" と書かれており、これは `map_ui.py` の現在の実装に依存した記述です。`map_ui.py` が変わると古くなりうるので、「現時点の map_ui.py では」と一言添えると誤読を防げます。
- 既存テスト `test_ui_gap_rows_match_status_counts_with_only_allowed_fields` の許容キー集合に `axis_gaps` を足した変更は、加算的キー追加として妥当です。assertion は弱まっていません。

**疑問**
- 特になし。

NDP-REVIEW: PASS

