# Testing and acceptance

Tests have three distinct entry points:

- `./nagi test` runs the host-compatible Cargo test suite.
- `./nagi test --acceptance ...` runs registered milestone acceptance wrappers.
- Existing commands such as `./nagi m17` remain available for direct runs.

Use the repository CLI for Acceptance runs. Their single source of truth is
[`tests/acceptance/registry.tsv`](../../tests/acceptance/registry.tsv), which
maps each Acceptance ID to a milestone, subsystem, scope, shell/PowerShell
wrapper pair, timeout, and short description.

## Local runs

```sh
./nagi test --acceptance --list
./nagi test --acceptance --host-only
./nagi test --acceptance --milestone M16
./nagi test --acceptance --subsystem servo --ci
```

Use `--target-only` to run guest/target cases. `--host-only` and
`--target-only` cannot be combined. The scope describes what the acceptance
proves: target cases exercise Nagi in QEMU even though a host script launches
them. The full command runs all registered cases and may take a long time.

`--ci` writes a JSON report to a unique file under `out/test-results/` and
adds an Actions step summary when `GITHUB_STEP_SUMMARY` is available. Pass
`--json PATH` to choose another report path. Each executed wrapper's combined
stdout/stderr is retained at `out/logs/acceptance/<run-id>/<ACCEPTANCE-ID>.log`.
Failed summaries include the command, host/target scope, failure stage, first
useful diagnostic, exit code, duration, and log path. Use `--verbose` to print
the captured output after each case.

Results use these statuses:

- `PASS`: wrapper exited successfully and did not report `FAIL`, `BLOCKED`, or
  `SKIP`.
- `FAIL`: implementation, build, link, assertion, artifact, command, or timeout
  failure.
- `BLOCKED`: required host tooling or external dependency/environment is
  unavailable.
- `SKIP`: a wrapper explicitly skipped a check; this keeps the Acceptance run
  non-green.
- `NOT RUN`: the case is in the registry but was outside the selected filters.

The JSON schema records each case's ID, milestone, subsystem, scope, status,
duration, command, exit code, failure stage, diagnostic, and log path. Cases
that were not selected remain `NOT RUN`; they are not treated as passes.

To compare a focused run with an earlier report, provide the earlier JSON:

```sh
./nagi test --acceptance --host-only --ci \
  --compare out/test-results/acceptance-<earlier-run>.json
```

A case is marked `regressed: true` when the earlier report says `PASS` and the
current selected run is not `PASS`. A filtered-out `NOT RUN` case is never
reported as a regression.

## CI tiers

- Ubuntu runs formatting, lint, build, host tests, Python diagnostic-parser
  regressions, and focused M0 host acceptance.
- Windows runs host build/tests and focused M0 PowerShell launcher acceptance.
- The target job runs the real target build and, if it reaches that gate, the
  M17 first-web-pixel Acceptance. A host-only success cannot satisfy M17.
- Each job uploads machine-readable results and available Acceptance/build
  logs even when an earlier step fails.

This PR workflow intentionally runs focused gates; `--acceptance --ci` is the
full registered Acceptance run. A `BLOCKED` or `FAIL` result fails CI. The
workflow does not convert missing or unfinished M17 evidence into a pass.

## Add an acceptance case

1. Add matching `tests/acceptance/<stem>.sh` and `<stem>.ps1` wrappers that
   preserve the underlying command's exit status and print a concise `PASS`
   line only after checking its acceptance evidence.
2. Add one row to `registry.tsv` with a unique ID, milestone, subsystem,
   `host` or `target` scope, wrapper stem, timeout in seconds, and description.
3. Keep the normative success criteria in the primary implementation spec;
   the registry maps those criteria to runnable wrappers instead of copying
   the criteria text.
4. Run `cargo test -p nagi-cli acceptance::tests` and a filtered CLI run for
   the new ID's milestone.

No M18 case is registered until its milestone has an implemented Acceptance
wrapper. A missing registered platform wrapper is reported as `BLOCKED`.
