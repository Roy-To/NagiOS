# NDP-A002 自動レビュー記録

## Round 1 — claude review of 63be6ecdd9a8

P1/P2 はありません。`generate.py` の修正とテストは、目標と受入条件を満たしています。再実行はしておらず、runner の結果（generator 48 件通過、skip 1 件）とコードの読解に基づいています。Bash は権限で拒否されたため、diff はファイルを直接読んで確認しました。

**確認できた点**
- **原因と修正:** 原因は、`mkdir(0o700)` が実際には `0o700 & ~umask` の mode で作られるのに、判定が固定の `0o700` だったことです。`generate.py:205-234` は期待 mode を `0o700 & ~umask` に変え、受け入れ判定（実 directory・所有者・期待 mode・空・同一 identity）を通った後にだけ `fchmod(0o700)` します。差し替えられた directory に chmod が掛かることはなく、fail-closed は弱まっていません。
- **umask の読み取り:** umask は `generate.py:263` で最初の `mkdir` より前に 1 回だけ読むため、作成と判定で値がずれません。
- **`unsupported_umask`:** owner の read bit（`0o400`）が落ちる umask では、directory を作る前に拒否します（`generate.py:265-269`）。作られた directory を owner が open できず検証できないためで、妥当です。`0o177` と `0o377` では owner の read が残るので、`fchmod` で救済できます。
- **既存テスト:** 受入条件の拒否系（group/world 書込み可への差し替え、中身ありへの差し替え、所有者違い）は、`UmaskTests` が `MASKS` 6 種すべてで `output_raced` を検証しています。拒否された directory が widen されず、何も書かれないことも確認しています。
- **テスト:**
  - 成功側（`test_generation_succeeds_and_final_modes_follow_the_umask`）は、公開後の mode と生成物一覧を実値で検証しています。
  - 書込み時の mode は、`os.open` の spy で `0o277` 下でも親が `0o700` であることを確認しています（`test_directory_is_private_while_written_even_under_restrictive_umask`）。
  - umask の復元は `run_with_umask` が `finally` で行います。
- **範囲外の変更:** 担当 path 外の変更はありません。

**P3（軽微）**
- `test_generate.py:661`: 拒否側のテストは `0o700 & ~mask` を決め打ちの式で比較しており、実装の式を鏡写しにしています。実害はなく、`0o277` なら `0o500` という具体値のコメントがあれば読みやすくなります。
- README: 期待 mode と一致する同一 uid による差し替えは判別できない限界と、umask 取得が「設定して即復元」で他 thread と併用する場合は保証しない点は、すでに PROVISIONAL として記載されているとのことです（README は直接読んでいません）。追加の要求はありません。
- 実装 AI が申告したとおり、修正前コードへの戻しによる回帰確認は未実施です。ただし `0o277` で `output_raced` になる点は修正前の実行で確認済みとのことで、許容範囲です。

結果はすべて host 上のもので、Nagi target の検証ではありません。

NDP-REVIEW: PASS

