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

`crates/nagi-diagnostics` is the shared, host-testable diagnostics contract
used by the developer CLI and available to first-party services. Its
versioned events carry severity, subsystem, stable event code, localization
message ID, timestamp, correlation/session/operation IDs, component, failure
class, source location, bounded structured fields, error chain, and recovery
hint. Event codes identify machine events; message IDs resolve through the
shared localization catalogs, with English-safe fallback text kept separate.

Fields carry a privacy class. `SENSITIVE` and `SECRET` values are redacted;
credential-like names, common inline credential forms, and path-like field
names are redacted as well. Absolute source paths collapse to the filename.
Unquoted `Authorization: Basic <credential>` and Bearer values redact the
whole scheme/payload pair, including mixed-case schemes and whitespace
separators. Quoted credential values follow the existing quoted-value rule.
Buffers store only sanitized event records. Dynamic values belong in
classified fields, not message templates.

`MemorySink` is a bounded test/recent-event sink. `DevelopmentSink` provides
human-readable console output. `EventBuffer::record` performs only bounded
in-memory work; sink flushing is explicit, catches sink failures/panics, and
retains events that could not be written. It is intended to be drained away
from critical paths.

Failure classes use DF-01's accepted stable vocabulary:
`SOURCE`, `BUILD`, `LINK`, `ABI`, `RUNTIME`, `BOOT`, `DEVICE`, `STORAGE`,
`GRAPHICS`, `NETWORK`, `MODEL`, `PERMISSION`, `ACCEPTANCE`, `CI_INFRA`,
`HOST_ENV`, and `UNKNOWN`.

The CLI report schema is
[`diagnostic-report.schema.json`](diagnostic-report.schema.json). Portable
developer snapshots and error reports use
[`diagnostic-snapshot.schema.json`](diagnostic-snapshot.schema.json). The
runtime validator compiles and exercises both Draft 2020-12 schemas before
reporting the contract check as `PASS`. It also enforces unique check IDs and
requires the overall outcome to match the executed check evidence. A malformed
report cannot be accepted as `PASS` by deserialization.

`HealthRegistry` tracks `HEALTHY`, `DEGRADED`, `UNAVAILABLE`, and `UNKNOWN`
states per subsystem with stable reason codes and transition timestamps.
Registration handles make stale owners unable to update a removed/replaced
record. Its health summary is distinct from the CLI `HealthCheckRegistry`,
which runs verification procedures.

## Fatal-event boundary

`CrashCapture` accepts a fatal structured event and queues a bounded sanitized
record with recent context and build/component identifiers. Persistence is
explicit through `flush_pending`; sink failures and panics are contained and
records remain available for retry. `ErrorReport` carries process/component,
failure category, stable error code, correlation, build metadata, safe context,
and an optional opaque backtrace reference. These are portable contracts, not
a Nagi kernel panic handler or durable guest crash store. Target persistence
can be connected when the target diagnostics/VFS boundary is available.

`DiagnosticSnapshot` exposes build metadata, a current health summary and
records, explicitly enabled components, classified/sanitized environment
fields, recent safe events, and bounded-drop counts. There is no automatic
environment capture or report upload.

`ActivityBridge` accepts only typed semantic candidates for AI file changes,
application launches, and Wayback restores. A caller must explicitly promote
one of those candidates through an owner-provided sink. Raw diagnostic events
have no promotion adapter and are not automatically copied to the Activity
Ledger.

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

## Basic authorization regression (host only)

The 2026-10-23 follow-up fixes the Basic credential payload left after the
previous sanitizer redacted only the scheme token. The contract follows the
0.2 implementation specification, section 17: credential payloads must not be
copied into diagnostics by default. The synthetic regression covers human and
JSON output, the stored safe view, public fields, recovery hints, error causes,
and serialization round trips. It covers colon/equal separators, mixed case,
tab/Unicode whitespace, and JSON-quoted headers. It adds no header collection.

For dependency-isolated host validation without fetching the parent workspace's
Servo patches, an ignored `target/diag-host-harness/Cargo.toml` points its library
directly at the production source (no source copy):

```toml
[package]
name = "nagi-diagnostics-host-regression"
version = "0.0.0"
edition = "2021"
[workspace]
[lib]
path = "../../crates/nagi-diagnostics/src/lib.rs"
[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

Use the installed pinned arm64 nightly compiler and Cargo together on arm64
macOS, the official SSD launcher, a dedicated `CARGO_TARGET_DIR`, and jobs=1:

```sh
cargo test --manifest-path target/diag-host-harness/Cargo.toml --offline -j 1
cargo clippy --manifest-path target/diag-host-harness/Cargo.toml --offline --locked --all-targets -j 1 -- -D warnings
rustfmt --edition 2021 --check crates/nagi-diagnostics/src/lib.rs
```

The focused regression failed before the fix (credential payload exported).
After the fix all 28 production-module unit tests and warning-denied Clippy
passed. The isolated harness resolved 11 cached packages offline; the production
workspace lockfile was unchanged. Full CLI/workspace, guest/VM, target, and
cross-platform execution were not run. Existing tests cover Bearer behavior;
this follow-up does not define additional authentication schemes.
