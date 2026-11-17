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

