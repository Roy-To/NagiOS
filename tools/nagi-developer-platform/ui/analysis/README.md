# NDP-06 Qt Designer / GTK Builder XML inspection pilot

This local host tool reads one explicit UTF-8 Qt Designer 4.0 or GTK Builder
XML file and returns source facts. Qt Designer XML was chosen first because
the NDP-10 synthetic input already exercises hierarchy, layout, text, accessibility metadata, tab
order and declared events in a small, dependency-free case. The existing
[UI contract](../../../../docs/architecture/ui-design-system.md) remains owned
by Nagi UI; this tool does not map to, modify or implement that contract.

Implementation scope: tools/nagi-developer-platform/ui/analysis/**, with synthetic
inputs under tests/developer-platform/evaluation/ui/**. Base fixture commit:
715d308. GTK is the second input format (NDP-06b), implemented in its dedicated
worktree with the task's explicit path authorization.
Shared registry, SDK, CLI, Cargo, CI and product decisions are not modified.

## Run

From the repository root:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/ui/analysis/analyze.py \
  --root tests/developer-platform/evaluation/ui task_panel.ui
PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/ui/analysis/analyze.py \
  --root tests/developer-platform/evaluation/ui gtk4_task_panel.ui
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/nagi-developer-platform/ui/analysis -p 'test_*.py' -v
```

Input must be relative to the explicitly selected root. File opening uses
no-follow directory handles and rejects symlinks, traversal, non-regular
files and oversized reads. This file-opening path currently requires Unix
O_DIRECTORY/O_NOFOLLOW/O_NONBLOCK; other hosts fail as unsupported_host.
The bytes-only parser is independent of file-opening support.

Limits are 65,536 bytes, 2,048 XML elements and depth 32. The parser rejects
DTD/entity declarations, non-UTF-8/NUL input and non-XML processing
instructions before inspection. These are pilot implementation bounds, not
product performance requirements. XML nesting/count limits are checked while
feeding the parser, before building the output model.

The internal report preserves widget/layout ownership, properties and text,
tab order, declared signal/slot connections, bindings, resource references,
unrecognized elements/classes and unresolved references. Unknown content is
visible rather than silently mapped to a supported control. Recognized means
recognized by this parser, not supported by Nagi. Declared events always have
execution=not_run. Resources and expressions remain data and are never fetched
or evaluated. Errors contain stable codes without absolute input paths or
source content. Successful reports intentionally contain selected UI text.

This is an internal pilot model, not a public UI schema or PT-01 report.
It is not full Qt XSD validation or Qt application compatibility. In particular,
complex typed property values remain textual evidence; type-specific geometry,
font and resource interpretation is deferred. No renderer, event handler,
keyboard/focus service, accessibility backend or localization catalog runs.
NDP-07 mapping and UI-owner agreement remain separate work.

## Verification — 2026-11-15

18 real-parser/CLI tests pass on the existing Python 3.9.6: the actual NDP-10
input, Unicode/accessibility properties, hierarchy, unknown widgets/layouts,
bindings/scripts as data, unexecuted external resources, unresolved references,
malformed forms, duplicate identities, invalid encodings, XML declaration
attacks, byte/element/depth bounds, symlinks/traversal/FIFO rejection, bounded
file reading, deterministic results, input preservation and sanitized errors.
No dependencies were installed. This focused result does not make the normal
Python 3.10+ Nagi host or cargo-based doctor pass. Target execution is not performed. Independent review and re-review found
no remaining P1/P2 issues; see the [Codex review](../../../../reviews/ndp-host-pilots.md).

## Review fixes

Layout source attributes and item attributes (grid row/column/spans/alignment),
contained object identities and spacer properties are retained. Custom-widget
class/extends/header declarations and their metadata are retained as data;
headers are never loaded and custom widgets are not promoted to supported
Nagi controls. Two regression tests fail on the original implementation and
pass on the fixed code. The existing scope/encoding/resource-limit checks
remain in place.

## GTK Builder subset — NDP-06b

Format selection is automatic **after the shared bounded XML parse**:
`<ui version="4.0">` selects Qt and `<interface>` selects GTK Builder. Both
frameworks use `.ui`, so suffix detection would be ambiguous. Distinct root
elements allow an unambiguous choice without changing the existing API/CLI or
evaluation runner. Other roots remain `unsupported_format`. GTK `requires`
versions are retained as declarations, not used to claim GTK version validity.
GTK 3 and GTK 4 use this same structural subset; no GTK library is imported.

GTK reports use the existing internal collections (`widgets`, `layouts`,
`connections`, `bindings`, `resources`, `unknown`, `unresolved_references`)
with `format=gtk-builder-ui` and `analysis_version=1`. GTK-specific evidence is
additive inside this internal model; no public schema or UI-owner contract
is introduced. Existing Qt report fields and values are unchanged.

| Source facts | Retained representation |
|---|---|
| Objects, including multiple top-level objects and nested property objects | Source order, class, ID, source attributes, owning `parent_widget`; unknown classes stay unrecognized |
| Anonymous objects (valid Builder input) | Deterministic `@gtk-object-N` report identity, suffixed with `_` on source-ID collision; original attributes remain unchanged; reference resolution uses source IDs only |
| Container layout | GtkBox/VBox/HBox, GtkGrid/Table, GtkFixed, GtkStack, GtkOverlay and GtkPaned appear as both hierarchy objects and layout owners with the same identity; a layout's `parent_widget` is its owning object |
| Child placement | Every object's `children` retains child attributes (`type`, `internal-child`), child identities/properties and GTK 3 `packing`; recognized layouts expose these as `items` too; GTK 4 child-object `layout` properties retain row/column/spans as data |
| Controls and text | A small class allowlist (windows/dialog, label, entry, button/check/radio, combo, switch, frame, scrolled window, image, text view, list box, separator, spin and progress) plus the containers above; direct property text and nested evidence use the existing `properties`/`values` shape |
| Translation and accessibility | UTF-8, XML character references, interface attributes such as `domain`, property attributes such as `translatable`/`context`/`comments`; GTK 3 accessible properties/AtkObject/internal-child; GTK 4 `accessibility` property/relation/state declarations |
| Signals | `connections`: owner as `sender`, exact signal name, handler as `slot`, optional `object` as `receiver` (empty when absent), all attributes including `after`/`swapped`, always `execution=not_run`; no callback target is inferred |
| Bindings | `bind-*` property attributes and binding expression trees, including lookup/closure text and attributes; always `execution=not_run`; missing explicit `bind-source` IDs and signal receivers are reported unresolved |
| Resources and unknowns | `requires`, file/filename/resource/pixbuf properties, `include` declarations (resources only, not unknown); unknown classes and unknown element trees with text/attributes retained; none loaded |

Unknown element trees: every unknown element is listed in source order, but only
the outermost one carries the subtree as `content`. Unknown descendants inside it
are listed with `nested_in_unknown: true` and no `content`, so evidence is kept
once instead of once per ancestor. Widgets (`object`) inside an unknown element
are still analyzed normally. `template` is intentionally still reported as unknown
(with its attributes and body as `content`) because composite templates are not
analyzed.

Properties containing nested objects retain ownership but do not imply packed
children: interpreting object-valued properties requires GTK class/type knowledge.
AtkObject and explicit layout-manager objects such as GtkGridLayout remain
unknown classes with their properties retained. Recognized never means supported
by Nagi. GTK has no Qt form class, slots block or explicit Qt tabstops; those
collections/fields remain empty rather than inventing equivalents.

The byte, element and depth limits, UTF-8/NUL checks, DTD/entity/PI rejection,
bounded regular-file reads, symlink/traversal rejection and sanitized error codes
are shared with Qt before any GTK inspection. XML's standard character references
are allowed; user-defined entities are rejected. Resources, handlers, closures,
scripts, bindings and library declarations are inert data. There is no framework
instantiation, import of declared classes, expression evaluation or resource I/O.

Not implemented: full GTK 3/4 schema/type/property validation, object-reference
typing/resolution beyond the explicit IDs described above, composite templates
(a template-only document is rejected as `missing_root_widget`), semantic layout
manager mapping, CSS/style/resource loading, translation catalog lookup, computed
properties/expressions, signal connection/execution, focus order inference,
accessibility backend, rendering, Nagi mapping or runtime compatibility. Unknown
syntax is evidence for follow-up rather than successful support.

Original synthetic fixtures are `gtk_minimal.ui`, `gtk3_task_panel.ui` and
`gtk4_task_panel.ui` under `tests/developer-platform/evaluation/ui/`. The GTK
focused tests exercise the real parser and CLI: minimal/nested/unknown objects,
both placement syntaxes (changed placement changes the report), signals,
bindings/accessibility, malformed input, exact/over-limit byte/element/depth
boundaries, DTD/entity/PI/encoding rejection, Unicode, deterministic anonymous
IDs and CLI output, source preservation, safe file opening and inert resources.
No dependencies, network or third-party sources are needed. The host evaluation
runner continues to exercise its original fixed C/Qt inputs; GTK coverage is in
the focused suite, not a new runner benchmark. Host checks do not establish Nagi
target acceptance.

## XAML (WPF / WinUI) as the third input — NDP-06d

Selection stays root-based after the shared bounded parse: a root element in the
XAML presentation namespace (`http://schemas.microsoft.com/winfx/2006/xaml/presentation`,
shared by WPF and WinUI) selects `format=xaml-ui`; `<ui version="4.0">` and
`<interface>` still select Qt and GTK. The file suffix is never used. WPF and
WinUI are not distinguished (`dialect_hints` only lists whether `clr-namespace:` /
`using:` namespace URIs were declared). The existing API/CLI and Qt/GTK reports are
unchanged; the shared parser additionally records `xmlns` declarations as data.

Same internal collections as Qt/GTK (`widgets`, `layouts`, `connections`, `bindings`,
`resources`, `unknown`, `custom_widgets`, `unresolved_references`, `tab_order`), plus
additive `namespaces` and `dialect_hints`. No public UI schema is introduced.

| Source facts | Retained representation |
|---|---|
| Hierarchy | Object elements in the provisional control table become `widgets` in source order with `parent_widget`; panels (Grid, StackPanel, WrapPanel, DockPanel, Canvas, UniformGrid, RelativePanel, VariableSizedWrapGrid) are also `layouts` with `items` (child placement) |
| Identity | `x:Name` or `Name` (equal values allowed; different values → `conflicting_object_name`); anonymous objects get `@xaml-object-N` (suffixed `_` on collision). Names are global: the same name in two templates is rejected as `duplicate_object_name` |
| Placement | Attached-property attributes (`Grid.Row`, `Canvas.Left`, …) of each child are kept in its `items[].attributes`; property-element children (`Button.Content`) are tagged with `property` |
| Text / properties | Attributes → `properties` (names as written; `x:` prefix for the XAML-language namespace); property elements (`Grid.RowDefinitions`) and direct text/inline content (property `Text`/`Inlines`/`Content`) with nested evidence |
| Events | Attributes whose name is in the provisional event table (also attached, e.g. `Button.GotFocus`) and `EventSetter` → `connections` (`slot` = handler text, `execution=not_run`). An empty handler is `incomplete_connection` |
| Bindings | `{Binding}`, `{x:Bind}`, `{TemplateBinding}` (also nested inside other extensions) and binding elements (`Binding`, `MultiBinding`, …) → `bindings` with the parsed argument tree, `execution=not_run`. `ElementName`/`x:Reference` names missing from the file → `unresolved_references` (`binding_source`) |
| Resources | `{StaticResource}`/`{DynamicResource}`/`{ThemeResource}` → `resources` (`resource_reference`); `x:Key` → `resource_definition`; `Source`/`ImageSource`/`NavigateUri`/`UriSource` text → `resources`. A key not defined **in this file** → `unresolved_references` (`resource_key`); scoping/merged dictionaries are not modeled |
| Accessibility / focus | `AutomationProperties.*`, `x:Uid`, `TabIndex` etc. stay ordinary properties; `tab_order` lists widgets with an integer `TabIndex` (ascending, source order for ties) |
| Code-behind | `x:Class` → `form_class`; the class, handlers and any named assembly are never loaded |
| Unknown | Unlisted element names, templates (`DataTemplate`/`ControlTemplate`, body kept as `content`, widgets inside still analyzed), meta-namespace elements, unknown or unparseable markup extensions (`parse=unparseable`), custom-namespace controls (unrecognized widget + `unknown` + `custom_widgets`) |

Markup extensions are split structurally (balanced braces/quotes, depth ≤ 8) and
never looked up or run; `{}` prefix is a literal escape. Inert-by-construction
syntax (`x:Null`, `x:Type`, `x:Static`, `RelativeSource`) is retained as evidence
only. Resource-only roots (`ResourceDictionary`, `Application`) have no screen and
are rejected as `missing_root_widget`. Limits (bytes/elements/depth), DTD/entity/PI
rejection, UTF-8/NUL checks and the safe file opener are the shared ones.

Known limits (PROVISIONAL, host-only, not target acceptance): elements of an unknown
namespace are treated as `custom` because non-visual elements (Style/Setter-like) cannot be
told apart without type resolution, and `x:Name` is assumed unique across the whole document
(same name in two templates → `duplicate_object_name`). See the PROVISIONAL section below.

Not implemented: type/base-class resolution (so inherited content properties and
event types), x:Bind path/function typing, namespace scoping, styles/triggers/visual
states, template instantiation, resource lookup across files or merged dictionaries,
x:Load/deferred loading, WPF/WinUI differences, rendering, accessibility backend,
localization catalogs (x:Uid keys are not looked up), runtime compatibility.

Fixtures (original, synthetic): `xaml_minimal.xaml`, `wpf_task_panel.xaml`,
`winui_task_panel.xaml` under `tests/developer-platform/evaluation/ui/`. Tests:
`test_xaml.py` (minimal screen, nesting, unknown elements/controls, events, bindings,
malformed syntax, byte/element/depth boundaries, DTD/entity/PI, Unicode, determinism,
input unchanged, inert sources, CLI) and the mapping tests in
`../mapping/test_mapping_xaml.py`. Host-only; not Nagi target acceptance.

### Limits of the threat model

Another process changing the input file, a parent directory or a symlink *while* the
analyzer runs (a concurrent race) is **outside the threat model**. The safe opener
checks the path at open time with no-follow handles and reads a bounded number of
bytes, but it does not defend against a hostile concurrent process and the XAML/Qt/GTK
analyses make no atomicity claim about the file on disk. The analyzer only reads its
single input and never writes. Analyze a private copy if the directory is shared.

## 仮定（PROVISIONAL）

- XAML（NDP-06d）: コントロール表 `XAML_CONTROLS`/`XAML_LAYOUTS`、データ型表 `XAML_VALUE_TYPES`、
  イベント名表 `XAML_EVENTS`、既定のcontent property表 `XAML_CONTENT_PROPERTY`、
  markup extension分類（`XAML_BINDING_EXTENSIONS`/`XAML_RESOURCE_EXTENSIONS`/`XAML_INERT_EXTENSIONS`）、
  resource属性名表は、型解決なしで置いた暫定の語彙。差し替えは各表1か所で、実行系には影響しない。
  CONFIRMEDなUI owner決定ではない。
- `x:` 接頭辞をXAML言語namespaceとして固定せず、解析は名前空間URIで行うが、markup extension内の
  `x:Bind` 等は文字列としての `x:` 接頭辞で照合する（`{xaml:Bind}` のような別名は unknown になる）。
- 名前はdocument全体で一意とみなす（template毎のnamescopeは扱わない。重複はerror）。
  NDP-07の`_validate`が名前の一意性を前提にしているため。namescope対応時に見直す。
- `Grid.Row`等の「`.` を含むattribute」はすべてplacement attributeとして `items[].attributes` に入れる
  （`AutomationProperties.*` も含む）。型情報がないため配置専用と区別しない。
- Template本文内のcontrolもwidgetとして列挙する（所有widgetはtemplateを持つ親）。
  `DataTemplate`/`ControlTemplate` 自体はunknown。
- event属性は名前表だけで判定する（型情報なし）。表にないevent名は通常のpropertyになる。
- WPFとWinUIを区別しない。`dialect_hints` は判定ではなくnamespace URI形式の証拠。
- `nested_in_unknown` は内部レポート上の追加フィールドで、公開schemaではない。
  UI ownerが別の表現（例: 親への参照ID）を望む場合はこの1か所を差し替える。
- 未知namespaceの要素は、型解決をしないため非視覚（`Style`/`Setter`等）か判定できず、
  `x:Key` を持たない限りcustomとして扱う。結果としてwidget（`recognized=false`）・`unknown`・
  `custom_widgets` に数えられ、NDP-07 mappingでは `needs_decision`（候補なし）の行になる。
  `x:Key` 付きの未知namespace要素はwidgetにせず `resources`（`resource_definition`）と `unknown` に残す。
  現状を `test_xaml.py` / `../mapping/test_mapping_xaml.py` で固定しているだけで、望ましい分類の決定ではない。
- `x:Name` はdocument全体で一意とみなす。別の `DataTemplate` 内の同名 `x:Name` は
  XAMLとして有効でも `duplicate_object_name` で失敗する（`test_xaml.py` で固定）。
  namescope対応時に見直す。
- 上記2点の固定はhostでのunittestのみで、Nagi target受入ではない。
- `template` を既知タグにしないのは、本文を解析しない間に「認識済み」に見せない
  ための保守的な判断。composite template対応を実装する時に見直す。
- テストの一時ディレクトリはOS既定（`tempfile`）を使う。ソースtreeには作らない。

Verified locally on 2026-11-17 (Python 3.9.6): the analysis discovery command
above passed, evaluation discovery passed, and
`python3 tests/developer-platform/evaluation/run_host_checks.py` passed with
three identical C/Qt repetitions and unchanged inputs. Target execution remains
unrun. Test counts are intentionally not recorded here because they go stale
(the earlier "40 tests" figure predated the NDP-06c follow-up); the current
count is whatever the runner's discovery command reports.
