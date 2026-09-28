# M19-PREP Semantic Search Workstream Status

**Status:** M19-PREP PASS (independent foundation only; no formal M19 status change)

## Provenance

- Fixed M17 PASS base: `94e9a027618182b10c0ac2315e94673543f22423`
- Dedicated branch: `codex/m19prep-semantic-search`
- Worktree: `/Users/tozawa/.codex/worktrees/m19prep-semantic-search/NagiOS`
- Initial HEAD and merge-base: `94e9a027618182b10c0ac2315e94673543f22423`
- Baseline M17 acceptance execution head: `31bf815b7230f2658f654643e6d6c898d9881d77`
- Pushed implementation and acceptance checkpoint: `e7a3ab43780d247f8cacc16f94da249c7cfe9d66`
- Remote branch: `origin/codex/m19prep-semantic-search`

## Scope and current checkpoint

This branch prepares M19 foundations independently of M18. It does not claim
M19 PASS. Embedding, vector search, models, Planner/Validator/Executor, Nagi
Bar, Servo, HTTP/HTTPS, Albert UI, and 0.2 runtime activation are excluded.

- Phase A — audit: PASS. Existing stable ID newtypes, M15 History ledger,
  M16 SDK identity exports, block VFS, security boundary, and absence of an
  existing semantic Search Service were inspected at the fixed base.
- Phase B — metadata model: PASS. `ObjectId`/`WorkspaceId` and related
  `AppId`/`AppSessionId` retain their existing types; only the four used IDs
  gain additive ordering/hash traits. Metadata uses `ObjectId`; Workspace uses
  `WorkspaceId`; location is descriptive only.
- Phase C — store: PASS. Version-1 checksummed snapshot, upsert/read/tombstone,
  validated references, corruption handling, a snapshot backend trait, and
  host-only atomic file backend are implemented.
- Phase D — relations and Workspace: PASS. Directed provenance-bearing edges,
  duplicate idempotence, bounded traversal, multi-Workspace membership,
  logical app sessions, and deterministic delete cleanup are implemented.
- Phase E — search: PASS. Title/filename, tags/attributes, kind/source,
  created/modified/observed ranges, relation query, stable sort, rationale,
  Workspace title search, owner-app filtering, and visible-result grouping are
  implemented.
- Phase F — privacy: PASS for the prep contract. Every service requires an
  injected filter; the provided safe default denies all. Tests cover denied
  objects/workspaces, object membership/session filtering, groups, query
  results, relation traversal, and missing-versus-denied identity behavior.
- Phase G — producer fixtures: PASS. Files, page, and Workspace adapters
  produce records that are indexed and searched in host tests.
- Acceptance review — PASS. All M19-PREP criteria in the attached workstream
  specification pass on the implementation checkpoint above. `docs/implementation_status.md`
  is unchanged: formal M18/M19 milestone state is not advanced by this branch.

## Backend and security assumptions

The version-1 deterministic checksummed snapshot has a `SnapshotBackend`
interface. The host-only atomic file backend is reference persistence only.
Guest storage persistence has not been implemented or claimed. The baseline
guest VFS accepts at most 1 KiB per file, so a later target adapter needs
chunked or multi-file storage.

The current 0.1 Permission Broker does not provide object-level metadata
enumeration. Search therefore requires an injected filter and provides a
deny-all implementation. `AccessContext` is descriptive; real integration
must bind filtering to trusted caller/capability state. Denied object/workspace
IDs must be indistinguishable from missing IDs.

## Verification

- `RUSTC=/Users/tozawa/.cargo/bin/rustc RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc /Users/tozawa/.cargo/bin/cargo test --manifest-path user/nagi-search/Cargo.toml --locked --offline` — PASS, 15 tests plus doc tests. Includes file-backend restart persistence for records, relations, Workspace membership, tags, attributes, and stable IDs after rename.
- `PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin RUSTC=/Users/tozawa/.cargo/bin/rustc CARGO_TARGET_DIR=user/nagi-search/target-clippy /Users/tozawa/.cargo/bin/cargo clippy --manifest-path user/nagi-search/Cargo.toml --all-targets --locked --offline -- -D warnings` — PASS.
- `/Users/tozawa/.cargo/bin/cargo fmt --manifest-path user/nagi-search/Cargo.toml -- --check` — PASS.
- `RUSTC=/Users/tozawa/.cargo/bin/rustc /Users/tozawa/.cargo/bin/cargo -Z build-std=core,alloc check --manifest-path user/nagi-search/Cargo.toml --target targets/x86_64-unknown-nagi-user.json --target-dir user/nagi-search/target-nagi --locked --offline` — PASS; builds the isolated no-std package and `nagi-model` for the Nagi user target.
- `git diff --check` — PASS.
- `git diff --cached --check` — PASS before the implementation checkpoint was
  committed; the committed worktree is clean.
- A supplementary root-workspace test attempt for `nagi-model`/`nagi-history` could not resolve the fixed-base root patch path `third_party/cc-nagi/Cargo.toml` (not present/materialized at this baseline). The isolated package tests compile the shared model dependency; M15/M16 code was not modified. No QEMU or guest persistence claim is made.

## Target integration boundary and next work after M18

This crate remains outside the root workspace and target service graph. The
Nagi user target `no_std` compile passes, but no service activation, capability
broker binding, guest persistence, live Files/page producer connection, or
QEMU acceptance was exercised. Host persistence proves only the reference
backend. The guest VFS is block-backed and its current file payload limit is
1 KiB, so the target backend needs bounded chunked/multi-file snapshots or a
separately reviewed storage change.

After M18 is formally complete, the next work is to:

1. Reconcile this branch's isolated crate and additive shared-ID trait change
   with the M18-complete base, preserving this fixed-base history and avoiding
   a blind merge to `main`.
2. Add the search service entry point to the target user-space service graph
   and bind `VisibilityFilter` to trusted caller/capability state.
3. Implement the guest `SnapshotBackend` over approved Nagi storage, including
   restart/corruption tests that respect the current 1 KiB file limit.
4. Connect real Files and page/History producers to the existing adapters and
   verify stable `ObjectId` behavior across rename/update.
5. Run formal M19 host and target acceptance, then update
   `docs/implementation_status.md` only from that evidence.
