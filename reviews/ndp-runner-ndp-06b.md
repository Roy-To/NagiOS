# NDP-06b 自動レビュー記録

## Round 1 — claude review of cf9395ae1060

## レビュー結果

`git diff` と変更ファイルを読みました。Bash が拒否されたため、テストの再実行や追加の実験はしていません。テストの成否は runner の報告（40件・7件・host runner PASS）をそのまま採用しています。

### 目標との照合
- **対象 path:** 変更は `ui/analysis/**`、`tests/.../ui/*.ui`（3件）、`evaluation/README.md` のみです。担当 path 外の変更はありません。
- **二重実装:** PT-01、SDK、validator、CLI、renderer への新規実装や架空 API はありません。
- **形式判別:** 共有の `_parse_xml` を通した後、ルート要素（`<interface>` か `<ui version="4.0">`）で自動判別します。`.ui` は Qt と GTK で共通のため拡張子では判別できず、README にその理由があります。
- **安全上限:** 入力サイズ・要素数・深さ、DTD/entity/PI 拒否、symlink・範囲外 path 拒否はすべて GTK にも共通で効きます。XML 解析前に GTK 固有の処理は入りません。
- **外部 resource と実行:** file、resource、include、signal handler、closure、script は文字列として保持するだけです。取得も実行もしません。
- **テスト:** `test_gtk_builder.py` は実際のパーサと CLI を使い、境界値（ちょうど上限と上限超過）、Unicode、決定性、入力不変、symlink・FIFO、エラーの無害化を検証しています。
  - レイアウトを変えるとレポートが変わることも確認しています。
  - `builtins.open`、socket、subprocess、eval などを patch して inert 性を検証しています。
  - 決め打ちの期待値で通るだけのテストにはなっていません。
- **host と Nagi target の区別:** README に「Nagi target 受入は未実行」「GTK は focused suite のみで runner には未追加」と明記されています。混同はありません。
- **Qt 回帰:** Qt の既存 18 テストは通り、`_analyze_qt` は元のコードを移しただけです。

### P1 / P2
なし。

### P3（軽微）
1. `analyze.py` の `_analyze_gtk` の `unknown` 収集（`element.tag not in GTK_TAGS`）
   - 未知要素が入れ子になると、祖先の未知要素ごとに子孫の内容を重複して保持します。出力は最大でも 深さ32 × 要素2048 程度に収まり、安全上は問題ありません。
   - 修正するなら、入れ子の未知要素は最上位のみ `content` に持たせます。
2. `GTK_TAGS` に `include` と `template` がありません。
   - `include` は resources に載るうえ `unknown` にも二重計上されます。
   - 最小修正は `include` を `GTK_TAGS` に足すことです。
3. `test_gtk_builder.py` の `temporary_directory` は `Path(__file__).parent`（ソースツリー内）に一時ディレクトリを作ります。
   - 終了時に自動削除されますが、`tempfile` の既定場所を使う方が安全です。
   - 担当 path 内なので許容範囲です。

### 疑問（確証なし）
- GTK4 の `<lookup>` の対象や `<relation>` の参照先は未解決として報告しません。README の「未対応」に明記されており、目標の範囲外と判断しました。

NDP-REVIEW: PASS

