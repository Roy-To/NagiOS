# NDP-07 — UI対応表・未対応レポート

NDP-06 `analysis/analyze.py` の実出力（`analysis_version=1`、Qt Designer / GTK
Builder）を標準入力から読み、Nagi UI契約への**仮の候補**と不足をJSONまたは
Markdownで出力する非AIのhostツール。Python 3.9の標準ライブラリだけを使う。
renderer、UI契約、SDK、元UI、業務ロジックは生成・変更しない。

## 実行

repository rootから:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/ui/analysis/analyze.py \
  --root tests/developer-platform/evaluation/ui task_panel.ui \
  | PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/ui/mapping/map_ui.py

PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/ui/analysis/analyze.py \
  --root tests/developer-platform/evaluation/ui gtk4_task_panel.ui \
  | PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/ui/mapping/map_ui.py --format markdown

PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/nagi-developer-platform/ui/mapping -p 'test_*.py'
```

`gtk3_task_panel.ui`、`gtk_minimal.ui`も同じ経路を使う。入力失敗時は非ゼロで終了し、
stderrに固定error codeだけを返す。標準入力はUTF-8 JSON、上限4 MiB・構造深度64・
値100,000個。重複JSON key、不明な解析版、不正なcollectionは拒否する。
これは内部ツールの処理上限であり製品要件ではない。

`map_analysis(analysis, contract=None, rules=None)` はJSON互換dictを返す。
`json_report` / `markdown_report` で出力できる。差し替え引数は信頼済みのローカル
adapter用であり、入力UIからルール、契約path、関数を指定する機能はない。

## 読み方

`mapping.status` と `mapping.relation` は契約の語彙に対する分類。
`approval=PROVISIONAL` は全候補に付き、UI ownerの承認を表さない。

| status | 意味 |
|---|---|
| `supported`（対応） | 同じ役割を表す契約の部品候補が存在する。描画・実行・意味の完全互換は未検証 |
| `partial`（部分対応） | 合成・再設計が必要な候補。配置、状態、範囲等の変換は未実装 |
| `unsupported`（未対応） | 調査した契約に相当する部品がない、またはこの工程がbackend接続を提供しない |
| `needs_decision`（要判断） | 意味情報・候補・根拠が不足。未知クラスや契約の版／根拠の変更を含む |

relationは `direct`（語彙の直接候補）、`redesign`（再設計）、`unavailable`
（未提供）、`unknown`（不明）。rowの`status`は全体の移植状態で、語彙対応があっても
最大`partial`。`summary.port_complete`は常にfalseであり、移植完了の判定器ではない。

rowの`status`と`mapping.status`は独立した情報ではなく、`map_ui._row_status` 1か所で
定義した導出関係にある: `status = mapping.status`（`unsupported` / `needs_decision`）、
それ以外（`supported` / `partial`）は`partial`。`summary.status_counts`は行の`status`、
語彙分類は`mapping.status`を見る。テストが全fixtureの全rowでこの関係を検証する。
どちらかを廃止するかは後続のUI owner判断（下記）。

各rowには次の独立した評価がある。全項目の`verification=not_run`はhostで構造を
照合しただけで、そのUI動作を実行していないことを表す。

| axis | 根拠と不足 |
|---|---|
| `appearance_layout` | ComponentKind / LayoutKind等の候補。token、style、geometry、size policy、grid span、packing、実描画は未検証 |
| `pointer_keyboard` | ButtonModel等の状態モデル候補。device event変換やアプリの操作接続は未実装。TextFieldModelはEnter/Escape等に限られ、cursor／selection／IMEの実装ではない |
| `focus` | 元のtab order／focus propertyを保持。情報欠落は要判断。FocusManager／FocusScopeへの変換、modal scope、disabled対象、focus復元は未検証 |
| `accessibility` | Qtのaccessible属性、GTK3のinternal-child accessible、GTK4 accessibilityを保持。stable name key／role／state／relationへの変換とservice連携は未実装 |
| `localization` | UTF-8本文・翻訳属性を保持。MessageKey、en-US／ja-JPのcatalog、fallback、shaping、長文の測定とoverflowは未検証。translatable属性だけで対応済みにしない |
| `backend` | renderer、device input、focus／a11y service、catalog、event／binding／業務ロジックのadapterはこの工程で提供しない |

JSONの`source_analysis`に解析器の**全出力**をdeep copyで保存する。各rowにも元factを
残し、部品、layout、connection、binding、resource、unknown、custom widget、
unresolved reference、declared slotを列挙する。元の順序・名前を保ち、GTKのcontainerが
widgetとlayoutの両方に現れる場合は別rowにする。summaryの件数はrow数であり画面部品数
ではない。追加propertyや配置証拠を保持することは、その意味を対応済みにすることではない。
Markdownは候補・軸別statusと理由・契約根拠を一覧にし、全元factの確認にはJSONを使う。

### 実fixtureの例

| 入力／対象 | 候補と判定 | 残るもの |
|---|---|---|
| Qt `addButton` | `ComponentKind::Button`、direct / supported（PROVISIONAL） | 外観・pointer/keyboard・focusはpartial、a11yは要判断、backendは未対応。`clicked()` → `addTask()`は別rowで未接続 |
| Qt `taskName` | `ComponentKind::TextField`、direct / supported（PROVISIONAL） | accessibleNameとtab orderは保持するが、IME、service、stable key、catalogは未接続 |
| GTK3 `body` | 明示verticalから`LayoutKind::Column`、redesign / partial | packing、spacing、size policyの変換と描画 |
| GTK4 `grid` | `LayoutKind::Grid`、redesign / partial | row／column／column-spanは元factに残す。正しい変換、描画、操作は未検証 |
| GTK3 `entryAccessible` | AtkObjectはunknown / needs_decision | entryのa11y証拠として関連付けるが、描画widgetに変換しない |

## XAML (WPF / WinUI) — NDP-06d

`format=xaml-ui` の解析結果も同じ経路で処理する（`rules.json` の `xaml-ui` 節）。
fixture: `xaml_minimal.xaml`、`wpf_task_panel.xaml`、`winui_task_panel.xaml`（`.ui`ではないので
上のGTK/Qt例のglobには入らず、`test_mapping_xaml.py` が扱う）。

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/ui/analysis/analyze.py \
  --root tests/developer-platform/evaluation/ui wpf_task_panel.xaml \
  | PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/ui/mapping/map_ui.py
```

| XAML | 候補 / 判定（PROVISIONAL） |
|---|---|
| Button / CheckBox / RadioButton / ComboBox / ToggleSwitch / TextBox / Separator | 契約語彙の直接候補（Button / Checkbox / Radio / Select / Switch / TextField / Divider）。direct / supported だが全体statusは最大partial |
| TextBlock / Label | `ResolvedText`（再設計） |
| Window | `ApplicationShellSpec`、Page / UserControl / Border / GroupBox / Expander は `Panel`（再設計） |
| ContentDialog / ListBox / ListView / ProgressBar / ScrollViewer | Dialog / List / ProgressIndicator / ScrollContainer（再設計） |
| Grid / UniformGrid | `LayoutKind::Grid`、WrapPanel は `Row`（再設計）。row/column定義、span、size policyは未変換 |
| StackPanel | **`Orientation` が明示されている場合だけ** Row / Column。省略時の既定は推測せず要判断（GtkBoxと同じ扱い。`ORIENTED_LAYOUTS` 1か所） |
| Canvas / DockPanel / RelativePanel / VariableSizedWrapGrid / Image / PasswordBox | 現調査の契約に相当部品なし（unavailable / unsupported） |
| 上記以外の認識済みcontrol（Slider等）・custom control・未知要素 | 規則なし／未知 → needs_decision。基底型や名前から自動対応しない |

`AutomationProperties.*`、`Text`/`Content`/`Header`等、`TabIndex`/`IsTabStop`等は
既存のa11y・text・focus証拠名の表（`ACCESSIBLE_NAMES`/`TEXT_NAMES`/`FOCUS_NAMES`）に追加しただけで、
意味解釈はしない。event・binding・resource・`x:Class` は他形式と同じく未接続（backend=unsupported）。

### 仮定（PROVISIONAL, XAML）

- 上表は内部の暫定照合でUI ownerの承認ではない。変更は `rules.json` の `xaml-ui` 節と上記の名前表
  だけで行える。承認状態は自動昇格しない。
- Page/UserControl/Border等を `Panel`、WrapPanelを `Row` とするのは再設計候補としての仮置き。
- hostでの結果はNagi target受入ではない。

## 仮定（PROVISIONAL）

- UI ownerの承認前の内部照合であり、公開support matrixや新SDK APIではない。
  `rules.json`にclass別候補・relation・理由・根拠・model名を分離した。
  実装方針の変更はこのデータとadapterを差し替える。承認状態を自動昇格しない。
- 現行`nagi-ui` 0.1.0の
  [UI契約](../../../../docs/architecture/ui-design-system.md) と
  [実API](../../../../user/nagi-ui/src/lib.rs)を読み取り参照する。
  `contract.py`は文書・Cargo.tomlと`src`のうちsnapshotに**列挙した**ファイルの
  SHA-256を、同梱`contract_snapshot.json`と照合する。加えて、snapshotの`enumerate`
  （現在は`user/nagi-ui/src`配下の`*.rs`）にあるのにsnapshotへ未列挙のファイルは
  `unlisted`として検知する。元APIの削除、版の変更、ファイル欠落、内容変更、
  未列挙ファイルの追加で部品の全非backend軸と対応statusを要判断へ戻す。候補名は
  過去の候補として残る。変更が意味に無関係でも保守的に要判断になる。
  snapshotは根拠の読取記録であり正式なSDK版契約ではない。
  snapshotの更新はowner調整後に文書・APIと各ルールを再確認してから行う。
- 列挙範囲の仮定: `enumerate`は`src`配下の`.rs`だけで、`examples/`、`build.rs`、
  `src`外のファイルは対象外。範囲を広げる／狭めるのはsnapshotデータの変更だけで
  行える（UI ownerの要判断）。未列挙ファイルは`sha256=null`、Markdownでは
  `not_in_snapshot`と表示する。
- symlinkの扱い（PROVISIONAL、fail-closed）: `contract.py`は`enumerate`配下を
  `os.walk(followlinks=False)`で列挙し、symlinkを**一切辿らず・読まない**。
  repo外を指すものもrepo内を指すものも区別せず同じ扱いで、`.rs`のsymlink、
  symlinkのdirectory、`enumerate`のdir自体や途中のsymlink、およびsnapshot列挙済み
  ファイルのpath途中にあるsymlinkは、evidenceに固定の`state="symlink_rejected"`
  （`sha256`／`actual_sha256`は共にnull、Markdownでは`not_in_snapshot`）として載り、
  全体が`needs_review`になる。リンク先のpath・内容・hashは出力しない（伏字化した固定
  コードのみ）。repo内を指すsymlinkも「除外」ではなく「拒否」とするのは、同一内容の
  別名で契約が変わったことを見逃さないための保守的な判断。repo rootより上位の
  directoryがsymlinkであること（例: 作業場所自体）は検査対象外。Windows等の
  junction／reparse pointは未検証。方針の変更（除外へ緩和等）はこの1か所の
  差し替えで行える（UI ownerの要判断）。
- QWidgetはPanel、main windowはApplicationShellSpec、QLabel／GtkLabelはResolvedText
  を使う再設計候補とする。rich text、画像、link等の等価性は仮定しない。
  Qt form layoutはGridへの再設計候補。grid span／絶対座標等は実装しない。
- GtkBoxはorientationが明示されている場合だけRow／Columnを選ぶ。implicit defaultは
  推測しない。GtkStackのpage切替とNagi LayoutKind::Stackの重なり配置は異なるため
  未対応。GtkFixed、GtkPaned、GtkImage、GtkTextView、GtkSpinButtonも現調査の未対応。
  unknown custom widgetはextends先が既知でも自動対応しない。
- 対応は文字列を同定して候補を提示する段階に限る。source property、disabled状態、
  focus policy、binding式、signal handler、業務ロジックの完全な意味解釈は行わない。
  元UIにないfocus／a11y／catalog情報は補完せず要判断にする。
- backendの未対応は、このtoolと参照したbackend-neutral契約が提供しないという意味。
  Nagi全体のrendererの不存在は断定しない。hostテストをNagi targetのGUI受入としない。

UI ownerへの後続確認候補: 上記部品対応とlayout再設計、画像／multiline editor／
spin／splitterの部品方針、公開可能なAPI範囲・版識別、device input／IME、modal focus、
accessibility service、翻訳とtext layout、型付きevent／binding接続の担当境界。
本工程の実装はこれらの承認を前提とせず、権限やUI契約を変更しない。

## 検証

NDP-07時点でrunner指定コマンドは23テスト成功（host、Python 3.9）だった。NDP-06cで
契約の新規ファイル追加検知、row statusの導出関係、一時ディレクトリの既定化を追加した。
その後の件数はrunnerの実行結果を正とし、ここには固定しない。
4つの実fixtureを実解析器の
API／CLIで読み、対応候補・未対応部品・未知要素・未解決参照・event／binding未接続、
focus／a11y情報、長い日本語と英語、全候補APIの実在、契約削除／版変更／API変更／ファイル追加、adapter差し替え、
決定性、入力と出力の独立性、元ファイルの不変性、JSON入力の異常系を確認する。
renderer・操作・Nagi targetの試験は実施していない。
