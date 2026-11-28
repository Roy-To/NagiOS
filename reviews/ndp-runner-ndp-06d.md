# NDP-06d 自動レビュー記録

## Round 1 — claude review of 13dba8dad7c8

P1/P2 は見つかりませんでした。ただし Bash が不許可（don't ask モード）だったため、実行して確かめたことはありません。`analyze.py` の全 diff、`map_ui.py` の XAML 該当箇所、`test_xaml.py` のテスト名一覧を読んで判断しました。次の項目は未確認です。
- `rules.json` の diff、`test_mapping_xaml.py`、README、fixture の中身は開いていません。
- 追加の挙動プローブは実行していません。
- runner が報告した全テストの成功は、再実行せずそのまま信頼しています。

## 確認できた点
- **形式選択と共有上限**: パース後のルート namespace で XAML を判定し、Qt/GTK の分岐は変えていません。`_parse_xml_namespaces` はサイズ・要素数・深さの上限、DTD/entity/PI 拒否を共有しています。`analyze_file` の symlink・範囲外 path 拒否もそのまま共有しています。
- **実行・取得なし**: markup extension は文字列処理だけで解析し、深さ上限は 8 です（`analyze.py` の `_markup_parse`）。`Source` や `NavigateUri` は値として記録するだけで、取得しません。`x:Class` は `form_class` に保持するだけです。
- **テスト範囲**: 要求された観点（最小画面、入れ子、未知要素、event、binding、壊れた構文、上限の境界、Unicode、決定性、入力不変、CLI）はテスト名から揃っています。実装が使う関数・フィールドを検証しているとまでは確認していません。
- **未実行の扱い**: 実装 AI の報告は、`run_host_checks.py` が XAML を実行しないこと、結果が host 限定で Nagi target の受入ではないことを明記しています。未実行テストの成功扱いはありません。
- **担当 path**: 変更は `ui/analysis`、`ui/mapping`、`tests/developer-platform/evaluation` に収まっています。

## P3（軽微）
1. `analyze.py` の `explicit_name`（`_analyze_xaml` 内）と、名前の一意性チェック。名前を文書全体で一意とみなすため、別の `DataTemplate` 内で同じ `x:Name` を使うと `duplicate_object_name` で全体が失敗します。実在の WPF/WinUI の XAML では起こりえます。README の PROVISIONAL 節に記載済みなので、現状は許容できます。将来は template スコープ付きの名前にする案があります。
2. `analyze.py` の `scan_attributes`。`Click=""`、または event 属性が `owner` なしで出た場合は `incomplete_connection` で全体を拒否します。部分的な XAML でも解析全体が失敗するため、`unknown` に落とす方が寛容です。ただし GTK と挙動を揃えた判断なら現状のままで構いません。
3. 同時に動く別プロセスによるファイルシステム競合（TOCTOU など）は、README に限界として記載済みと報告されています。この記載は私は確認していません。

## 疑問
- `analyze.py` の `kind_of` は、未知 namespace の要素を `custom`（widget 扱い、`recognized: False`）にします。`x:Key` 付きの要素だけは `unknown` に回ります。`custom` の要素が `Style` や `Setter` のような非視覚要素だった場合、widget として数えられて NDP-07 の mapping に出る可能性があります。`test_custom_control_is_unrecognized_and_not_loaded` の範囲だけでは判断できません。

NDP-REVIEW: PASS

