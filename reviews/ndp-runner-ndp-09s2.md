# NDP-09s2 自動レビュー記録

## Round 1 — claude review of 7695137af132

NDP-09s2 の diff を読み、`stage2.py`、`stages.json` と `validate.py` の `_run` を確認しました。P1/P2 に当たる問題は見つかりませんでした。`README.md` と `test_stage2.py` は開いておらず、テストも再実行していません。テストについては runner の実行結果（41 件 OK、2 件 skip）と実装 AI の報告に頼っています。

**目標との対応（コードで確認できたもの）**
- **コマンド・引数の固定**: argv は `Target.compile_command` の 1 か所だけで組み立てます（`stage2.py` の `compile_command`）。flag は `ALLOWED_FLAGS` の closed set で、`-I`・`-include`・`@file`・plugin は入れられません。実行バイナリは引数なしです。
- **入力が command を決められない点**: 入力や CLI から command・flag・ファイル名を指定する経路はありません。fixture にある Makefile や build script は、対象表に載っていないので読まれません。
- **資源の制限**: 一時ディレクトリは `TemporaryDirectory` で、使用後に削除されます。環境変数は許可リストのみです。timeout は 0 より大きく 60 秒以下に制限され、出力は `_run` で上限があり、cancel にも対応しています。
- **fixture の保護**: 許可ディレクトリに symlink があれば `_real(allowed) != allowed` で拒否します。読み取りは `read_manifest` 経由で、列挙したファイルだけです。元ファイルは書き換えません。
- **拒否は実行前**: 拒否条件は `Popen` より前に判定され、compile が失敗したら test は走りません。
- **Stage 3**: `run_stage3` は何も読まず、常に `not_run` を返します。`stages.json` でも `may_pass:false` です。
- **host と target の区別**: `scope` は `host_build_test_fixture_only` で、Nagi target の成功とは区別されています。

**P3（軽微）**
1. `stage2.py:111-124` の `prior` は、呼び出し側が渡した dict の `status` を信頼します。Stage 1 を実際に走らせたかどうかは検証しません。CLI の `--stage1-manifest` 経由なら実行済みなので実害はありません。README に「`prior` は呼び出し側の自己申告」と書くことを勧めます。
2. `stage2.py:126-145` では、検査後に fixture が差し替えられる競合（TOCTOU）は防げません。これは範囲外なので、README に限界として明記することを勧めます。
3. clang は PATH の最初のものを使います。これは README の仮定に記載済みです。

**疑問（確証なし）**
- compile の stdout/stderr を返さないため、失敗時の診断性は低くなります。仮定として README に記載済みで、P2 にはしません。

NDP-REVIEW: PASS

