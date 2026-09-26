# First-Party Software Workstream Status

This file tracks only the requested Nagi 0.1 first-party `M-APP-*` workstreams.
The authoritative Nagi 0.1 OS milestone state remains in
[`../implementation_status.md`](../implementation_status.md). Do not copy or
update M17 status here. These app workstreams are independent of M17 and must
not modify or merge into M17 worktrees.

## Preparation checkpoint

- Prepared from the committed, pushed DF-01 foundation base:
  `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9`.
- Preparation created four dedicated local branches/worktrees. No product
  implementation, app test suite, shared-contract integration, or M17 change
  is part of this checkpoint.
- The detailed per-workstream request is in
  `/Users/tozawa/.codex/attachments/150d2efc-b6d8-4c60-8493-2a749c535bd0/貼り付けたテキスト.txt`.
  A new task should read only its assigned section, plus the specifications
  listed below.
- This is the first-party status record required by
  `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md`. DF-01's `.dev`
  registry remains unchanged: it tracks Nagi 0.2 workstreams, whose product
  runtime activation gate is M30 PASS.

## M-APP-02 — Activity + Wayback

- **Status:** `NOT_STARTED`
- **Branch:** `codex/app-activity-wayback`
- **Worktree:** `/Users/tozawa/Developer/NagiOS-activity-wayback`
- **Base:** `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9`
- **Implemented components:** None; setup only.
- **Tests run / acceptance tests passed:** None.
- **Known limitations:** No Activity/Wayback implementation or runtime adapter
  has been started in this worktree.
- **Migration notes:** Preserve existing M15 history and connect through
  versioned contracts when runtime support is available.
- **Decisions:** Keep this workstream separate from M17 and the other app
  branches; use the existing first-party Activity, Transaction, Revision,
  Checkpoint, privacy, and restore contracts.
- **Next action:** In its independent Codex task, read `AGENTS.md`,
  `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`,
  `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md`,
  `docs/implementation_status.md`, and inspect the assigned worktree's status
  and diffs before auditing and implementing only Activity/Wayback.

## M-APP-04 + M-APP-07 — Home + Search

- **Status:** `NOT_STARTED`
- **Branch:** `codex/app-home-search`
- **Worktree:** `/Users/tozawa/Developer/NagiOS-home-search`
- **Base:** `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9`
- **Implemented components:** None; setup only.
- **Tests run / acceptance tests passed:** None.
- **Known limitations:** No Home or Search implementation has been started in
  this worktree.
- **Migration notes:** Reuse shared Search and Workspace contracts; keep
  lexical search functional without AI or semantic providers.
- **Decisions:** Keep this workstream separate from M17 and the other app
  branches; follow the first-party Search and Home acceptance criteria.
- **Next action:** In its independent Codex task, read `AGENTS.md`,
  `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`,
  `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md`,
  `docs/implementation_status.md`, and
  `docs/architecture/language-architecture.md`; inspect the assigned
  worktree's status and diffs before auditing and implementing only Home/Search.

## M-APP-05 — Files

- **Status:** `NOT_STARTED`
- **Branch:** `codex/app-files`
- **Worktree:** `/Users/tozawa/Developer/NagiOS-app-files`
- **Base:** `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9`
- **Implemented components:** None; setup only.
- **Tests run / acceptance tests passed:** None.
- **Known limitations:** No Files implementation or filesystem adapter has
  been started in this worktree.
- **Migration notes:** Keep host preview confined to a sandbox backend; do
  not represent it as Nagi production filesystem capability.
- **Decisions:** Keep this workstream separate from M17 and the other app
  branches; use typed operations and the existing Resource, Capability,
  Activity, Wayback, and Search contracts.
- **Next action:** In its independent Codex task, read `AGENTS.md`,
  `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`,
  `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md`,
  `docs/implementation_status.md`, and
  `docs/architecture/language-architecture.md`; inspect the assigned
  worktree's status and diffs before auditing and implementing only Files.

## M-APP-06 — Notes

- **Status:** `IN_PROGRESS`
- **Branch:** `codex/app-notes`
- **Worktree:** `/Users/tozawa/Developer/NagiOS-app-notes`
- **Base:** `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9`
- **Preparation commit:** `be18b0287434f1a5e351fc3ef1fa0cc4bfa2eb85`.
- **Implementation checkpoint:** `690b1a6855cc8e69cb815c815a015e429e3ca993`
  is pushed to `origin/codex/app-notes`.
- **Implemented components:** Notes-only Cargo package; ObjectId-based note
  and block model; 10 Markdown block/reference kinds; front matter and stable
  block identities; live editor sessions; debounced, coalesced autosave with
  dirty-state retention and retry; optimistic immutable revisions; in-memory
  and explicitly sandboxed host stores; Quick Note capture; trash/recovery,
  revision restore and restore-as-copy; app-bound search records/hits;
  typed Activity events with User/Agent/Mixed provenance; policy-injected
  action executor with search and reference target authorization; English and
  Japanese catalogs; and an interactive terminal host preview.
- **Acceptance evidence:** `NOTES-001` through `NOTES-004`, `NOTES-006`,
  `NOTES-007`, `NOTES-010` through `NOTES-014` pass the Notes core/host suite.
  `NOTES-005` is `PARTIAL`: typed Albert page/selection references roundtrip
  in host persistence, but no Albert caller is connected. `NOTES-008` passes
  at the typed Notes Activity boundary, and `NOTES-009` passes with local
  immutable revision restore; shared Activity/Wayback service connections
  remain pending.
- **Tests run / acceptance tests passed:** 27 focused integration tests pass
  with pinned `nightly-2025-08-01` on `aarch64-apple-darwin`; all-target Cargo
  check and warning-free all-target Clippy pass; terminal host preview
  quick-create/save/close/reopen/show smoke passes across two processes. The
  initial root `./nagi dev status`
  command cannot load the absent `third_party/cc-nagi/Cargo.toml`; this is an
  unrelated workspace/dependency issue. No target runtime test has run.
- **Known limitations:** There is no Notes native desktop surface in the
  repository's available UI framework; the runnable UI is a host terminal
  preview. Nagi Document/Storage capability and application permission
  adapters are not available on this base. Search, Activity/Wayback and
  Albert remain adapter contracts, with no shared-service or Albert runtime
  connection. `HostPreviewStore` and `InMemoryNoteStore` are not target
  persistence and are reported only as host/mock verification.
- **Migration notes:** Replace the host store with the Nagi Document/Storage
  capability adapter when that runtime contract is available. Connect the
  existing provider traits and mandatory action policy to the shared Search,
  Activity/Wayback and capability services; integrate the typed Albert
  reference input through Albert's page/selection adapter. Do not merge M17 or
  other app branch implementations into this worktree to do so.
- **Decisions:** Keep IDs on the existing `nagi_model::ObjectId` and
  `WorkspaceId` types. Keep Notes isolated from the in-progress M17 and the
  other M-APP branches. The `.dev` registry is the Nagi 0.2 development
  foundation registry, gated on M30; Notes M-APP state is recorded here rather
  than misregistering a 0.1 app milestone in that registry. Host revisions
  use immutable Markdown entries and create-only atomic installation so a
  stale writer cannot replace an existing revision.
- **Next action:** Resume when the Nagi Document/Storage capability, native
  Notes surface, shared Search/Activity/Wayback adapters, or Albert reference
  caller is available; connect those through the existing Notes interfaces
  and run the target acceptance checks. Until then, Notes core and host
  verification are independently usable. Keep M17 and sibling app workstreams
  separate.
