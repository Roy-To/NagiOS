# NDP-06 Qt Designer XML inspection pilot

This local host tool reads one explicit UTF-8 Qt Designer 4.0 XML file and
returns source facts. Qt Designer XML was chosen because the NDP-10 synthetic
input already exercises hierarchy, layout, text, accessibility metadata, tab
order and declared events in a small, dependency-free case. The existing
[UI contract](../../../../docs/architecture/ui-design-system.md) remains owned
by Nagi UI; this tool does not map to, modify or implement that contract.

Scope: tools/nagi-developer-platform/ui/analysis/**. Base fixture commit:
715d308. This is a local NDP-06 branch stacked on NDP-10, using one writer in
a dedicated worktree following the user's explicit workspace authorization.
Shared registry, SDK, CLI, Cargo, CI and product decisions are not modified.

## Run

From the repository root:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/ui/analysis/analyze.py \
  --root tests/developer-platform/evaluation/ui task_panel.ui
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

16 real-parser/CLI tests pass on the existing Python 3.9.6: the actual NDP-10
input, Unicode/accessibility properties, hierarchy, unknown widgets/layouts,
bindings/scripts as data, unexecuted external resources, unresolved references,
malformed forms, duplicate identities, invalid encodings, XML declaration
attacks, byte/element/depth bounds, symlinks/traversal/FIFO rejection, bounded
file reading, deterministic results, input preservation and sanitized errors.
No dependencies were installed. This focused result does not make the normal
Python 3.10+ Nagi host or cargo-based doctor pass. Target execution and the
independent review required by DEC-051 are not performed.
