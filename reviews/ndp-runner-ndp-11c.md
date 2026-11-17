# NDP-11c 自動レビュー記録

## Round 1 — claude review of 7f2433a3e30b

P1/P2 の指摘はありません。変更4ファイルを読み、一部は実装を直接確認しました。テストは再実行していません。

**確認できた点**
- **API rules**: 期待値の `API_CASES` は、行・rule id・検出種別・alias を fixture の evidence から手書きしたものです。`validate_api` はこれを `==` で照合します。negative control が既知 API を主張しないこと、`compatibility=not_assessed`、`target_acceptance=not_run`、`approval=PROVISIONAL` も検証しています。
- **generator**: 毎回新しい一時 checkout に生成します。5ファイルの過不足、出力先外に何も作られないこと、Manifest の内容、3回の stdout と生成 tree の一致を確認します。
- **Stage 2**: `compile` と `run_test` の両 step の成功を確認します。stage2.py は一時ディレクトリに作業し、リポジトリ内には書きません。
- **入力不変**: 追加した tool・fixture・template は `INPUTS` に入り、`finally` で必ず再検査されます。
- **failure-path**: 誤出力、欠落、timeout、nonzero、不正 JSON、途中の入力改変、phase 名の保持の各テストがあります。
- **主張の範囲**: `not_run` に `scanner_output_to_api_rules`、`generated_crate_build`、`stage2_third_party_build`、`stage3_nagi_target` などが載っています。contract-audit.md は速度比・移植成功・target 成功を主張せず、比較限界を明記しています。

**P3(軽微)**
1. `tests/developer-platform/evaluation/run_host_checks.py:325` 付近: `check_generator` は `tempfile` の checkout に書き込むため、実行中に別のローカルプロセスが一時ディレクトリを操作すると結果が乱れます。TOCTOU として、README に「host pilot の限界」と一行記載してください。
2. 同ファイル `check_stage2`: timeout が 90 秒固定です。遅い環境では timeout で FAIL になるだけで誤 PASS にはなりません。README に記載済みなので対応は不要です。

NDP-REVIEW: PASS

