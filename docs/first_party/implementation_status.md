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

- **Status:** `PARTIAL`
- **Branch:** `codex/app-home-search`
- **Worktree:** `/Users/tozawa/Developer/NagiOS-home-search`
- **Base:** `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9`
- **Resume checkpoint:** Implemented on top of the preparation commit
  `be18b0287434f1a5e351fc3ef1fa0cc4bfa2eb85` in this dedicated worktree.
- **Implementation commit:** `c26b7b3baba25b2ae00dbbc3e1286c9156179404`.
- **Push:** PASS; implementation commit is on
  `origin/codex/app-home-search`.
- **Implemented components:** Data-driven Home app registry; read-only adapter
  to M16 `PackageService`; duplicate-safe registry composition; typed app,
  object, workspace, intent, and action contracts; shared Workspace/Object
  references; capability-filtered Home projection; provider-based Search
  coordinator; deterministic Unicode-aware ranking; provider deadlines,
  explicit cancellation, stale request suppression, result deduplication and
  limits; fixture providers; localized keyboard-accessible Home/Search host
  preview.
- **Verification:** 34 library tests PASS; 1 preview-server test PASS;
  JavaScript syntax check PASS; Clippy with `-D warnings` PASS; host HTTP smoke
  checks PASS for CSP, Japanese Home/API strings, typed file-open results,
  denied private-note filtering, cancel route, and malformed UTF-8 rejection.
  Rust formatting and `git diff --check` PASS.
- **Acceptance boundary:** Core: PASS; host backend/preview: PASS; mock
  integration: PASS. Nagi target runtime integration: NOT RUN. External
  runtime service integration: BLOCKED because the current runtime
  interfaces do not provide the general app launcher, Workspace data service,
  Search providers, or action dispatcher needed for guest integration. Files,
  Notes, Activity, Action, and Workspace preview providers remain in-memory
  fixtures and are identified as such in the preview.
- **Migration notes:** Connect future runtime services through
  `AppRegistry`, `HomeDataSource`, `SearchProvider`, and typed-action
  integration. Installed M16 packages remain explicitly unlaunchable until
  an app process launcher exists. Keep lexical search functional without AI.
- **Decisions:** Keep this workstream separate from M17 and the other app
  branches. Keep the Home/Search package in its own manifest so it does not
  modify the shared workspace manifest/lockfile. The host preview binds only
  to loopback and reads no host or guest user data. `.dev` DF-01 state and M17
  state remain unchanged.
- **Next action:** When Nagi exposes the needed runtime services, connect
  `AppRegistry`, `HomeDataSource`, real `SearchProvider` implementations, and
  the typed-action dispatcher through these boundaries. Until then, keep the
  fixture providers clearly labeled and do not change M17 or other workstreams.

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
