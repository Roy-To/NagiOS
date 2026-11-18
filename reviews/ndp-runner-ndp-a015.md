# NDP-A015 自動レビュー記録

## Round 1 — codex review of 73389262df6b

- **P1 — テスト補助処理が、入力の範囲制限を迂回します。** [一時コピーの作成](tools/nagi-developer-platform/validation/test_api_snapshot_drift.py:107)の `shutil.copyfile` と[非変更確認](tools/nagi-developer-platform/validation/test_api_snapshot_drift.py:168)の `read_bytes` は、fixture や親ディレクトリがシンボリックリンクに置き換わると、リポジトリ外のファイルを読み、一時ディレクトリへコピーできます。巨大ファイルの読み込みにも上限がありません。実行前からリンクが存在するだけで起きます。最小修正は、両方の処理でリポジトリ側の読み込みに `validate.read_manifest` を使い、取得したバイト列から一時コピーを作ることです。
- **P2 — API 名のずれを検証するテストが、API 名の照合を削除しても通ります。** [API 名の変更テスト](tools/nagi-developer-platform/validation/test_api_snapshot_drift.py:139)は `ReadFile` を `WriteFile` に変えますが、`not on that line` は先に実行される evidence 照合でも出ます。最小修正は、一時コピーのソースと snapshot の evidence を一致させたまま、診断行の API 名だけをその行に存在しない名前へ差し替え、API 名固有のエラーを検証することです。実 fixture を使う既存の照合テストはそのまま使えます。

差分は担当パス内の2ファイルでした。行番号を変えた一時コピーのテストは変更後の行番号を含む不一致を確認しており、README は手書き snapshot との照合を PROVISIONAL として記載しています。テストは再実行していません。

NDP-REVIEW: FIX

## Round 2 — claude review of 85e78f63564a

**レビュー結果:P1/P2 は見つかりませんでした。** 目標と受入条件の 3 点を満たしています。

**目標と受入条件**
- `report.API_CASES` と `report.FIXTURES` から固定 fixture を引き、全プラットフォーム(windows, linux)の snapshot を検査しています(`test_api_snapshot_drift.py:87-98`)。検査内容は 3 つです。
  - `limitations` が元ソースを名指ししていること
  - 各 finding の行番号が存在し、evidence がその行にあること
  - `report.diagnose` が返す API 名がその行にあること
- snapshot の読込は `adapt_report_v1` と `load_report_document` を使っており、パーサの二重実装はありません。
- 行番号の変更、行末超過、API 名の差し替え、ソースへの行挿入、別ソースの名指しは、いずれも一時コピー上で検知を確認しています(`:123-177`)。
- API 名テスト(`:141-159`)は evidence 照合を通したまま API 名照合だけを働かせます。API 名の照合を削ると失敗します。
- 実ファイルは変更していません(`:179-187`)。
- README の追記は PROVISIONAL で、手書き snapshot との照合であり実 scanner 出力ではないこと、host のみで Nagi target 受入ではないことを明記しています。

**範囲とセキュリティ**
- 変更は `validation/` 配下と `reviews/` のレビュー記録だけです。`api/` と snapshot は変更していません。
- ファイル読込は `validate.read_manifest` を経由します。symlink を辿らず、サイズ上限があり、リポジトリ外は読みません。前回の P1 は解消済みです。

**P3(軽微)**
1. `test_api_snapshot_drift.py:81`: API 名の照合は大文字小文字を区別しない部分文字列一致です。たとえば `ReadFile` は `ReadFileEx` の行にも一致します。README の限界の項に「API 名も部分文字列一致」と一文足すと、説明が揃います。
2. 一時コピーの作成から検査までの間に、同時に動く別プロセスがファイルを書き換える競合は考慮していません。host pilot の範囲外ですが、README に限界として一行記載してください。

NDP-REVIEW: PASS

