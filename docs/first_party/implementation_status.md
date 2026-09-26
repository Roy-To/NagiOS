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

- **Status:** `PARTIAL`
- **Branch:** `codex/app-activity-wayback`
- **Worktree:** `/Users/tozawa/Developer/NagiOS-activity-wayback`
- **Base:** `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9`
- **Implemented components:**
  - Additive, bounded append-only Activity events with actor/provenance,
    intent/authority references, action-group/transaction/correlation/causal
    links, device/workspace context, reversibility, redacted scalar metadata,
    and deterministic time-ordered filtering.
  - Producer/read/search/store contracts (`ActivitySink`, `ActivityReadStore`,
    `ActivityStore`, `TemporalActivitySearch`) plus a bounded in-memory host
    implementation; durable adapters can rebuild immutable events with
    persisted IDs through `ActivityDraft::into_event`. M15 `HistoryService`
    and persisted operation codes remain unchanged.
  - Separate semantic Undo planning/execution with policy and stale-revision
    checks; a test-only in-memory revision backend.
  - Checkpoint and revision records that reference a separate snapshot backend,
    permission-filtered queries, authorized/versioned open contracts, summary-
    only diff provider, audited pin/unpin, restore preview/confirmation,
    partial restore accounting, restore-as-copy ObjectId results, and restore
    Activity records. Checkpoint and revision drafts materialize records with
    persisted IDs for durable adapters; checkpoint drafts also rehydrate pin
    state, and `RevisionWriteStore` completes the revision mutation contract.
    Checkpoint validity remains trusted-provider metadata and cannot be changed
    on a public record copy. In-place restore captures and records the
    pre-restore state as an automatic recovery checkpoint before applying the
    target state; the sandbox test restores from that recovery checkpoint to
    prove the restore itself can be reversed.
  - Localized `en-US`/`ja-JP` Activity and Wayback rendering contracts,
    visibility-filtered timelines/details, action-group/transaction views,
    target-open capability contract, checkpoint markers, diff/restore views,
    and an executable host sandbox preview example.
- Activity drafts start as `Pending`; producers must explicitly record terminal
  success/failure. Public single-record reads use visibility-filtered methods;
  unfiltered ledger/store reads are crate-private. Service adapters must source
  viewer identity and policy from authenticated context.
- **Focused verification:**
  - `nagi-history` isolated-copy suite: **41 passed**, 0 failed; doctests: 0.
  - `x86_64-unknown-uefi` library check with default features off: **PASS**.
  - `x86_64-unknown-uefi` library check with `sandbox` feature: **PASS**.
  - `activity_wayback_preview` host example: **PASS**; output explicitly labels
    the in-memory sandbox and says it is not Nagi target restore.
  - These focused commands ran on an isolated copy of the workstream crate
    because Cargo in the repository cannot resolve its absent local patch
    source `third_party/cc-nagi/Cargo.toml`. The normal repository invocation
    and DF-01 `./nagi dev` commands therefore remain **BLOCKED** by that missing
    dependency; no result is attributed to target runtime execution.
- **Acceptance state:** Typed model, bounded host ledger/query, privacy
  filtering, Undo, checkpoint/revision contracts, localized framework-neutral
  UI, restore preview, reversible in-place host sandbox restore, restore-as-copy
  references, and failure/partial accounting: **PASS** in focused tests. Nagi
  target Activity persistence, target checkpoint/version storage, target
  open/restore, authenticated service policy integration, and a native
  first-party Activity/Wayback app surface: **NOT RUN**.
- **Known limitations:** The fixed-capacity ledger/store implementations are
  host/test backends, not durable target persistence. No Nagi snapshot/version
  backend is connected. Visibility APIs require the trusted service to supply
  authenticated actor and policy values; these framework-neutral contracts do
  not implement OS capability authorization themselves. The presentation
  layer is framework-neutral; the repository has no packaged first-party
  Activity app/event bridge to wire it into, so the host example is the
  available preview. The adapter contracts and record materializers are
  implemented, but no persistent target provider currently consumes them.
- **Migration notes:** Keep the M15 history API and storage format intact.
  App producers emit through `ActivitySink`; durable service implementations
  can implement the store/read contracts. Do not copy sibling Notes/Files/Home
  internals into this workstream.
- **Decisions:** Keep this workstream separate from M17 and sibling app
  branches. Activity records contain references and typed metadata, never
  arbitrary document bytes, credentials, or model reasoning. Restore preview
  is separate from execution and requires permission plus explicit
  acknowledgement that external side effects are not reversed. A confirmed
  in-place restore must preserve a recovery checkpoint and retain capacity for
  both checkpoint and restore Activity records before any target-state change.
- **DF-01 / resume:** This subsection is the persistent M-APP-02 workstream
  record; `.dev/workstreams.json` covers the separate Nagi 0.2 foundation
  streams. Resume on the existing branch/worktree above. Next action: connect
  these contracts to the authenticated Nagi Activity/checkpoint service and
  target snapshot provider, then wire the native app surface and run its target
  acceptance path.

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
