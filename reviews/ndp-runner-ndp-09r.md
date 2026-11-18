# NDP-09r 自動レビュー記録

## Round 1 — codex review of cd37b931a429

**P2 — portable fixture の入力拒否が CLI の終了コードに反映されません。**  
`summary.c` を入力ルート外へのシンボリックリンクに置き換えるか、入力サイズ上限を超えるファイルにすると、Stage 2 はコンパイル前に拒否します。`report.py` はその結果を `not_run` のまま出力するため、他工程が成功または未実行なら CLI は `0` で終了します。README では選択済み工程の入力エラーを `failed` と定義しており、API・UI の入力拒否も `failed` です。最小修正は、Stage 2 の入力拒否をレポート側で `failed` に変換し、理由を保持することです。一時 fixture のシンボリックリンクで、コンパイル未実行、`failed`、CLI 終了コード `1` を検証してください。 [stage2.py:199](tools/nagi-developer-platform/validation/stage2.py:199) · [report.py:265](tools/nagi-developer-platform/validation/report.py:265) · [report.py:343](tools/nagi-developer-platform/validation/report.py:343) · [README.md:220](tools/nagi-developer-platform/validation/README.md:220)

指定の差分は担当パス内の4ファイルでした。4 fixture の実 CLI 実行、具体的な件数、入力ハッシュ、時刻を除く再現性、実際の XML 解析失敗後の継続を検証するテストを確認しました。Stage 3 と Nagi target 受入は常に `not_run` です。テスト結果は runner の報告を参照し、今回の指摘はコードから確認しました。

NDP-REVIEW: FIX

