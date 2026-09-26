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

- **Status:** `IN_PROGRESS`
- **Branch:** `codex/app-files`
- **Worktree:** `/Users/tozawa/Developer/NagiOS-app-files`
- **Product base:** `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9`; the branch
  also contains the committed first-party status preparation checkpoint
  `be18b0287434f1a5e351fc3ef1fa0cc4bfa2eb85`.
- **Current checkpoint:** Files domain, view model, typed Action dispatch,
  capability enforcement, in-memory provider, directory-handle-rooted host
  sandbox provider, search, hooks, localization, and host preview are
  implemented. Focused package verification passes. Nagi target desktop/provider
  integration is blocked by missing public Files runtime APIs.
- **Implemented components:** `apps/nagi-files` contains resource/location and
  operation models; FilesService and a typed FilesActionApi for the 15 stable
  Files action IDs; three-pane navigation/selection/inspector view state; scoped
  capability checks; in-memory orchestration backend; host sandbox backend with
  persistent Trash/tags, identity retention for same-filesystem rename/move,
  copy/restore/conflict handling, explicit one-use permanent-delete confirmation,
  bounded previews, and capability-rooted traversal/symlink checks; metadata Search;
  Context/Workspace/Activity/Wayback adapter hooks; en-US and ja-JP resources;
  and an interactive Japanese/English host preview.
- **Verification evidence:** 48 focused tests PASS; package Clippy with
  `-D warnings` PASS; package `cargo fmt -- --check` PASS; Japanese host preview
  listing and open smoke PASS in a disposable `/tmp` sandbox. Test/build output
  is isolated under `/tmp/nagi-files-cargo-target`.
- **Acceptance status:**

  | Criterion | Status | Evidence / boundary |
  | --- | --- | --- |
  | FILES-001 GUI operations | `PARTIAL` | Host three-pane terminal preview and core operations work; Nagi desktop GUI integration is blocked. |
  | FILES-002 Action API | `PARTIAL` | Typed local FilesActionApi dispatches the catalog through FilesService; shared target Action Registry is not connected. |
  | FILES-003 Agent Activity | `PARTIAL` | Activity adapter contract and mock tests pass; agent mutations fail closed when Activity is unavailable; no target ledger connection. |
  | FILES-004 Resource ID retention | `PASS (host)` | Same-filesystem rename/move preserve IDs in host and memory providers; copy receives a new ID. |
  | FILES-005 Trash restore | `PASS (host/mock)` | Persistent host Trash and memory Trash restore/conflict tests pass. |
  | FILES-006 permanent-delete confirmation | `PASS (host/mock)` | User-only, bound, one-use confirmation is required and tested. |
  | FILES-007 Context publish | `PASS (contract)` | Selection/current-location snapshot publishes through a typed boundary; shared Context service is not connected. |
  | FILES-008 Workspace reference | `PASS (mock)` | Adapter tests show add/remove reference does not move the resource; target Workspace service is not connected. |
  | FILES-009 Wayback restore | `PARTIAL` | Typed checkpoint boundary, affected-resource list, transaction ID, and truthful reversible hints are tested; supported-resource snapshot restore needs the unavailable Wayback runtime. |
  | FILES-010 UI-independent core tests | `PASS` | 48 package tests run without the desktop UI. |

- **Known limitations:** The host backend rejects lexical traversal, selected-root
  symlinks, checked symlink paths, and reserved metadata aliases. Operations
  below the selected root use `cap-std` directory handles; Unix metadata writes
  and Trash operations also compare the held metadata-directory identity with
  its current in-root entry. The preview runs with host-user authority and is
  not an adversarial production sandbox. The preview is a host terminal UI, not
  a Nagi GUI app.
  M7 currently exposes a root-directory VFS API, while M10 Files remains a
  static desktop panel; neither supplies a general Files provider, app-scoped
  capability service, or shared Action/Activity/Wayback/Search integration.
  Semantic/cross-provider Search is also not connected.
- **Migration notes:** Do not modify the root Cargo workspace or DF-01 registry
  for this isolated package. The package is a standalone Cargo workspace.
  Future target integration must adapt the typed provider and capability
  contracts to public Nagi runtime APIs; host/mock results are not target
  acceptance.
- **Failure classification:** No current package test/build failure. The
  remaining target work is blocked by external runtime/API dependencies:
  M7's root-only VFS, M10's static Files panel, and missing public Files,
  Action Registry, Activity, Wayback, and Search provider APIs. Host and mock
  implementation remains independently runnable.
- **Next action:** When those public runtime APIs become available, connect the
  provider and typed action/hook contracts to the Nagi desktop and services,
  then run target integration acceptance for FILES-001/002/003/009. Keep this
  work isolated from M17 and the other app workstreams.
- **Decisions:** Keep this workstream on `codex/app-files`; use typed operations
  and local contracts because shared Resource/Capability/Activity/Wayback/Search
  runtime APIs do not yet exist in this base. Keep target integration separate
  from M17 and the other first-party app branches. DF-01's `.dev` registry/state
  remains unchanged.
- **Next action:** When public target contracts are available, connect this
  provider and Action adapter without widening or bypassing the capability
  boundary. Separately harden host operations against concurrent path
  replacement if this preview is expected to run in a hostile shared sandbox.

## M-APP-06 — Notes

- **Status:** `NOT_STARTED`
- **Branch:** `codex/app-notes`
- **Worktree:** `/Users/tozawa/Developer/NagiOS-app-notes`
- **Base:** `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9`
- **Implemented components:** None; setup only.
- **Tests run / acceptance tests passed:** None.
- **Known limitations:** No Notes implementation or host preview has been
  started in this worktree.
- **Migration notes:** Keep storage behind a backend boundary until the Nagi
  Document and Storage capabilities are available.
- **Decisions:** Keep this workstream separate from M17 and the other app
  branches; use the shared Document, Object, localization, Activity, Wayback,
  and Search contracts.
- **Next action:** In its independent Codex task, read `AGENTS.md`,
  `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`,
  `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md`,
  `docs/implementation_status.md`, and
  `docs/architecture/language-architecture.md`; inspect the assigned
  worktree's status and diffs before auditing and implementing only Notes.
