# Diagnostics and Verification

Nagi's diagnostics foundation is a local, host-side developer tool. It does
not collect user analytics or upload reports. A report is written to disk
only when a command receives an explicit `--output` path.

## Commands

```sh
./nagi diagnostics [--json|--format text|json] [--scope SCOPE] [--output PATH]
./nagi verify [--scope SCOPE] [--json|--format text|json] [--output PATH]
./nagi smoke [--host-only|--vm] [--json|--format text|json] [--output PATH]
```

`verify` runs registered health checks. Its current scopes are `repository`,
`diagnostics`, `workstreams`, and `host`; a health-check provider can add a
subsystem or workstream scope without changing the report contract. Selecting
an unknown scope produces `NOT_RUN` and a nonzero exit code. Checks run to
completion after a failure so the report retains later evidence.

Host checks and guest evidence are separate. `smoke` defaults to quick host
checks. `smoke --vm` also runs the existing M1/QEMU boot acceptance path and
records it as `VM`; it does not count as a target compilation check. The
underlying acceptance command remains authoritative for its acceptance
conditions.

## Event contract

`tools/nagi-cli/src/diagnostics.rs` defines versioned structured events with
severity, subsystem and event identifiers, timestamp, optional operation ID,
failure class, source location, bounded structured fields, error chain, and
recovery hint. Human and JSON sinks serialize the same safe event view. Every
field carries a privacy class. `SENSITIVE` and `SECRET` values are redacted;
secret-looking field names and common inline credential forms are redacted as
well. Dynamic values belong in classified fields, not message templates.

Failure classes use DF-01's accepted stable vocabulary:
`SOURCE`, `BUILD`, `LINK`, `ABI`, `RUNTIME`, `BOOT`, `DEVICE`, `STORAGE`,
`GRAPHICS`, `NETWORK`, `MODEL`, `PERMISSION`, `ACCEPTANCE`, `CI_INFRA`,
`HOST_ENV`, and `UNKNOWN`.

Reports are versioned by
[`diagnostic-report.schema.json`](diagnostic-report.schema.json). The runtime
validator also enforces unique check IDs and requires the overall outcome to
match the executed check evidence. A malformed report cannot be accepted as
`PASS` by deserialization.

## Fatal-event boundary

`CrashCapture` accepts a fatal structured event, attaches a bounded recent
event context and build/component identifiers, and sends the safe record to a
caller-provided `CrashSink`. Tests use an in-memory sink. This is a portable
capture contract, not a Nagi kernel panic handler or durable guest crash store.
Target persistence can be connected when the target diagnostics/VFS boundary
is available; the host command does not pretend to provide guest persistence.

## DF-01 and other workstreams

The mainline `nagi verify` API exposes named, scoped checks and versioned JSON
results. DF-01's `.dev/workstreams` registry is not present on this branch;
the `workstreams` check reports that as `SKIPPED` and does not duplicate DF-01
state parsing. After DF-01 is integrated, its owner can register the existing
state validator as a `HealthCheck` under the `workstreams` scope. The existing
`nagi dev diagnose` remains responsible for arbitrary CI/build-log analysis;
this command reports local verification and safe system context.

This workstream does not change M17, Servo, CI acceptance criteria, Activity,
Wayback, capabilities, the App SDK, or first-party app behavior.
