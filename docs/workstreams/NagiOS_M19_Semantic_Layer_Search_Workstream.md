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
  The target filter and caller context are scoped to this acceptance fixture;
  they are not registered as production authority.

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
  the live VFS file/ObjectId marker. Logs are
  `out/logs/m19-vfs-objectid-initial.log` and
  `out/logs/m19-vfs-objectid-restart.log`; the bootstrap log is also retained.
  Earlier `m19-search-*` logs remain untouched. Failed development attempts
  are preserved under `out/evidence/m19-stale-target-dir-attempt/`,
  `out/evidence/m19-guest-failure-before-trace/`, and
  `out/evidence/m19-rename-capacity-attempt/`.

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
