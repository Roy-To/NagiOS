# UPD-01 — Update and Installation Foundation

## Boundary

UPD-01 is a host-testable reference library for application package staging,
install, update, uninstall, policy hooks, and interrupted-transaction recovery.
It does not install `.xapp` files into guest VFS, change the M16 Package
Service, add an installer UI, or provide Nagi kernel/runtime updates. It does
not decide the final Nagi package archive/container format.

`tools/nagi-installer` is intentionally a standalone Cargo package until the
Integration Owner accepts the registration and root-workspace proposal in
`.dev/workstreams/update-installation/`. This keeps the workstream isolated
from integration-owned root `Cargo.toml`, `Cargo.lock`, registry, and CI files.

## Package and manifest boundaries

`PackageSource` accepts either a directory tree or virtual files. Both use the
same path validation and staged-copy checks. ZIP, tar, and `.nagiapp` parsing
are not implemented; a future package reader can adapt its entries to this
source interface without changing transaction policy.

Package paths are UTF-8 relative paths with `/` separators. Absolute paths,
empty/dot/traversal components, backslashes, drive/alternate-stream syntax,
Windows device names, control characters, symlinks, special files, duplicate
destinations, and file/directory destination conflicts are rejected. File
enumeration is sorted. Each file's size, executable bit, and SHA-256 are
captured and checked again while staging, so a source that changes after plan
creation cannot silently replace different bytes of the same length. This
per-file copy check is not build provenance or an artifact fingerprint.

`AppSdkManifestAdapter` consumes the public
`nagi_sdk::app_contract::AppManifestContract` and validates it with the SDK's
own rules. `PackageMetadata` is only the installer's normalized projection of
app ID, semantic version, schema version, entrypoint, required capabilities,
and optional SBOM/license/provenance references. It is not a second persisted
manifest contract. APP-LC-01 remains authoritative; integration should keep
this adapter or replace it with an equivalent adapter over the same public
types.

## Store layout and inventory

Given a caller-selected store root, the library manages:

```text
<root>/inventory.json
<root>/.installer.lock
<root>/transactions/<transaction-id>.json
<root>/staging/<transaction-id>/payload/...
<root>/packages/<app-id>/versions/<transaction-id>/...
```

Application data is outside the installer-owned package tree (for example,
`<root>/data/<app-id>` in tests) and is never removed by uninstall. Inventory
schema v1 uses a sorted app-ID map and stores a relative active package
location, version, transaction ID, and capability declaration. Absolute host
paths are not persisted. Unknown inventory or journal schema versions fail
closed.

Install plans are read-only and capture the expected installed record, source
version, operation, package destinations, rollback location, capability
delta, and required policy checks. Execution rechecks the source and expected
installed record while holding an OS file lock; stale plans fail before any
active inventory change. Semantic-version precedence allows a fresh install
or upgrade, rejects downgrade, and rejects equal-precedence reinstall,
including versions that differ only by build metadata.

## Commit and recovery

Package files are copied into a unique staging tree and validated again before
promotion. Promotion renames that complete immutable tree under the per-app
version store. The version inventory is serialized to a temporary file,
flushed, and atomically replaced as one snapshot (`rename` on Unix-like hosts;
`MoveFileExW` with replace/write-through on Windows). Consumers use the
inventory's active location, so they never resolve a partially copied tree.

The durable journal advances through `created`, `validating`, `staged`,
`ready_to_commit`, `committing`, and `committed`; rollback adds
`rolling_back` and `rolled_back`, and illegal transitions fail. The inventory
pointer switch is the only active-state switch. An interruption before the
journal records `committed` rolls the inventory back to the prior record (or
removes a fresh install), even if the new snapshot had already been written.
The candidate tree and staging data are then removed. A committed update keeps
the previous immutable version available until a later uninstall. A committed
uninstall deactivates first, then removes all recognized versions for that
app. Unknown package-store entries are preserved and leave cleanup pending;
recovery retries cleanup. Corrupt journals are retained for diagnosis rather
than interpreted as deletion instructions.

Recovery is run explicitly through `Installer::recover` and before inventory,
planning, install, and uninstall operations. Orphan staging directories are
removed only when their names match the transaction-ID format. Cleanup
validates each path component and refuses symlink escapes. Failure injection
points cover journal creation, staging, the active inventory switch, and
uninstall cleanup. Tests verify that recovery restores a known-good update and
does not delete unrelated paths or user data.

## Policy and error boundaries

`PolicyEvaluator` is invoked before any install staging for capability changes,
SBOM, license, provenance, and future trust checks. Each check can allow,
deny, report not applicable, or return an error. Denial and validator failure
are structured errors and prevent commit. The library does not infer unknown
licenses, implement SBOM/provenance internals, define legal policy, add a
signing PKI, or grant capabilities. Capability deltas are reported to the
policy hook; they never change grants. Uninstall has a policy hook. This
host-only library cannot determine whether a guest app process is currently
in use; a future runtime adapter must supply that policy decision.

`InstallerError::kind()` exposes stable categories for invalid package or
metadata, unsafe paths, version conflicts, downgrade rejection, missing apps,
staging/commit/rollback/inventory failures, policy denial/failure, unsupported
schemas, I/O, stale plans, recovery conflicts, and injected interruptions.
Callers do not need to inspect display strings.

## Verification and integration

The standalone crate owns its lockfile and is validated offline with the
repository-pinned Rust toolchain:

```sh
cargo fmt --manifest-path tools/nagi-installer/Cargo.toml -- --check
cargo test --manifest-path tools/nagi-installer/Cargo.toml --locked --offline
cargo clippy --manifest-path tools/nagi-installer/Cargo.toml --all-targets --locked --offline -- -D warnings
cargo check --manifest-path tools/nagi-installer/Cargo.toml --all-targets --locked --offline
```

Windows target type-checking is also run from the macOS host where the target
is installed. The current workstream does not edit the shared root workspace
or CI. At integration, the owner should register the row proposed in
`.dev/workstreams/update-installation/registration-proposal.json`, add this
package to root workspace and CI host checks, retain the package-local lock
until workspace resolution is agreed, and add a focused manifest-path test
to the shared host matrix. No target M18, Servo, Mesa, relibc, or unrelated
runtime acceptance is required for this host foundation.
