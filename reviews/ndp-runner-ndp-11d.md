# NDP-11d 自動レビュー記録

## Round 1 — claude review of 2876a78bd2f3

P1/P2 はありません。`BASE..HEAD` の差分と、変更した 4 ファイルのうち runner 本体、`contract-audit.md`、README の追記部分を読みました。`test_run_host_checks.py`（+399 行）は、実装 AI の報告とテスト件数以外は読んでおらず、テストの中身は検証していません。テストの再実行はしていません。

目標との対応は次のとおりです。
- **雛形 Manifest の実 `nagi-pkg` 検証**: 既存の Stage 1 adapter（`validate.py`）を呼ぶだけで、検証を二重実装していません。
- **validator がないとき**: プロセスを起動せず `not_run` と理由 `validator_unavailable` を出し、トップレベルの `not_run` にも残します。成功扱いになりません。
- **3 回の一致と入力不変**: stdout と生成 tree の一致を確認しています。バイナリの sha256 は fingerprint に含めており、manifest が書き換えられていないことも確認しています。
- **失敗検知**: `id` を不正値にしたコピーが拒否されることを確かめるので、何でも受理する validator は検知できます。
- **XAML 3 件**: XAML 本文から書いた期待表と照合し、handler と binding が `not_run` のままであることも確認しています。
- **範囲と二重実装**: 担当 path 内の変更だけで、`nagi-pkg` の build や変更はありません。tempdir を使うので範囲外への書込みもなく、新たな正本の二重実装もありません。
- **host と target の区別**: `contract-audit.md` は「host 結果で Nagi target の受入ではない」と明記しています。

## P3
- `tests/developer-platform/evaluation/run_host_checks.py:546`
  - **問題**: 計測値 `median_ms` は adapter 呼出（accept 側）だけの時間です。報告上の「雛形生成＋実 validator」という表現とは範囲が少しずれます。
  - **修正案**: `contract-audit.md` の表現を「adapter 経由の validator 呼出」に寄せるか、現状の注記を維持してください。
- `tests/developer-platform/evaluation/README.md:194`
  - **問題**: 旧文の「40 checks」の段落と、新しい「63 checks」の段落が併存しています。
  - **修正案**: 旧文を削除するか、更新してください。
- `tests/developer-platform/evaluation/run_host_checks.py:67`
  - **問題**: `validator.is_file()` から実行までの間に別プロセスがバイナリを差し替える競合は、`run_host_checks.py` の fingerprint 比較では検知できません。
  - **修正案**: 本 task の脅威モデル外なので、README に限界として 1 行記載してください。

## 疑問
- `nagi-pkg` の sha256 は実行前後で比較していますが、実行そのものは symlink 拒否と実行権限確認のみです。外部 resource の取得や任意コード実行にあたる経路は見当たりませんでした。

NDP-REVIEW: PASS

