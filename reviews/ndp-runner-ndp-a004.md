# NDP-A004 自動レビュー記録

## Round 1 — claude review of cb5156fc5d6a

## レビュー結果

P1/P2 に該当する問題は見つかりませんでした。差分は読みましたが、テストの再実行はしていません。runner の結果（mapping 31件、analysis 45件、他も OK）を前提にしています。

### 受入条件の確認
- **repo 外を指す symlink を読まないこと**: `test_symlink_to_file_outside_repo_is_not_read` が確認しています。`_sha256` の呼び出しを記録し、`leak.rs` や `outside` を読んでいないことを検証しています。出力 JSON にハッシュや外部パスが含まれないことも見ています。
- **repo 内を指す symlink の扱い**: 実装・テスト・README がすべて「拒否（`symlink_rejected`）」で一致しています。README は `ui/mapping/README.md` に PROVISIONAL として明記されています。
- **既存 snapshot と既存テスト**: `contract_snapshot.json` は差分に含まれません（`:!*.json` で除外して確認）。`test_mapping.py` は追加のみで、既存テストの変更はありません。
- **analysis README の「40 tests」**: 削除されています。

### 設計面で問題がなかった点
- `os.walk(followlinks=False)` で、ディレクトリ symlink は `dirs` から除外して辿りません。ファイル symlink は `is_symlink()` を先に判定しています。壊れた symlink も `.rs` なら拒否側に入ります。
- 列挙済みファイルの途中パスに symlink があれば `_has_symlink` で拒否されます。`src` 自体が symlink の場合は、`_unlisted` でも個別ファイルの検査でも拒否されます。
- 担当 path 外の変更や二重実装はありません。外部資源の取得もありません。

### P3（軽微）
1. `mapping/README.md` の追記（`contract.py:36-57` 付近の `os.walk` 周辺）: 並行する別プロセスが検査と読み取りの間に symlink へ差し替える競合（TOCTOU）は、限界として書かれていません。`_has_symlink` の確認から `_sha256` で読むまでに間があります。仕様どおり host pilot の範囲外ですが、「README に限界として記載」を求めます。1行追記で足ります。
2. `contract.py` の `os.walk` に `onerror` がなく、読めないディレクトリは黙って飛ばされます。これは従来の `OSError: continue` と同等の挙動で、今回の退行ではありません。気になるなら README に「未検証」と書いてください。
3. 追加テストを旧実装で走らせて失敗することは確認されていません（実装 AI の報告にもあります）。ただ、`_sha256` の読み取り記録で検証しているため、旧実装でも `leak.rs` を読むことは検出できる構造です。実害のある指摘ではありません。

### 疑問
なし。

NDP-REVIEW: PASS

