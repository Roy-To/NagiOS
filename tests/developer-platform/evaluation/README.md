# NDP-10 synthetic evaluation inputs

Original, synthetic sources for PT-06: no third-party application was copied
or fetched. Existing SDK fixtures and hello-nagi are not duplicated here.
The existing [PT-06 evaluation form](../../../docs/porting-toolkit/PT06_MIGRATION_EVALUATION_TEMPLATE_2026-11-13.md)
remains the template for future comparisons; no migration benchmark has run.
Repository license terms remain unchanged.

| Input | Expected observation | Execution scope / unverified |
|---|---|---|
| portable/summary.c + summary.h | Count open/done/blocked states without OS calls; reject invalid input without changing output | Host build and summary_test.c only; not a Nagi application |
| windows/read_prefix.c | windows.h, CreateFileW, ReadFile, CloseHandle need Windows file-I/O review | Static only; no Windows compilation or file access |
| linux/watch_changes.c | sys/inotify.h, inotify_init1, inotify_add_watch, close require Linux notification/descriptor review | Static only; no Linux compilation or watch creation |
| controls/text_only.c | OS API names appear only in comments and a string | Negative control: lexical evidence is not proof of calls |
| ui/task_panel.ui | Qt widget/layout hierarchy, English/Japanese text, accessibility label, tab order, declared clicked/addTask connection | XML input only; addTask has no implementation, no rendering/input/runtime validation |
| ui/gtk_minimal.ui | Minimal GTK Builder button, Unicode label and translation attribute | Original synthetic XML only; no GTK instantiation |
| ui/gtk3_task_panel.ui | GTK 3 nested objects, GtkBox packing, accessible object, property binding and declared signal | Static source facts; AtkObject remains an unknown class; no handler or binding execution |
| ui/gtk4_task_panel.ui | GTK 4 GtkGrid child layout, accessibility declarations, binding expression and signal | Static source facts; no resource, expression, rendering or runtime validation |

These are source facts, not expected output hardcoded to a scanner. No
PT-01 interface, API replacement, public UI contract or support matrix is
invented. Qt Designer XML is one pilot input format, based on its
[documented format](https://doc.qt.io/qt-6/designer-ui-file-format.html).
The text is fixture data rather than production localization resources.
Windows and Linux have equal evaluation priority. Unknown APIs stay unknown.
Do not execute platform inputs or fetch resources referenced in UI inputs.
Only the portable C test is allowed to execute in this fixture check.

## Host check

From the repository root, keep build output outside source inputs:

```sh
fixture_output=$(mktemp -d /tmp/nagi-ndp10.XXXXXX)
clang -std=c11 -Wall -Wextra -Werror -pedantic \
  tests/developer-platform/evaluation/portable/summary.c \
  tests/developer-platform/evaluation/portable/summary_test.c \
  -o "$fixture_output/summary-test"
"$fixture_output/summary-test"
```

Tests cover empty/mixed/all-done input, invalid state after a valid prefix,
null pointers, input/output preservation and repeated results. Build scripts,
network, external secrets, Nagi SDK validation, Qt rendering and Nagi target
execution are not part of this check. Host success does not prove a port.

## Verified locally — 2026-11-15

Strict Clang build and portable boundary checks pass; two runs agree. The
text-only control passes syntax checking, the UI is well-formed XML, and
all source bytes remain unchanged. XML inspection used Python 3.9.6 and
does not satisfy the repository-wide Python 3.10 host requirement.
Platform execution, scanner integration, SDK validation, target acceptance
and DeepSeek review were not performed.

## Repeatable host evaluation — 2026-11-17

From any working directory, invoke this script by its repository path:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tests/developer-platform/evaluation/run_host_checks.py
```

Requires the existing Python standard library and `clang` on PATH. `--repeats`
accepts 2–20 (default 3). The runner builds the fixed portable C test with the
strict flags above, syntax-checks the text-only control, then repeats the C test
and NDP-06 UI CLI. It compares stdout bytes and checks widget/layout identities, tab order and the declared connection against
the fixed XML, and verifies that all seven fixture
sources and the analyzer source remain unchanged, including on command failure.
Temporary build output is removed automatically. Nonzero exits, timeouts, changed
stdout or changed source fail the evaluation. It never compiles or executes the
Windows/Linux inputs and accepts no arbitrary source or command arguments.

Stdout is a local evaluation JSON summary, not a public SDK/PT report contract.
It includes environment versions, individual subprocess wall times and medians,
UI fact counts and explicit unexecuted work. Times include process/interpreter
startup, file opening and serialization; there is no warmup exclusion. They do
not measure parser-only time, human effort, porting time or a speedup ratio.
This runner uses no AI at runtime; its implementation was developed with Codex.
Python 3.9.6 focused success does not satisfy the normal Python 3.10+ host gate.

Runner failure-path checks (subprocesses mocked for orchestration only):

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tests/developer-platform/evaluation -p 'test_*.py' -v
```

Initial real run: Clang 21 / macOS arm64 / Python 3.9.6; three repeats agree,
inputs unchanged. UI: 3 widgets, 1 layout, 2 tab stops, 1 unexecuted connection,
no unknown or unresolved references; CLI wall-time median about 34 ms.
Seven runner checks and all 18 existing UI checks pass. Two new regression
tests detect stable-but-wrong UI reports (five failing cases on the old runner). Manual/LLM comparisons,
SDK validation, rendering, platform execution and Nagi target remain unrun.

## GTK Builder as the second UI input — NDP-06b

The three GTK fixtures above were written for this task; no third-party source
was copied or fetched. Run the real GTK parser/CLI tests alongside the existing
18 Qt checks:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/nagi-developer-platform/ui/analysis -p 'test_*.py'
PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/ui/analysis/analyze.py \
  --root tests/developer-platform/evaluation/ui gtk3_task_panel.ui
```

The analyzer selects the format from the parsed XML root: Qt `ui` version 4.0
or GTK `interface`. Both use `.ui`; root detection avoids ambiguous suffixes and
preserves the existing CLI. All size/element/depth, DTD/entity/PI and safe-path
checks run through the shared input path. Signals, bindings, library and resource
references remain data. Tests verify placement changes, unknowns, Unicode,
boundary/error handling, determinism and unchanged inputs.

The supported subset and unsupported templates, class/type validation, layout
managers, resource/CSS loading, runtime bindings/signals and rendering are detailed
in the [analyzer README](../../../tools/nagi-developer-platform/ui/analysis/README.md#gtk-builder-subset--ndp-06b).
The existing evaluation runner still measures its fixed portable C/Qt fixtures;
it does not run GTK or benchmark the new inputs. GTK coverage comes from the
focused suite above. No Nagi target acceptance is claimed.

Verified locally on 2026-11-17: analysis 40/40 (Qt 18 + GTK 22), evaluation
7/7, and `run_host_checks.py` PASS (three C/Qt repetitions, inputs unchanged).
All three task-specified commands ran with `PYTHONDONTWRITEBYTECODE=1`.
