# NDP-09r 自動レビュー記録

## Round 1 — codex review of cd37b931a429

**P2 — portable fixture の入力拒否が CLI の終了コードに反映されません。**  
`summary.c` を入力ルート外へのシンボリックリンクに置き換えるか、入力サイズ上限を超えるファイルにすると、Stage 2 はコンパイル前に拒否します。`report.py` はその結果を `not_run` のまま出力するため、他工程が成功または未実行なら CLI は `0` で終了します。README では選択済み工程の入力エラーを `failed` と定義しており、API・UI の入力拒否も `failed` です。最小修正は、Stage 2 の入力拒否をレポート側で `failed` に変換し、理由を保持することです。一時 fixture のシンボリックリンクで、コンパイル未実行、`failed`、CLI 終了コード `1` を検証してください。 [stage2.py:199](tools/nagi-developer-platform/validation/stage2.py:199) · [report.py:265](tools/nagi-developer-platform/validation/report.py:265) · [report.py:343](tools/nagi-developer-platform/validation/report.py:343) · [README.md:220](tools/nagi-developer-platform/validation/README.md:220)

指定の差分は担当パス内の4ファイルでした。4 fixture の実 CLI 実行、具体的な件数、入力ハッシュ、時刻を除く再現性、実際の XML 解析失敗後の継続を検証するテストを確認しました。Stage 3 と Nagi target 受入は常に `not_run` です。テスト結果は runner の報告を参照し、今回の指摘はコードから確認しました。

NDP-REVIEW: FIX

## Round 2 — codex review of 62764199ce00

- **P1 — Stage 2 が、承認済みのパスを使うだけで C ソースの内容まで承認済みとして実行します。** `portable` の `summary.c` や `summary_test.c` が通常ファイルのまま、コンパイル可能な第三者コードに置き換わると、サイズとパスの検査を通過し、生成されたバイナリがホスト権限で実行されます。CI で fixture を変更したブランチを評価する場合に起き得ます。実行後の終了コードと出力プレフィックスの検査では、外部ファイルへのアクセスや通信を防げません。最小修正は、Stage 2 が読み込んだスナップショットを、評価対象ブランチから独立した承認済みダイジェストとコンパイル前に照合し、不一致なら理由付きで拒否することです。通常ファイルの内容だけを変更し、コンパイルとテストが未実行になることを検証してください。 [report.py:257](tools/nagi-developer-platform/validation/report.py:257) · [stage2.py:201](tools/nagi-developer-platform/validation/stage2.py:201) · [stage2.py:232](tools/nagi-developer-platform/validation/stage2.py:232)
- **P3 — 指定範囲の差分に担当パス外のレビュー記録が含まれます。** `BASE..HEAD` にはラウンド1のレビュー記録を追加した独立コミット `11ba7a9` が含まれます。実装コミットの変更は担当パス内です。最小修正は、runner が実装差分とレビュー記録を別々に集計することです。 [reviews/ndp-runner-ndp-09r.md:1](reviews/ndp-runner-ndp-09r.md:1)

前回指摘のシンボリックリンクとサイズ超過について、理由を保持した `failed` と CLI 終了コード `1` を検証するテストを確認しました。4 fixture の具体的な件数、入力ハッシュ、時刻を除いた再現性、解析失敗後の継続、Stage 3 の `not_run` もテストで確認しています。今回は差分とテストコードを確認し、テスト実行結果は runner の報告を参照しました。

NDP-REVIEW: FIX

