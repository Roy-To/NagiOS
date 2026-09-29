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

The acceptance uses the explicitly host-only `HostFileBackend` and a fixture
visibility policy. It proves the provider-neutral contract and reference
snapshot restart behavior; it does **not** claim guest VFS persistence,
authenticated capability enforcement, or a running Nagi Search service.

## Regressions

- M17 was rebuilt and passed `./nagi m17` on this continuation worktree. QEMU
  printed `PASS M17 first web pixel`; the trace records a nonzero Servo/Mesa
  frame reaching Nagi Surface.
- M18 passed `./nagi m18` on this continuation worktree. The QEMU log records
  HTTPS rendering for `example.com`, `example.org`, and `example.net`, and the
  CLI printed `PASS M18 Albert`.
- This M18 baseline contains no `.dev` DF-01 registry or verify command. No
  unrelated subsystem or workstream state was changed.

## Verification evidence

On the pinned aarch64 macOS Rust toolchain:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
/Users/tozawa/.cargo/bin/cargo test \
  --manifest-path user/nagi-search/Cargo.toml --locked --offline
```

Result: 16 tests passed, including the restart/search contract acceptance; no
doc tests are defined. Formatting and Clippy with `-D warnings` passed.

The isolated `no_std` Nagi user-target check also passed:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
/Users/tozawa/.cargo/bin/cargo -Z build-std=core,alloc check \
  --manifest-path user/nagi-search/Cargo.toml \
  --target targets/x86_64-unknown-nagi-user.json \
  --target-dir /tmp/nagi-m19-target --locked --offline
```

No M19 QEMU acceptance was run because the target Search service, guest
snapshot backend, trusted visibility provider, and real producer adapters are
not connected on this branch.

## Remaining acceptance blockers

1. Add a guest-owned snapshot adapter that supports bounded multi-chunk data
   and a crash-safe commit protocol. The current VFS limits a file to 1 KiB;
   the host atomic-file adapter cannot be used as guest persistence.
2. Activate Search as a user-space service with a capability-scoped storage
   handle and authenticated caller context. `AccessContext` is descriptive;
   the fixture filter is not an authority provider.
3. Resolve and persist canonical Object IDs from actual Files/page providers,
   preserving identity over rename/move/restart.
4. Add QEMU acceptance that creates guest data, indexes/searches/filters it,
   reboots using the same guest storage, and re-searches the same stable IDs.

These missing services prevent M19 PASS. They do not justify a host fallback,
an allow-all filter, or a claim that the guest Search Service is active.
