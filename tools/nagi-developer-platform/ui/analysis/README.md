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

## 仮定（PROVISIONAL）

- `nested_in_unknown` は内部レポート上の追加フィールドで、公開schemaではない。
  UI ownerが別の表現（例: 親への参照ID）を望む場合はこの1か所を差し替える。
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
