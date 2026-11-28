# NDP-06c 自動レビュー記録

## Round 1 — claude review of 8006fb8b8571

P1/P2 の指摘はありません。差分を読んだ範囲では、目標は満たしています。Bash が拒否されたため、テストは再実行しておらず、差分とスナップショットの該当箇所をファイル読み取りで確認しただけです。runner 側のテスト結果は analysis 45 件、mapping 26 件とも OK でした。

## P3
1. `analyze.py` 付近の `nested_in_unknown` は、入れ子の unknown 要素に `content` を持たせない内部フィールドです。報告の JSON を読む下流（例: ホスト評価）が `content` の存在を前提にしていないか、README に一言あると安全です。実装 AI の報告どおり、`facts["unknown"]` を使うホスト評価は kind=`widget` の項目を見るので現状は影響しません。
2. `contract.py:_unlisted` の `rglob` は、`user/nagi-ui/src` 配下の symlink ファイルを辿り、リポジトリ外のファイルの SHA-256 を読む可能性があります。対象は `user/nagi-ui` 配下で、読み取りのみです。脅威としては小さいので、README に「symlink は追跡しない、またはリポジトリ内に限る」旨を限界として書くか、`path.is_symlink()` を除外してください。
3. 要判断として報告された 2 点は、実装 AI の保守的な判断で妥当です。
   - 列挙範囲は `src` 配下の `.rs` のみで、`examples/` などは対象外です。
   - row `status` と `mapping.status` の二重表現は、`_row_status()` に導出を一本化して残しました。どちらかを廃止するかは UI owner の判断です。

担当 path 外の変更はなく、スキーマも変わっていません。

NDP-REVIEW: PASS

## Round 2 — claude review of 6306403ca34e

NDP-06c のP3整理は目標を満たしており、P1/P2の指摘はありません。差分を全読して確認しました。テストはrunnerの結果を採用し、再実行はしていません。

**確認できた点**
- **入れ子unknownの重複排除（`analyze.py:344-353`）**: 最上位のunknownだけが `content` を持ち、子孫は `nested_in_unknown: true` で一覧に残ります。unknownの個数と順序は変わりません。
  - 追加テストは `json.dumps(...).count("深い")==1` と depth=20 の `LEAF` 1回で、実際に重複しないことを検証しています。
  - unknown内の `object` が通常どおり解析されることも、別テストで確認しています。
- **`include` と `template`（`analyze.py:235-243`）**: `include` は既知タグになり、resourcesにのみ出て unknown には出ません。`template` は意図してunknownのままにし、属性と本文を `content` に残します。どちらもテストと、README の「仮定（PROVISIONAL）」節に記載があります。
- **一時ディレクトリ**: `dir=` 指定を除き、tempfile 既定に変更しました。
- **契約スナップショットの列挙差分（`contract.py:16-33`）**: スナップショットに `enumerate`（`user/nagi-ui/src` の `*.rs`）を追加し、未列挙の `.rs` は `unlisted` として `needs_review` に倒します。
  - 列挙規則は同梱のスナップショット由来で、入力UI値は影響しません。`user/nagi-ui` は読み取りのみです。
  - テストは、追加された `.rs` を検知すること、`.txt` を無視すること、レポートが `needs_decision` になること、Markdownが `not_in_snapshot` を表示することを確認します。
  - 変更なしのコピーが `matched` になる対照テストもあります。
  - 列挙範囲は `src` 配下の `.rs` に限られ、`examples/` や `build.rs` は対象外です。READMEが要判断として明記しています。
- **row status と `mapping.status` の二重表現**: 導出を `_row_status` の1か所に集約しました。全fixtureの全rowで導出関係をテストします。どちらを廃止するかは、READMEが後続のUI owner判断としています。

**指摘（P3）**
- P3 `tools/nagi-developer-platform/ui/analysis/README.md:158-160`: 「Verified locally」の節に古い件数（40件）が残っています。直後の注記で古い旨を断っているので実害は小さいです。次回更新時に件数を削除するか更新してください。
- P3 `contract.py:25`: `rglob` と `is_file()` はファイルのsymlinkを辿るため、`src` 内のsymlinkが repo 外のファイルを指すと、そのハッシュ読み取りが起こり得ます。READMEに「列挙はsymlinkを区別しない」と限界を記載してください。

**疑問**
- なし。

NDP-REVIEW: PASS

