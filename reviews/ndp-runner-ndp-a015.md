# NDP-A015 自動レビュー記録

## Round 1 — codex review of 73389262df6b

- **P1 — テスト補助処理が、入力の範囲制限を迂回します。** [一時コピーの作成](tools/nagi-developer-platform/validation/test_api_snapshot_drift.py:107)の `shutil.copyfile` と[非変更確認](tools/nagi-developer-platform/validation/test_api_snapshot_drift.py:168)の `read_bytes` は、fixture や親ディレクトリがシンボリックリンクに置き換わると、リポジトリ外のファイルを読み、一時ディレクトリへコピーできます。巨大ファイルの読み込みにも上限がありません。実行前からリンクが存在するだけで起きます。最小修正は、両方の処理でリポジトリ側の読み込みに `validate.read_manifest` を使い、取得したバイト列から一時コピーを作ることです。
- **P2 — API 名のずれを検証するテストが、API 名の照合を削除しても通ります。** [API 名の変更テスト](tools/nagi-developer-platform/validation/test_api_snapshot_drift.py:139)は `ReadFile` を `WriteFile` に変えますが、`not on that line` は先に実行される evidence 照合でも出ます。最小修正は、一時コピーのソースと snapshot の evidence を一致させたまま、診断行の API 名だけをその行に存在しない名前へ差し替え、API 名固有のエラーを検証することです。実 fixture を使う既存の照合テストはそのまま使えます。

差分は担当パス内の2ファイルでした。行番号を変えた一時コピーのテストは変更後の行番号を含む不一致を確認しており、README は手書き snapshot との照合を PROVISIONAL として記載しています。テストは再実行していません。

NDP-REVIEW: FIX

