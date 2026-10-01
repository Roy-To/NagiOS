# Nagi OS M19 — Semantic Layer / Search

**Status: PARTIAL**

## Provenance and scope

This continuation is based on `44155ea04f0f5b3c34eb9804ca2029fef6094194`,
where M17 first-web-pixel and M18 three-site HTTPS/QEMU acceptance passed.
M18 remains PARTIAL for unrelated browser providers, as recorded in
`docs/implementation_status.md`.

The search foundation from `codex/m19prep-semantic-search` was selectively
reused as commit `f7b6a0b`; the prep branch itself remains unchanged. The
formal M19 search contract is deterministic and metadata-based. Embedding,
vector, and LLM retrieval are out of scope.

## Implemented and verified

- Canonical `ObjectId`, `WorkspaceId`, `AppId`, and `AppSessionId` keys are
  reused from `nagi-model`. Metadata records retain stable object identity
  across descriptive location/title updates.
- Versioned, bounded, checksummed snapshot encoding persists object metadata,
  tombstones, relations, and logical Workspace membership.
- Search supports title, filename, tags, attributes, kind/source, time ranges,
  relations, Workspace title, stable ordering, match rationale, and grouping.
- A visibility filter is mandatory. The default implementation denies all;
  filtering occurs before matching, result limits, rationale, and grouping so
  denied IDs/counts are not returned.
- Files, page, and Workspace producer adapters provide typed metadata mapping.
- `m19_acceptance_indexes_filters_restarts_and_researches_stable_objects`
  creates file/page/Workspace metadata, searches it, filters a denied object,
  checks Workspace grouping, reopens persisted metadata, updates a file under
  the same Object ID, and re-searches it.
- `GuestSnapshotBackend` now stores a bounded snapshot in two VFS-backed
  generations. It writes and flushes data chunks before the checksummed
  manifest commit, rejects snapshots over 4 KiB, and falls back to the older
  valid generation when the newer manifest or payload is corrupt. The target
  `nagi-init` adapter stores its files under `/var/lib/nagi-search`.
- The opt-in `m19-search` init feature exercises a private fixture through the
  real target VFS and SearchService. `nagi m19` also enumerates one regular
  root file from the VFS, maps its real file metadata through
  `FilesProducerAdapter`, and persists a fixture ObjectId separately from its
  inode. It renames the file, remounts the VFS, reboots QEMU with the same
  persistent user disk, and verifies the same ObjectId and updated location.
  The guest SearchService is also registered as M21 `file.search`; a bounded
  `NagiPlan@1` is parsed, validated, and executed against it, and the action
  returns only the fixture file's visible ObjectId. Its caller/capability
  policy denies a foreign fixture caller but is private test authority, not
  production authority.

## Authenticated IPC prerequisite audit — 2026-09-30

The shared M18–M23 service audit found no production caller identity at the
user-space service boundary: `ServiceRegistry` invokes in-process handlers
without a kernel-authenticated caller context, and the bootstrap does not
expose Channel send/receive syscalls. The Channel core now records the process
ID supplied by its kernel `Process` argument when enqueuing a message and
returns it separately from the untrusted header and payload. An IPC regression
test puts the receiver's ID in the payload and verifies that receive metadata
still reports the sender's kernel Process ID.

This is an M4 kernel-core improvement, not an authentication claim for M19:
the current bootstrap has one shared-address-space init process, no app/session
identity binding, and no user Channel syscall path. A trusted supervisor,
isolated processes, and capability-bound production Search IPC remain
necessary before the M19 fixture policy can be replaced by production
authority. The post-M27 storage-check regression also passed `./nagi m19` and
the three-boot `./nagi m22` QEMU flow on 2026-09-30; logs are in `out/logs/`.

The host acceptance uses the explicitly host-only `HostFileBackend` and a
fixture visibility policy. It proves the provider-neutral contract and
reference snapshot restart behavior. The QEMU acceptance separately proves
bounded guest VFS persistence for its private fixture; it does **not** claim
authenticated capability enforcement, live file/page producer integration, or
a production IPC Search Service.

## Regressions

- M17 was rebuilt and passed `./nagi m17` on this continuation worktree. QEMU
  printed `PASS M17 first web pixel`; the trace records a nonzero Servo/Mesa
  frame reaching Nagi Surface.
- M18 passed `./nagi m18` on this continuation worktree. The QEMU log records
  HTTPS rendering for `example.com`, `example.org`, and `example.net`, and the
  CLI printed `PASS M18 Albert`.
- This M18 baseline contains no `.dev` DF-01 registry or verify command. No
  unrelated subsystem or workstream state was changed.
- M19 QEMU acceptance passed on a fresh, isolated persistent user disk on
  2026-09-30. The initial boot printed `Nagi M19 initial snapshot/reopen PASS`;
  the following QEMU boot printed `Nagi M19 previous-boot snapshot PASS` and
  the live VFS file/ObjectId marker. Those original two-boot logs and bootstrap
  log are preserved under `out/evidence/pre-m28-continuation-20260930/`.
  Current integrated M19/M21 guest evidence is in
  `out/logs/m19-vfs-objectid-initial.log`.
  Earlier `m19-search-*` logs remain untouched. Failed development attempts
  are preserved under `out/evidence/m19-stale-target-dir-attempt/`,
  `out/evidence/m19-guest-failure-before-trace/`, and
  `out/evidence/m19-rename-capacity-attempt/`.
- The follow-up guest run also passed the M21 `file.search`
  Plan/Validate/Execute path against the persisted guest SearchService; serial
  evidence is in `out/logs/m19-vfs-objectid-initial.log`. Its first attempt
  rejected the fixture's own Workspace and timed out; that image, data disk,
  OVMF vars, and log are preserved in
  `out/evidence/m19-m21-action-failure-workspace-caller-20260930/`. The
  pre-run accepted artifacts are at
  `out/evidence/m19-m21-action-before-qemu-20260930/`.

## M24 persistent-index integration — 2026-10-01

The Completion Sweep now opens a separate exact semantic index from
`/var/lib/nagi-search-semantic` through the same crash-recoverable guest VFS
snapshot adapter. A deterministic fixture provider indexes one visible Servo
article, one unrelated memo, and a high-scoring ObjectId with no visible
metadata. `SearchService` returns the two visible records in stable score
order and excludes the hidden ID. The following QEMU boot reopens the snapshot
and repeats the query. This is storage and visibility-filtering evidence only;
it does not alter M19's metadata-search acceptance or claim model inference.

M19 passed `./nagi m19` across two boots, including the semantic-index restore
marker on the second boot. The 29-test `nagi-search` suite, warnings-denied
Search/CLI Clippy, changed-package formatting, and Nagi no-std target compile
pass. Images, disks, OVMF vars, and serial logs are preserved under
`out/evidence/m24-persistent-semantic-index-20261001/`.

## Verification evidence

Verification with `nightly-2025-08-01-aarch64-apple-darwin`:

- `cargo test --locked --offline -p nagi-search` — PASS, 23 tests.
- `cargo clippy --locked --offline -p nagi-search --all-targets -- -D warnings`
  — PASS.
- `cargo test --locked --offline -p nagi-cli -p nagi-search -p nagi-ai
  -p nagi-history --all-targets` — PASS, 114 CLI unit tests, 18 CLI
  integration tests, 23 Search tests, 23 AI tests, and 9 History tests.
- `cargo clippy --locked --offline -p nagi-cli --all-targets -- -D warnings
  -A unknown-lints` — PASS. The pinned Clippy predates an existing lint name
  in `tools/nagi-cli/src/image.rs`; only that unknown-lint warning was allowed.
- Changed-file `rustfmt --check` — PASS.
- `cargo -Z build-std=core,alloc,compiler_builtins check --locked --offline
  -p nagi-init --features m19-search --target
  targets/x86_64-unknown-nagi-user.json` — PASS.
- `./nagi m19` — PASS across the initial search snapshot and the following
  QEMU reboot with the same persistent user disk. The fresh run also verified
  actual VFS metadata, rename, and stable fixture ObjectId.

## Remaining acceptance blockers

1. Activate Search as a production user-space service with a capability-scoped
   storage handle and authenticated caller context. `AccessContext` is
   descriptive; the fixture filter is not an authority provider.
2. Synchronize records from production Files/page providers and define
   identity across delete/recreate and inode reuse. The target VFS currently
   reports inode generation 1, so this single-file fixture does not establish
   general identity guarantees. Search also lacks authenticated IPC exposure.

These missing integrations keep M19 `PARTIAL`. They do not justify a host
fallback, an allow-all filter, or a claim that the production Search Service
is active.

## Default-init regression repair — 2026-09-30

The first M19 integration declared `alloc` for every `nagi-init` build. The
default `./nagi image` path builds only `core`, so its target compile failed
with `E0463: can't find crate for alloc`. The crate declaration is now gated
by `m19-search`, matching the M19 allocator and feature. This preserves the
default image build while retaining `alloc` for M19 snapshots.

Verification after the repair:

- `./tests/acceptance/m0_launcher.sh` — PASS with the pinned nightly on PATH;
  this builds the default Nagi image and checks launcher exit propagation.
- `./nagi m19` — PASS; the fixture survived VFS remount and a second QEMU
  boot. Logs remain in `out/logs/m19-search-initial.log` and
  `out/logs/m19-search-restart.log`.
- CI run `36584567375` on the pre-repair M19 commit failed the Ubuntu M0
  launcher and Windows launcher exit-propagation jobs. The corrected commit
  `14918d11905220c6aa6122135361c8f40d25651d` is covered by run
  `36588249000`: Ubuntu host and Windows launcher passed, and `nagi-target`
  built init and UEFI before starting the M17 first-web-pixel regression. The
  M17 and following M18 gates were still in progress at this checkpoint.

## Completion Sweep: preserve the executed Search result for M22 — 2026-10-01

After the guest M21 `file.search` plan succeeds against the real SearchService,
M19 now returns a bounded typed event with the matched Object ID, query
summary, fixture caller context, intent, and occurrence tick. M13 passes that
event to M22, which records it as a read-only NAL1 activity with no
transaction ID. M22 verifies the same entry after restart without adding
duplicates. This connects Search execution to the existing Activity Ledger
fixture; the caller context remains unauthenticated fixture data, so this does
not activate Search as a production IPC service. M19 remains `PARTIAL`.
