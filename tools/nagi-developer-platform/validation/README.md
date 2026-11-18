# NDP-08/09 Manifest validation adapter

This host-only Stage 1 adapter invokes the existing `nagi-pkg manifest validate`
command (`validate.py`); `stage2.py` adds Stage 2 for self-made fixtures only (see
below). Neither implements the Manifest schema, builds or runs an inspected
application, creates packages, signs, installs, or grants capabilities. The canonical
SDK contract is [App Contract v1](../../../sdk/rust/docs/APP_SDK_CONTRACT.md).

## Prepare the trusted tool

From the repository root, use the repository-pinned `nightly-2025-08-01`:

```sh
cargo build --locked --manifest-path tools/nagi-pkg/Cargo.toml \
  --target-dir out/ndp-host/target --jobs 2
```

Preparation can fetch only the dependencies pinned by that tool's lockfile;
it is a separate developer action and never runs implicitly in Stage 1.
The adapter only uses `out/ndp-host/target/debug/nagi-pkg` in its own checkout.
There is no arbitrary executable, shell, Cargo command or command-template option.
The checkout, adapter and prepared binary are trusted developer tools; inspected
Manifest contents are data. A missing tool yields `unavailable` without fetching.

## Run

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/validation/validate.py \
  --root tests/developer-platform/validation minimal-third-party.json
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/nagi-developer-platform/validation -p 'test_*.py' -v
```

The input is one relative regular file below the explicitly selected directory.
No-follow directory/file descriptors reject traversal, symlinks and FIFO input;
reads are capped at the canonical loader's 1 MiB bound. The selected bytes are
copied to a private temporary `manifest.json` and passed to the real validator.
Original inputs are never rewritten. Scratch output is removed after the run.
This Unix descriptor/process-group pilot is not a native Windows runner.

Stdout is an internal JSON result. Stable reasons distinguish rejected metadata,
invalid tool output, missing tools, unreadable input, timeout, cancellation,
output overflow and I/O failure. Actual return codes are retained. Successful
output must match the actual tool's documented CLI shape; a zero exit alone is
insufficient. Raw stdout/stderr, input text, original paths and environment are
not copied into result logs. Diagnostic capture is bounded to 64 KiB total;
overflow stops the process group. The default timeout is 10 seconds (maximum
60), covering inherited output pipes even when the direct child exits early.

`validate(..., cancel=threading.Event())` supports cancellation before and during
a run; Ctrl-C is also handled by the CLI. Cleanup kills the created process
group and reaps the direct child. No cancellation triggers a later stage.

CLI exit values: 0 passed, 1 failed/overflow/I/O error, 2 unavailable,
3 not run, 124 timeout, 130 cancelled. In `validate.py`, `--stage 2` and
`--stage 3` still return `not_run` and read/run nothing (it is the Manifest
adapter only); Stage 2 runs through `stage2.py` below. Nagi image acceptance
(Stage 3) is not connected.

## Stage contract (data: `stages.json`)

`stages.json` holds the tool table and Stage contract that `stage2.py` reads. Its
tool entries are taken from the NDP-08 audit
([contract-audit.md](../../../docs/development/developer-platform/contract-audit.md)).

| Stage | Input | May run | Must not run | Results |
| --- | --- | --- | --- | --- |
| 1 static | one relative Manifest below a selected directory | `nagi-pkg manifest validate` | input build scripts/entrypoints, fetch, arbitrary commands | passed / failed / unavailable / timeout / cancelled / not_run |
| 2 host build/test | approved target id, directory and exact source/header bytes | `clang` with a fixed argument vector, then the binary built from the verified snapshot | input-specified commands, Makefile/build scripts, unlisted files, fetch, network | same as Stage 1 |
| 3 Nagi target | not defined | nothing | everything | `not_run` only; never PASS |

Stage 2 stops, without starting any process, on: a prior stage result that is not
`passed`, an unknown target, a fixture directory other than the approved one
(including symlinked paths), a cwd outside the checkout, an unreadable or
non-regular listed file, a fixture digest mismatch, invalid timeout, or an
unreadable contract. A failed compile skips the test step. `unavailable`
indicates missing `clang` or scratch space. Stage 1/2 results are independent
of Stage 3.

## Stage 2 (self-made fixtures only)

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/validation/stage2.py \
  --target portable-summary --fixture-dir tests/developer-platform/evaluation/portable
# optionally gate on Stage 1 first: --stage1-root DIR --stage1-manifest REL.json
```

The only target is `portable-summary`: `summary.c`, `summary_test.c` and
`summary.h` from `tests/developer-platform/evaluation/portable/`. The listed files
are read through the Stage 1 no-follow bounded reader. Before any subprocess
starts, Stage 2 compares the complete set of filenames and SHA-256 digests of
the captured bytes with the approval pinned in `stage2.py` from NDP-10 commit
`715d308`. A mismatch returns `not_run` with `fixture_digest_mismatch` and
zero steps executed. On a match, the same bytes go into a private temporary
directory used as the cwd and `TMPDIR` of both processes and removed afterwards.
The fixture directory is never written. The argument vector is built in
`Target.compile_command`:

```text
clang -std=c11 -Wall -Wextra -Werror -O0 -fno-color-diagnostics \
  -o summary_test summary.c summary_test.c
./summary_test        # no arguments
```

Flags can only be chosen from a closed allowlist in `stage2.py`; names must be plain
file names; the directory must be under `tests/developer-platform/evaluation/`.
Nothing from CLI arguments, the fixture directory or a Manifest becomes a command,
flag or file name, and other files there (for example a `Makefile`) are ignored.
Processes get an allowlisted environment (`PATH`, `HOME`, `DEVELOPER_DIR`,
`SDKROOT`, plus `LC_ALL=C` and scratch `TMPDIR`) so `CPATH`/`CCC_*`-style variables
and secrets do not reach clang. Defaults: compile 30 s, test 10 s (maximum 60 s
each); capture is bounded to 64 KiB per step with process-group kill on overflow or
timeout. Cancel via `cancel=threading.Event()` or Ctrl-C works in either step.
Results keep per-step status, reason and real exit code (a signal is negative) but
not raw output, source text, original or scratch paths, or environment. A passing
test needs exit 0 and stdout starting with the target's `expect_stdout_prefix`.

Tests (`test_stage2.py`) use the real `clang` for the success, compile-error,
warning-as-error, failing-test, wrong-output, timeout, output-flood and
planted-build-script cases. Tests that deliberately compile changed code verify the
baseline before applying and approving their private test edits. The rejection test
uses the shipped approval and checks that changes to any listed file start no
process. Short fake compiler scripts exercise process orchestration. The
`unittest discover` command above skips real-compiler tests when clang is absent.

## Generator output -> Stage 1 (NDP-09c)

`generated_check.py` connects the NDP-03 scaffold generator to Stage 1. It loads
`generator/generate.py` by path (read-only; the generator is unchanged), generates the
Level 0 scaffold into a private temporary checkout stand-in (only `sdk/rust/Cargo.toml`
placeholder plus `samples/`), then passes only the generated `manifest.json` to
`validate.validate` (real `nagi-pkg manifest validate`). The temporary directory is
removed by this module afterwards. The generated crate is not built, Cargo is not run,
nothing is fetched, and the scaffold is not given to Stage 2.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/validation/generated_check.py
```

The result is the Stage 1 JSON plus `check: generated_manifest`, `built: false`,
`generator` (`created`/`failed`/`unavailable`) and `generator_reason`. A generation
failure is `failed` (`generation_failed`, Stage 1 is not run); a missing `nagi-pkg` is
`unavailable`, never passed. Exit values are those of `validate.py`. Python API:
`generated_check.check_generated(inputs=None, timeout=10, cancel=None)`.

`test_generated_check.py` uses the real generator in all cases. Orchestration tests
observe (via a patched `validate.validate`) that Stage 1 receives the generated
manifest and no process starts; they are not evidence of the real validator. The test
that runs the real `nagi-pkg` is skipped with a reason when the binary is not built;
a skip is not a pass. Host result only, not Nagi target acceptance.

## 仮定（PROVISIONAL）

- Generator connection (NDP-09c): the default scaffold inputs and manifest file name are
  data in `generated_check.py` (`DEFAULT_INPUTS`, `MANIFEST_NAME`); the generator's
  `generate()` API and `GenerationError.code` are used as-is. The stand-in checkout
  mirrors the generator's current "SDK `Cargo.toml` must exist" check; if the SDK
  delivery method or that check changes, replace the stand-in in `check_generated`.
  Only Level 0 default inputs are checked; user-supplied inputs are possible via the
  Python API but not exposed on the CLI.

These are working assumptions so Stage 2 can proceed; none is a CONFIRMED decision,
and each is isolated in `stages.json` or one function so it can be replaced.

- Stage 2 scope is only the self-made portable C fixture; the target table in
  `stages.json` configures the build. A trusted adapter revision supplies the
  content approval independently of the evaluated fixture and its target table.
  CI that evaluates untrusted branches must source the adapter and its
  dependencies from a trusted revision. Updating the pin requires reviewing the
  exact new fixture bytes. Third-party and input-project builds are out of scope.
- The fixture has no Manifest, so Stage 1 is not required for it. If a Stage 1
  result is supplied it must be `passed` (`prior` argument / `--stage1-*`).
- The compiler is the first `clang` on `PATH` (a trusted developer tool, like the
  prepared `nagi-pkg`); the flag set (`-std=c11 -Wall -Wextra -Werror -O0`) is a
  guess at "strict but portable". Change `flags` in `stages.json` within the
  allowlist.
- Passing is judged by exit 0 plus the fixture's stdout prefix, not by parsing the
  test's assertions. Raw compiler output is still never returned.
- Diagnostics summary (NDP-A003, additive and unconfirmed): when the compiler ran,
  the Stage 2 result has a top-level `diagnostics` object
  `{"errors": n, "warnings": n, "items": [{"file", "line", "severity", "category"}],
  "truncated": bool}` (`passed` with no output gives zero counts and no items; a
  warning-only build lists warnings). It is built from the bounded 64 KiB compile
  capture by `summarize_diagnostics`; only `file:line:col: error|warning:` lines
  naming a listed fixture file are recognised, `file` is the fixture-relative name
  (a leading scratch directory or `./` is stripped; other paths are dropped),
  `severity` is `error` (including `fatal error`) or `warning`, and `category` is the
  `-W...` flag name or `null`. Message text, source snippets, notes, absolute paths,
  input text and environment values are never copied. `items` holds at most
  `MAX_DIAGNOSTIC_ITEMS` (16) entries and `MAX_DIAGNOSTIC_CHARS` (1024) serialised
  characters; `truncated` reports omissions while the counts still cover all
  recognised lines. Limits: only the compile step is summarised (the test binary's
  output is not); counts stop at the 64 KiB capture (an `output_limit` result is a
  partial count) and at clang's own error limit; clang message wording is not
  returned, which still costs diagnosability. Rejection and `unavailable` results
  carry no `diagnostics`; `status`, `reason` and exit codes are unchanged. Format
  and limits are constants in `stage2.py`, easy to replace. Host result only, not
  Stage 3 / Nagi target acceptance.
- Host build/test results are not Nagi target (Stage 3) acceptance, and the host
  `clang` binary does not represent the Nagi toolchain.
- "cwd" is the caller's working directory used only as an allow gate (inside the
  checkout) and as the base for a relative `--fixture-dir`; execution itself always
  happens in the private scratch directory.

## Migration report (PROVISIONAL, NDP-09r)

From the checkout root, select `portable`, `windows`, `linux`, `ui` or `all`.
JSON is the default. Markdown summarizes the same report fields on stdout.

```sh
python3 -B tools/nagi-developer-platform/validation/report.py --fixture portable
python3 -B tools/nagi-developer-platform/validation/report.py --fixture ui --format markdown
```

The format and fixture routing are **PROVISIONAL**. JSON version 1 has
`recorded_at` (UTC), `fixture`, `source_kind`, `mode`, `scope`,
`target_acceptance`, `checks`, and `evaluation`. Each check has a `status`,
stable `reason`, `input_paths` and `counts`. `counts.inputs` counts the paths
scheduled for the check. Other count keys describe the available evidence
for that check, such as diagnostic rows, UI facts or executed build steps.
The paths are relative to the checkout, except when `input_root` is
`scratch_scaffold` for the generated Manifest. The report never includes
temporary absolute paths. With the same fixture and tool environment,
`recorded_at` is the only time-dependent field.

| Status | Meaning |
| --- | --- |
| `passed` | The check completed successfully within its scope. API and UI checks can pass while reporting gaps. |
| `failed` | A selected check reported an input, tool or validation error. |
| `not_run` | A tool, input prerequisite or stage connection was unavailable. See `reason`. |
| `unsupported` | The selected fixture or UI format is outside the applicable check's scope. |

**PROVISIONAL overview.** JSON version 1 also has `overview` with `total`,
`counts` (`passed`, `failed`, `not_run`, `unsupported`, summing to `total`) and
`not_passed`, which lists every non-passed check as `name`, `status`, `reason`
in check order. Existing keys are unchanged. Markdown starts with an Overview
section listing what passed and what is unsupported or not run, with each
reason. The short English descriptions come from the `REASON_DESCRIPTIONS`
table in `report.py`. A reason missing from that table is shown as the code
only, and no explanation is guessed. The table and overview format may change
and stay inside `validation/`; they are not a contract.

**PROVISIONAL reason coverage.** `test_report.py` builds real reports for all
four fixtures (normally, with the validator forced missing and with `clang`
forced missing, so `not_run` reasons are covered on any host), collects every
reason from each check, each per-file result and each step
(`collect_reasons`), and fails if one is in neither `REASON_DESCRIPTIONS` nor
`CODE_ONLY_REASONS` (`undescribed_reasons`). `CODE_ONLY_REASONS` in `report.py`
is the intentional code-only allowlist; it is empty today. When the test fails,
add a description that matches what the code actually does, or list the code
on purpose; never guess a description. Reasons that only occur on other
failure paths (for example `invalid_template_input:app_id`) are not in the
four-fixture reports and stay code-only. In Markdown, the problem lines for
non-passed files and steps also show the table description after the reason;
an unlisted reason stays code-only.

**PROVISIONAL all-fixture matrix.** `--fixture all` runs `build_report` for
`portable`, `windows`, `linux` and `ui` and adds a fixture x check overview.
JSON has the same `version`, `approval`, `recorded_at`, `scope` and
`target_acceptance` fields (`fixture` is `all`), the unchanged per-fixture
reports under `reports`, and `summary`: `fixtures`, `checks`, `matrix`
(`matrix[fixture][check]` = `status` + `reason`), `total`, `counts` (the sum
of each fixture's `overview.counts`), `by_fixture`, `not_passed` (with
`fixture`) and `reasons`. A `reasons` entry carries the `REASON_DESCRIPTIONS`
text, or `null` for a code missing from that table. Markdown begins with the
matrix table (status and reason per cell) and the reason list, followed by
each unchanged single-fixture Markdown report. The exit code follows the
existing rule over every check of every fixture: 1 if any check `failed`,
0 for other completed runs, 2 for invalid arguments. Output is capped at
four times the single-report limit.

Limits: the matrix only aggregates host runs of the self-made fixtures. A tool
or compiler that is missing stays `not_run` and is never shown as passed. It is
not Nagi target acceptance (`target_acceptance` stays `not_run`), and the
format may change inside `validation/`; it is not a contract.

Independent checks continue after a failure. UI files each have their own
analysis and mapping result. A mapping with a failed analysis gets
`not_run` with `ui_analysis_did_not_pass`; other files still proceed.
Aggregate UI status is the first present of `failed`, `not_run`,
`unsupported` or `passed`. A generation failure skips Stage 1.
Stage 2 builds the original portable fixture and is independent of the
scaffold and its Manifest. A rejected Stage 2 input is `failed` with the
original reason and zero build or test steps executed. This includes
`fixture_digest_mismatch` for a snapshot that differs from the pinned approval.
The CLI emits the report and exits 1 if any check failed, 0 for other completed
reports, and 2 for invalid arguments.

| [PT-06 evaluation section](../../../docs/porting-toolkit/PT06_MIGRATION_EVALUATION_TEMPLATE_2026-11-13.md) | Report evidence |
| --- | --- |
| A. Input and permission scope | Bundled self-made fixture ID and relative paths. The [fixture README](../../../tests/developer-platform/evaluation/README.md) records provenance and the permitted execution scope. |
| B. Comparison conditions | `toolkit_non_ai` run. Manual, local LLM and external AI runs are `not_run`. |
| C. Work and gaps | Level 0 scaffold file count, API rows and availability counts, UI fact and mapping counts. Human effort, reuse, speedup and resource usage are unmeasured. |
| D. Verification | Stage 1 checks the scratch Manifest with the real `nagi-pkg` when prepared. Stage 2 records approved portable host compile and test results. Package/signature and all Nagi target checks are `not_run`. |
| E. Follow-up evidence | Check reasons, API rule IDs and per-file UI counts expose gaps for review. A comparison or improvement estimate requires separate runs. |

### 仮定（PROVISIONAL）

- The fixed fixture list lives in `report.py`. API diagnosis uses the existing
  handwritten report-v1 evidence snapshots under `tests/developer-platform/api`;
  these can drift from their named source files and are not fresh scanner output.
  The UI pilot reads the seven Qt Designer, GTK Builder and XAML fixtures.
- The fixture list and portable target in `stages.json` select input paths.
  Stage 2 uses the content approval in the trusted adapter before starting
  clang. The adapters reject symlinks and path escapes and leave declared
  UI resources unread. Local executables and the adapter code are trusted.
- Generator defaults come from `generated_check.py`. The output is temporary and
  removed after checking the Manifest. `validator_unavailable` means Stage 1
  was `not_run`. The scaffold is neither a source conversion nor a built app.
  The report runs offline with the available Python tools, the prepared
  `nagi-pkg` binary and the existing Stage 2 host compiler.
- The API source/evidence reader is capped at 1 MiB per file; the UI analyzer's
  existing limits apply. API/UI subprocesses have a 10 s timeout and a combined
  stdout/stderr cap of 512 KiB per invocation. API details show up to 32 rows
  with a `truncated` flag. UI entries cover at most seven paths, and each output
  format is capped at 128 KiB. Stage 1 and 2 keep their existing limits.
- UI mapping row details (PROVISIONAL, additive; existing report version 1 keys
  are unchanged). Each `ui_mapping` file result gains `gap_rows`,
  `gap_rows_total` and `gap_rows_truncated`. `gap_rows` lists the mapping rows
  whose `mapping.status` is not `supported`. The filter uses `mapping.status`
  because the row `status` is a whole-port state that `map_ui.py` never sets
  better than `partial`; filtering on it would list every row. Rows are stably
  ordered `unsupported`, `needs_decision`, `partial` (then `map_ui.py` order) and
  cut to 32 per file (`MAX_UI_DETAILS`), so truncation drops the least severe
  rows first. `gap_rows_total` is the count before the cut and
  `gap_rows_truncated` is true when it exceeds the cap. Each entry has only `id`,
  `source_kind`, `status` (row), `mapping_status` and `reason` (the row's
  `mapping.reason` code); a value that is not a plain identifier fails that file
  as `invalid_tool_output`. Widget names, text, properties and absolute paths are
  not copied. Row `status` counts bound the list: `unsupported` and
  `needs_decision` rows are listed in full (up to the cap), `partial` rows only
  when their vocabulary match is not `supported`. Markdown lists up to 8 rows per
  file (`MAX_UI_MARKDOWN_ROWS`) as `- ui_mapping <path> row <id> (<kind>) ·
  <mapping_status> · <reason>`, with a description only for codes in
  `REASON_DESCRIPTIONS`, and adds an "omitted (see JSON)" line computed from
  `gap_rows_total`. Vocabulary-supported rows are not listed even though their
  other axes (focus, accessibility, localization, backend) may still be gaps;
  those axis details stay in the `map_ui.py` output. The fields are absent when the
  mapping tool is unavailable or analysis failed for the file. Limit: the reason
  is a vocabulary classification, not a rendered or executed compatibility result,
  and the format may change inside `validation/`; it is not a contract.
- Markdown "API gaps" section (PROVISIONAL; the JSON is unchanged). When a
  `windows_diagnosis` or `linux_diagnosis` check passed, Markdown adds
  `## API gaps` after the problem lines. It lists rows from the JSON `rows`
  (so at most 32 per check, `MAX_API_DETAILS`) whose `availability` is
  `unsupported` or whose `rule_id` is null, as `- <check> line <n> · <apis> ·
  <note>`. The note is `unsupported (rule <rule_id>)` or `no rule matched
  (unresolved)`; no rule description is guessed. Each line shows up to 6 API
  names (`MAX_API_MARKDOWN_NAMES`), each cut to 80 characters, with `+N more`.
  When `truncated` is true a line `… rows beyond the first <N> are omitted (see
  JSON counts)` (N = rows kept in JSON, 32 by default) is added, even if no listed row qualifies. No section is emitted
  for fixtures with nothing to list (`portable`, `ui`). With `--fixture all` the
  section appears inside each fixture's own part. The format may change inside
  `validation/`; it is not a contract, and host diagnosis is not Nagi target
  acceptance.
- Stage 3 stays `not_run`. Rendering, runtime API behavior, install, launch,
  permissions and data persistence on Nagi require separate target evidence.

## Evidence and remaining scope — 2026-11-17

On macOS arm64, Clang 21, Python 3.9.6, pinned nightly Rust 1.90.0-nightly:
11 adapter/transport tests pass, including actual validator acceptance/rejection
and CLI invocation; the canonical `nagi-pkg` suite passes all 7 tests. Process
transport tests use controlled Python children only for orchestration cases.
The synthetic third-party Manifest has no requested capabilities, intents,
resources or background services. Its declared artifact is not launched or
checked for existence by metadata validation. Incompatible SDK version, unsafe
entry and unknown secret-bearing field are rejected without leaking values.

The Mac's Rust install/cache is isolated under ignored `.nagi-host-tools/`;
its shell configuration and global PATH were not changed. For this checkout's
isolated installation, prefix the build/test recipe with:

```sh
export CARGO_HOME="$PWD/.nagi-host-tools/cargo"
export RUSTUP_HOME="$PWD/.nagi-host-tools/rustup"
export PATH="$CARGO_HOME/bin:$PATH"
```

Rustup bootstrap 1.28.2 SHA-256:
`20ef5516c31b1ac2290084199ba77dbbcaa1406c45c1d978ca68558ef5964ef5`.
The toolchain pin remains `rust-toolchain.toml`; lockfiles and SDK/Package source
are unchanged. Python 3.10+ normal host readiness, SDK delivery-owner agreement,
Stage 2/3, generator, package/runtime integration and target acceptance are not
proven by this result.

Stage 2 evidence (2026-11-18, macOS arm64, Apple clang 21.0.0, Python 3.9.6): the
`unittest discover` run executes 41 tests, all passing; the 2 skipped are the existing
real-`nagi-pkg` tests because that binary is not built in this worktree. This is host
evidence only.
