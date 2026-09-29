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
  real target VFS and SearchService. `nagi m19` boots QEMU with one persistent
  user disk, verifies the fixture after a guest remount, reboots QEMU, then
  verifies the same ObjectId and Workspace again. The target filter is scoped
  to this acceptance fixture and is not registered as production authority.

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
- M19 QEMU acceptance passed with a persistent user disk. The initial boot
  printed `Nagi M19 initial snapshot/reopen PASS`; the following boot printed
  `Nagi M19 previous-boot snapshot PASS`. Logs are preserved at
  `out/logs/m19-search-initial.log` and `out/logs/m19-search-restart.log`.

## Verification evidence

Verification with `nightly-2025-08-01-aarch64-apple-darwin`:

- `cargo test --locked --offline -p nagi-search` — PASS, 19 tests.
- `cargo clippy --locked --offline -p nagi-search --all-targets -- -D warnings`
  — PASS.
- `cargo test --locked --offline -p nagi-cli -p nagi-search` — PASS, 113 CLI
  unit tests, 18 CLI integration tests, and 19 search tests.
- `cargo clippy --locked --offline -p nagi-cli --all-targets -- -D warnings
  -A unknown-lints` — PASS. The pinned Clippy predates an existing lint name
  in `tools/nagi-cli/src/image.rs`; only that unknown-lint warning was allowed.
- Changed-file `rustfmt --check` — PASS.
- `cargo -Z build-std=core,alloc,compiler_builtins check --locked --offline
  -p nagi-init --features m19-search --target
  targets/x86_64-unknown-nagi-user.json` — PASS.
- `./nagi m19` — PASS across the initial search snapshot and the following
  QEMU reboot with the same persistent user disk.

## Remaining acceptance blockers

1. Activate Search as a production user-space service with a capability-scoped
   storage handle and authenticated caller context. `AccessContext` is
   descriptive; the fixture filter is not an authority provider.
2. Resolve and persist canonical Object IDs from actual Files/page providers,
   preserving identity over rename/move/restart. Current QEMU acceptance uses
   one fixed private fixture rather than live producers.

These missing integrations keep M19 `PARTIAL`. They do not justify a host
fallback, an allow-all filter, or a claim that the production Search Service
is active.
