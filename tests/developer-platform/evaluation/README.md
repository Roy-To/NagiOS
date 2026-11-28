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
| mixed/read_and_watch.c | PROVISIONAL (NDP-A017): one source with an `#ifdef _WIN32` Windows part (windows.h, CreateFileW, ReadFile, an unknown `NtFancyFlush`, CloseHandle) and a Linux part (sys/inotify.h, inotify_init1, inotify_add_watch, an unknown `fancy_notify_create`, close) so the migration report lists both platforms and the unresolved lines | Static only; read by the validation `report.py --fixture mixed` through handwritten snapshots `api/mixed/read_and_watch.{windows,linux}.report-v1.json`; no compilation, not scanner output |
| app_windows/ (README only) | PROVISIONAL (NDP-A018): a small Windows-flavoured project made of `windows/read_prefix.c` and `ui/wpf_task_panel.xaml`, so the migration report shows API gaps and UI row/axis gaps together | Static only; routed by `report.py --fixture app_windows`, nothing copied, no compilation, XAML not loaded; Stage 3 `not_run` |
| ui_unsupported/panel_layout.xml | PROVISIONAL (NDP-A019): a three-element self-made UI layout whose XML root is none of Qt/GTK/XAML, so the migration report shows `unsupported_format` (analysis `unsupported`, mapping `not_run`) | Static only; routed by `report.py --fixture ui_unsupported`; no new format analysed, nothing rendered; Stage 3 `not_run` |
| ui_invalid/panel_layout.qml | PROVISIONAL (NDP-A021): a four-line self-made QML-like text that is not XML, so the migration report shows `invalid_xml` (analysis `failed`, mapping `not_run`, CLI exit 1) next to the `unsupported` case above | Static only; routed by `report.py --fixture ui_invalid`; not loaded by Qt/QML, nothing rendered; Stage 3 `not_run` |
| controls/text_only.c | OS API names appear only in comments and a string | Negative control: lexical evidence is not proof of calls |
| ui/task_panel.ui | Qt widget/layout hierarchy, English/Japanese text, accessibility label, tab order, declared clicked/addTask connection | XML input only; addTask has no implementation, no rendering/input/runtime validation |
| ui/gtk_minimal.ui | Minimal GTK Builder button, Unicode label and translation attribute | Original synthetic XML only; no GTK instantiation |
| ui/gtk3_task_panel.ui | GTK 3 nested objects, GtkBox packing, accessible object, property binding and declared signal | Static source facts; AtkObject remains an unknown class; no handler or binding execution |
| ui/gtk4_task_panel.ui | GTK 4 GtkGrid child layout, accessibility declarations, binding expression and signal | Static source facts; no resource, expression, rendering or runtime validation |

Migration report fixture count: 9 (`report.py --fixture all`, in this order:
`portable`, `windows`, `linux`, `ui`, `mixed`, `app_windows`, `ui_unsupported`,
`app_linux`, `ui_invalid`). This is the validation report's fixture list, not the
host-check suite size below; `test_report_ui_invalid.py` checks it against
`report.FIXTURES`.

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
the fixed XML, and verifies that all nine fixture
sources (including the GTK 3/4 task panels, see below) and the analyzer source remain unchanged, including on command failure.
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
GTK 3/4 task panels are also part of the repeated runner (NDP-11b below).
No Nagi target acceptance is claimed.

Verified locally on 2026-11-17: analysis 40/40 (Qt 18 + GTK 22), evaluation
7/7, and `run_host_checks.py` PASS (three C/Qt repetitions, inputs unchanged).
All three task-specified commands ran with `PYTHONDONTWRITEBYTECODE=1`.

## GTK in the repeated runner — NDP-11b

`run_host_checks.py` now also runs the analyzer CLI `--repeats` times on
`gtk3_task_panel.ui` and `gtk4_task_panel.ui` (JSON keys `gtk3_ui_analysis`,
`gtk4_ui_analysis`, each with its own phase name). Per input it compares stdout
bytes between runs and checks, against expectations written from the fixed XML:
format/source, fact counts, widget name/class/parent, layout class and ordered
child placement (GTK 3 packing, GTK 4 grid column/row/span), the declared
`clicked`/`on_add_task` signal (not run), the property binding or expression
binding owner, `AtkObject` as the only unknown (GTK 3), the `requires` library
version and empty tab order. Both inputs are in the source fingerprint, checked
even after a failed command. GTK is never instantiated and no handler or
binding runs.

The failure-path suite (14 checks at NDP-11b, subprocesses mocked) covers, for GTK:
empty stable reports, malformed JSON, correct counts with a wrong widget,
parent, layout class, placement value or order, receiver/slot, unknown class,
binding owner/tag and `requires` version, plus per-phase timeout, nonzero exit
and nondeterministic stdout, and GTK fixture mutation.

## XAML (WPF / WinUI) as the third UI input — NDP-06d

Three original synthetic fixtures were written for this task (no third-party source
copied or fetched); none is executed, compiled or rendered:

| Input | Expected observation | Execution scope / unverified |
|---|---|---|
| ui/xaml_minimal.xaml | A single named Button with a Unicode `Content`, `x:Uid` kept as data | Static source facts only |
| ui/wpf_task_panel.xaml | WPF Window/Grid/StackPanel hierarchy, `Grid.Row` placement, `{Binding}`, `{StaticResource}`, `{x:Reference}`, `Click` handler, `AutomationProperties.Name`, `TabIndex`, resource dictionary, `clr-namespace:` custom control | `x:Class` and the handler are not loaded or run; the custom control stays unknown |
| ui/winui_task_panel.xaml | WinUI Page/StackPanel, `{x:Bind}` (also on an event), `{ThemeResource}` with an undefined key, `DataTemplate`, `using:` custom control | No resource, template or binding is resolved or run |

The files use `.xaml` on purpose, so the existing `*.ui` fixture globs and the
repeated runner (`run_host_checks.py`, which is outside this task's scope) are
unchanged. Run the real parser/CLI tests:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/nagi-developer-platform/ui/analysis -p 'test_*.py'
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/nagi-developer-platform/ui/mapping -p 'test_*.py'
```

Verified locally on 2026-11-18 (Python 3.9): both discovery commands pass (analysis
71 tests incl. 26 XAML; mapping 41 incl. 10 XAML) and the evaluation discovery
passes (40). `run_host_checks.py` was re-run once afterwards and completed with
identical repeated stdout (it does not exercise the XAML inputs). Host-only; Nagi
target execution, rendering and runtime compatibility are not claimed. A concurrent
process modifying the input during analysis is outside the threat model (see the
analyzer README).

## API rules, generator and Stage 2 — NDP-11c

`run_host_checks.py` also repeats three trusted host tools `--repeats` times
(JSON keys `api_rules`, `generator_scaffold`, `stage2_build_test`):

- NDP-04/05 `diagnose.py` on the five handwritten report-v1 fixtures under
  `tests/developer-platform/api/`, fed by stdin. Expected (line, rule id, detection,
  matched aliases) per case are written in `API_CASES` from the fixture evidence.
  Negative controls must not claim a known API. The reports are not scanner output.
- NDP-03 `generate.py create` into a fresh temporary stand-in checkout each time
  (placeholder `sdk/rust/Cargo.toml`). Compared: stdout bytes, generated tree hashes,
  the exact five files, nothing outside the output directory, and manifest id/names/
  no capabilities. The crate is not built and the manifest is not validated here.
- NDP-09s2 `stage2.py --target portable-summary` (self-made C fixture only): both
  `compile` and `run_test` steps must pass.

The tools, report fixtures, templates and the manifest schema are in the source
fingerprint and must be unchanged afterwards, even after a failure. The suite grew in this step (real CLI output as the base for injected regressions; the
Stage 2 tests need `clang` and are skipped, not passed, without it). The current count is
stated once, under NDP-11d.

## XAML and the real nagi-pkg — NDP-11d

`run_host_checks.py` additionally repeats (`--repeats` times, default 3):

- **XAML** (`xaml_minimal_analysis`, `wpf_xaml_analysis`, `winui_xaml_analysis`): the
  analyzer CLI on the three `ui/*.xaml` fixtures. Stdout must be identical between
  runs; widgets/parents, layout children and `Grid.Row`, tab order, declared handlers,
  bindings, resources, unknown classes/elements and unresolved resource keys are
  compared with tables (`XAML_CASES`) written from the XAML text. Handlers and
  bindings must stay `not_run`. The inputs are in the source fingerprint.
- **Manifest validation** (`generated_manifest_validation`): the Level 0 scaffold is
  generated into a fresh temporary stand-in checkout and its `manifest.json` is checked
  by the existing Stage 1 adapter (`validation/validate.py`), which runs the already
  prepared `out/ndp-host/target/debug/nagi-pkg manifest validate`. The accepted report
  envelope must match exactly (status, reason, exit code, SDK contract 1.0, numeric
  app id), stdout and the generated tree must be identical between runs, the validator
  must not modify the manifest, and a negative control (a copy whose `id` is `BAD ID`
  in a separate directory) must be rejected, so an accept-everything validator or
  adapter fails the run.
- **Missing validator**: if the prepared binary is absent, a symlink or not executable,
  no process is started; the phase is reported as `{"status": "not_run", "reason":
  "validator_unavailable"}` and `sdk_validator` / `generated_manifest_validation` stay in
  the top-level `not_run` list. That is never a success. The runner does not build,
  fetch or modify `nagi-pkg`; its sha256 is recorded and checked as unchanged.

Current suite size: 65 checks (the manifest-validation class is skipped, not passed,
if the prepared validator is absent; the other tests run regardless). This number is the
unittest cases collected from `test_run_host_checks.py`, including the two that compare it
with this sentence (`ReadmeCountSyncTests`); update it when tests are added or removed.
The runner itself has no check counter.

### 仮定（PROVISIONAL）

- The negative control (`id` = `BAD ID`) and the adapter envelope keys follow the current
  `nagi-pkg` / adapter behaviour. If the owners change them, replace `validate_manifest_report`
  and the control in `check_manifest_validation` only. The XAML expectation tables describe
  the current synthetic fixtures and the analyzer's provisional vocabulary (not CONFIRMED).
- Expectations in `API_CASES`, `GENERATOR_ARGS`/`GENERATOR_FILES` and the manifest
  checks describe the current Level 0 scaffold and rules (rules are PROVISIONAL, the
  PT-01 report is the interim v1). When the owners change them, replace the data tables
  only. None of this is a CONFIRMED decision.
- The generator stand-in checkout mirrors the generator's current "SDK `Cargo.toml` must
  exist" check (same assumption as `validation/generated_check.py`).
- Stage 2 timeout is 90 s per run in this runner; the tool's own limits stay unchanged.
- Host-only evidence: no speed ratio, porting success or Nagi target acceptance is
  claimed.
- NDP-09t (task document record): the NDP-09 task document's `状態:` line was changed from
  NOT STARTED to PARTIAL and a "実績・根拠" section was added. This is a PROVISIONAL
  judgement delegated by the Core Developer to the runner on 2026-11-28; it is not an
  approved decision. Reason: Stage contract, self-made-fixture Stage 2, generated-scaffold
  Stage 1 connection and the all-fixture migration report exist on the integration branch,
  but Stage 3 (Nagi target) acceptance, the real `nagi-pkg` environment and the real PT-01
  scanner are not done, so DONE is not allowed; keeping the status below the evidence is
  small, reversible and widens no safety boundary. `test_task_doc_status.py` checks that
  the status line is PARTIAL (not NOT STARTED, not DONE), that every cited file exists
  inside the repository (after resolving `..` and symlinks; the temporary copy also refuses
  sources outside the repository and destinations outside the copy root), and that
  "Stage 3 / Nagi target acceptance 未実施" and "real `nagi-pkg` / PT-01 scanner 未実施" are
  each stated within one bullet. When the owners decide otherwise,
  replace the doc's status line and evidence section; the test's `REQUIRED_EVIDENCE` table
  is the only data to adjust.

### 限界（Limits）

- `median_ms` is the median wall time of the adapter call (the accept side) only; the
  negative control, generation and fingerprinting are not included.
- Replacing the validator binary immediately before it runs is a race the fingerprint
  cannot detect; it is outside the threat model (the sha256 is recorded, not enforced
  against a concurrent swap).
