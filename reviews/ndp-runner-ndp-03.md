# NDP-03 自動レビュー記録

## Round 1 — codex review of c90c8aa6c699

- **P1 — cleanupが第三者の差し替えファイルを削除する**
  - **箇所:** `tools/nagi-developer-platform/generator/generate.py:179–188`
  - **条件:** generatorが作成した`Cargo.toml`などを別プロセスが削除・差し替え、その後の書込みが失敗すると発生します。作成記録は名前だけなので、cleanupは差し替え後の第三者ファイルも`unlink()`します。「この実行が作成したものだけ削除する」という契約に反し、ユーザーデータを失います。directoryの`rmdir()`も同様です。
  - **確認:** 実際の`_cleanup()`をメモリ上の名前空間に対して実行し、差し替えファイルが削除対象になることを確認しました。既存の競合テストは別名ファイルの追加や未作成ファイルの衝突だけで、この条件を検証していません。
  - **最小修正案:** 第三者の変更が入り得る場合は自動削除せず、`left_behind`として残す保守的なcleanupに変更してください。自動削除を維持するなら、作成物の同一性と確認後の競合まで保護する仕組みが必要です。差し替え後に失敗を注入し、第三者ファイルが残る回帰テストも追加してください。

変更は担当path内の3ファイルのみです。既存previewの再利用、実validator未準備時の明示的skip、host結果とtarget受入の区別は確認できました。runnerのテスト一式は再実行していません。

NDP-REVIEW: FIX

## Round 2 — codex review of 61669d060ea8

- **P1 — inode再利用で第三者ファイルを削除し得る**  
  **箇所:** `tools/nagi-developer-platform/generator/generate.py:349`、`:224–227`  
  **条件:** 作成したファイルは書込み後に閉じられます。そのファイルが削除され、第三者の新規ファイルに同じinodeが再利用されると、記録した `(dev, inode, type)` が一致します。後続処理が失敗した際、隔離後の照合も通り、第三者ファイルを削除します。隔離への移動ではinode再利用を識別できません。  
  **最小修正案:** 作成ファイルのfdを成功確定またはcleanup完了まで保持し、inodeの再利用を防いでください。既存の差し替えテストは、置換後のinodeが異なる場合しか検証していません。

- **P1 — mkdir直後の空ディレクトリへの差し替えを受け入れる**  
  **箇所:** `tools/nagi-developer-platform/generator/generate.py:320–327`  
  **条件:** `mkdir` と `open` の間に作成ディレクトリが移動され、同名の別の空ディレクトリが置かれると、`os.listdir(child)` は空なので検査を通ります。第三者のディレクトリを作成物として記録し、その中へファイルを書き込み、失敗時には削除対象にもします。「空」は作成物の同一性の証明になりません。  
  **最小修正案:** 書込み・削除対象の同一性を確保できる作成方式に変更してください。その保証ができない競合は、PROVISIONALへの記載だけで許容せず、安全側に中止・残置する必要があります。`mkdir→open` 間の空ディレクトリ差し替えを回帰テストに追加してください。

差分には担当path外の `reviews/ndp-runner-ndp-03.md` も含まれますが、レビュー記録のみでした。実validator未準備時の明示的skipと、build・target成功を主張していない点は確認しました。テストは再実行していません。

NDP-REVIEW: FIX

## Round 3 — codex review of 72424cddfe94

P1が2件残っています。

- **P1 — 同条件の空ディレクトリへの差し替えを受け入れる**
  
  **箇所:** `tools/nagi-developer-platform/generator/generate.py:218–225`、`test_generate.py:540–549`
  
  **条件:** `mkdir`→`open`間に、同一uid・mode `0o700`の別の空ディレクトリへ差し替えると検査を通過します。生成器は既存ディレクトリへ書き込み、最後に権限も変更します。「既存ディレクトリは空でも拒否」という要件に違反します。PROVISIONALへの記載では要件を満たせません。回帰テストも、この違反を成功として固定しています。
  
  **最小修正案:** 排他的に管理できる作業領域で作成し、既存先を置換しない公開方式に変更してください。現在の方式を維持するなら、並行差し替えを許容する要件変更の承認が必要です。

- **P1 — 親ディレクトリの移動後に範囲外へ書き込む**
  
  **箇所:** `tools/nagi-developer-platform/generator/generate.py:145`、`:268`、`:317–325`
  
  **条件:** preflightで`parent_fd`を取得した後、別プロセスがその親ディレクトリを許可root外へ移動すると、保持したfd経由の生成は移動先で続行されます。最後の`verify_published()`は異常を検出しますが、既に範囲外へ生成物を書き込み、残置しています。no-followはディレクトリの移動を防ぎません。
  
  **最小修正案:** 祖先の移動を防げる排他管理を生成中の前提として実装するか、書込み先を強制的に閉じ込める方式へ変更してください。親取得後にroot外へ移動する回帰テストを追加し、範囲外に書き込まないことを検証してください。直前のpath再検査だけでは競合は解消しません。

差分には担当path外の`reviews/ndp-runner-ndp-03.md`もありますが、レビュー記録のみです。削除処理の撤去、実validator未準備時の明示的skip、build・target成功を主張していない点は確認しました。テストは再実行していません。

NDP-REVIEW: FIX

## Round 4 — claude review of d096fa64ca95

**レビュー結果: P1/P2 なし。** 実装の確認は `generate.py` のみで、`test_generate.py`、README、`contract/` の差分は開いていません。テストの再実行もしていません。このセッションでは Bash が拒否されたためです。runner の報告（41 件 OK、skip 1 件）を前提にしています。

**確認できた点**
- 出力先は `mkdir` で排他的に作成し、ファイルは `O_EXCL|O_NOFOLLOW` で作成します。祖先は no-follow で辿ります。このため、既存の出力先、非空ディレクトリ、symlink は拒否されます（`generate.py:70-86,157`）。
- `.git`・`sdk`・`tools/nagi-pkg` 配下は拒否します（`generate.py:42,139`）。
- 削除・rename・上書きは一切しません。失敗時は作成済みの項目を `left_behind` に報告します。
- 結果は `built:False` と `validated:False` を返し、host 上のファイル生成に限った結果だと明示しています。build の成功を主張していません。
- 生成内容は NDP-02 の `contract/contract.py` に委譲しています。二重実装はなく、`contract` ディレクトリを経由しない書込みもありません。担当 path 外の変更は `reviews/ndp-runner-ndp-03.md` のみで、runner が出力するレビュー記録です。

**P3（軽微）**
- `generate.py:198-202`、`generate.py:221`: `_current_umask()` は umask を set して復元するため、同一プロセス内の並行スレッドとは競合します。同じ並行性の限界として README に追記してください。
- `generate.py:221`: `_claim_directory` は mode が `0o700` ちょうどであることを要求します。`umask 0o277` のように owner の x ビットを落とす umask では、正常な作成でも `output_raced` と誤判定します。実害はほぼありません。README の「仮定」に一行書くか、`mkdir` 後に `fchmod(0o700)` で揃えるのが最小修正です。
- 並行プロセスによる敵対的な競合（TOCTOU、祖先ディレクトリの移動など）は、README の「脅威モデルと限界」節で限界として明記されています。追加の対応は不要です。

**疑問（確証なし）**
- `os.path.realpath(dest_root)` は、root 自体が symlink であることを許容します（`generate.py:126`）。README に「呼び出し側が選ぶ root は symlink でもよい」と書かれているので、設計どおりと判断しました。

実 validator が未準備のため、manifest の検証テストは skip になっています。この skip は報告で明示されており、成功扱いにはなっていません。

NDP-REVIEW: PASS

