# NDP-07 自動レビュー記録

## Round 1 — claude review of d43ee72573be

NDP-07 は目標を満たしており、P1/P2 の指摘はありません。

実装は NDP-06 の `analyze.py` 出力を入力に、仮の対応候補と6軸の不足レポートを出す構成です。変更は `tools/nagi-developer-platform/ui/mapping/**` の6ファイルだけで、担当path外への変更や renderer・UI契約・SDK の二重実装はありません。runner のテスト結果と HEAD の diff・ソースを読んで確認しましたが、sandbox の制約で Bash を実行できなかったため、テストの再実行はしていません。

## 確認できた点
- **host 結果の扱い**: `summary.port_complete` は常に false で、`target_acceptance` は `not_run`、全軸の `verification` も `not_run` です。README の PROVISIONAL 節には、host 結果を Nagi target の受入としないと明記されています。
- **契約の根拠**: `contract.py` が UI 契約の文書と `nagi-ui` の Cargo.toml・src の SHA-256 を、同梱 snapshot と照合します。変更、欠落、版変更のいずれでも、部品の非 backend 軸と対応 status が要判断に戻ります。この3パターンはテストされています。
- **架空API対策**: `test_every_rule_candidate_has_real_contract_evidence` が、`rules.json` の候補が実際の Rust ソースに存在するかを確かめます。
- **入力の安全性**:
  - JSON 入力は 4MiB、深度 64、100,000 値までに制限されています。重複 key も拒否します。循環参照は `analysis_limit` で拒否します。
  - エラーは固定コードだけで返り、入力内容は出ません。
  - 入力値は読み取り先や実行内容に使われず、外部取得や任意コード実行もありません。
- **テストの実質性**: 実 fixture 4本を実 analyzer の API と CLI 経由で読んでいます。未知部品、未対応部品、未解決参照、binding 欠落、focus 欠落、長い日本語・英語、決定性（dict 順序反転を含む）、入力・出力の独立性、契約ドリフト、rules の差し替えをカバーしています。
- **README の PROVISIONAL 節**: 仮定がデータ（`rules.json`）と adapter に分離されており、差し替えやすい構造です。

## P3（軽微）
1. `contract.py:19-26` と README:87: README は「全src」の照合と書いていますが、実際は snapshot に列挙した10ファイルだけを照合します。`user/nagi-ui/src` に新しいファイルが追加されても検知しません。`src/*.rs` を列挙して差分を要判断にするか、README の文言を「列挙ファイル」に直してください。
2. `test_mapping.py:234`: ドリフト試験が担当path内に一時 dir を作ります。`finally` で削除されるため害はありませんが、`tempfile` の既定 dir を使うほうが安全です。
3. `map_ui.py:186`: row の `status` は、対応が direct でも `partial` に丸められます。README に説明があるので問題ありませんが、`mapping.status` との二重表現は UI owner に渡す前に整理候補です。

## 疑問
なし。

NDP-REVIEW: PASS

