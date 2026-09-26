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
    persisted IDs for durable adapters; providers can rehydrate read-only pin
    state on returned records, while user pin changes require the policy-gated
    store API. `RevisionWriteStore` completes the revision mutation contract.
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

- **Status:** `IN_PROGRESS`
- **Branch:** `codex/first-party-integration`
- **Worktree:** `/Users/tozawa/Developer/NagiOS-first-party-integration`
- **Product base:** `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9`; the branch
  also contains the committed first-party status preparation checkpoint
  `be18b0287434f1a5e351fc3ef1fa0cc4bfa2eb85`.
- **Current checkpoint:** Files domain, view model, typed Action dispatch,
  capability enforcement, in-memory provider, directory-handle-rooted host
  sandbox provider, search, hooks, localization, and host preview are
  implemented. Host list, metadata, and read operations bind authorization to
  an opened directory or file snapshot. FileEntry previews and checkpoint
  snapshots additionally bind reads to the caller's ResourceId. Operations
  recheck source identity and relevant rights after Activity/checkpoint hooks;
  Trash restore, deletion, and recovery validate the stored payload identity.
  Focused package verification passes. Nagi target desktop/provider integration
  is blocked by missing public Files runtime APIs.
- **Implemented components:** `apps/nagi-files` contains resource/location and
  operation models; FilesService and a typed FilesActionApi for the 15 stable
  Files action IDs; three-pane navigation/selection/inspector view state; scoped
  capability checks; in-memory orchestration backend; host sandbox backend with
  persistent Trash/tags, Unix and memory identity retention for same-filesystem
  rename/move, Windows volume/file identity with a path fallback when stable
  metadata is unavailable, copy/restore/conflict handling, bounded one-use
  permanent-delete confirmations with binding-safe consumption and user
  cancellation, provider-aware canonical spelling before scoped authorization
  on case-insensitive host filesystems,
  bounded previews, and capability-rooted traversal/symlink checks; metadata Search;
  Context/Workspace/Activity/Wayback adapter hooks; en-US and ja-JP resources;
  and an interactive Japanese/English host preview.
- **Verification evidence:** 66 focused tests PASS (65 library tests and one
  CLI localization test). Regressions cover case-insensitive scoped denies,
  resource creation during authorization before list/read/metadata, stale
  ResourceId reads, source replacement during checkpoint callbacks, Trash
  payload replacement before restore/permanent delete, directory replacement
  by a symlink, descendant-tag cleanup, corrupt journal paths, and
  permanent-delete recovery when either index write fails. Package host Clippy with `-D warnings`, Windows-target
  `cargo check --tests`, Windows-target Clippy with `-D warnings`, formatting,
  and `git diff --check` all PASS. Windows filesystem behavior remains
  cross-compiled, not executed; the current macOS package run used the local
  ignored Cargo target directory.
- **Acceptance status:**

  | Criterion | Status | Evidence / boundary |
  | --- | --- | --- |
  | FILES-001 GUI operations | `PARTIAL` | Host three-pane terminal preview and core operations work; Nagi desktop GUI integration is blocked. |
  | FILES-002 Action API | `PARTIAL` | Typed local FilesActionApi dispatches the catalog through FilesService; shared target Action Registry is not connected. |
  | FILES-003 Agent Activity | `PARTIAL` | Activity adapter contract and mock tests pass; agent mutations fail closed when Activity is unavailable; no target ledger connection. |
  | FILES-004 Resource ID retention | `PASS (Unix host); Windows compile verified` | Same-filesystem rename/move preserve IDs in Unix and memory providers. Windows IDs now use volume serial/file index metadata when available; the existing move/restart/Trash/restore test is type-checked on Windows but has not been executed on a Windows host. Unsupported metadata falls back to a path-derived ID. Copy receives a new ID. |
  | FILES-005 Trash restore | `PASS (host/mock)` | Persistent host Trash and memory Trash restore/conflict tests pass. |
  | FILES-006 permanent-delete confirmation | `PASS (host/mock)` | User-only, bound, one-use challenges are bounded to 64 pending entries; explicit cancellation releases a slot, replay is rejected, mismatched challenges preserve other pending slots, and the limit error has en-US/ja-JP text. |
  | FILES-007 Context publish | `PASS (contract)` | Selection/current-location snapshot publishes through a typed boundary; shared Context service is not connected. |
  | FILES-008 Workspace reference | `PASS (mock)` | Adapter tests show add/remove reference does not move the resource; target Workspace service is not connected. |
  | FILES-009 Wayback restore | `PARTIAL` | Typed checkpoint boundary, affected-resource list, transaction ID, and truthful reversible hints are tested; supported-resource snapshot restore needs the unavailable Wayback runtime. |
  | FILES-010 UI-independent core tests | `PASS` | 66 package tests run without the desktop UI (65 library and one preview CLI test). |

- **Known limitations:** The host backend rejects lexical traversal, selected-root
  symlinks, checked symlink paths, and reserved metadata aliases. Operations
  below the selected root use `cap-std` directory handles; Unix metadata writes
  and Trash operations also compare the held metadata-directory identity with
  its current in-root entry. List and metadata authorization use the checked
  directory handle; file reads authorize the resolved spelling and use the
  same checked file handle. FileEntry previews and checkpoint snapshots also
  compare that opened handle's ResourceId with the caller's ID. Operations
  recheck source IDs and rights after Activity/checkpoint callbacks, while Trash
  restore/deletion/recovery verify payload identity against the stored entry.
  A versioned permanent-delete journal records the complete affected ResourceId
  set and startup resumes interrupted confirmed deletions.
  Filesystems without stable entry identities use path-derived IDs for metadata
  but fail closed on handle-bound traversal and reads. Windows identity and
  test code cross-compile, but
  runtime regression execution still needs a Windows host. The preview runs
  with host-user authority and is not an adversarial production sandbox. The
  preview is a host terminal UI, not a Nagi GUI app.
  M7 currently exposes a root-directory VFS API, while M10 Files remains a
  static desktop panel; neither supplies a general Files provider, app-scoped
  capability service, or shared Action/Activity/Wayback/Search integration.
  Semantic/cross-provider Search is also not connected.
- **Migration notes:** Do not modify the root Cargo workspace or DF-01 registry
  for this isolated package. The package is a standalone Cargo workspace.
  Future target integration must adapt the typed provider and capability
  contracts to public Nagi runtime APIs; host/mock results are not target
  acceptance.
- **Failure classification:** No current Files package test/build failure. The
  remaining target work is blocked by external runtime/API dependencies:
  M7's root-only VFS, M10's static Files panel, and missing public Files,
  Action Registry, Activity, Wayback, and Search provider APIs. Host and mock
  implementation remains independently runnable. The full `./nagi test` host
  suite remains blocked on arm64 macOS by x86_64 syscall-register assembly in
  `user/libnagi`; no ABI, M17, or third-party changes were made.
- **Next action:** Continue reviewing independent Files host-side error and
  compatibility cases. When public runtime APIs
  become available, connect the provider and typed action/hook contracts to
  the Nagi desktop and services, then run target acceptance for
  FILES-001/002/003/009. Keep this work isolated from M17 and the other app
  workstreams.
- **Decisions:** Keep this workstream on `codex/first-party-integration`; use typed operations
  and local contracts because shared Resource/Capability/Activity/Wayback/Search
  runtime APIs do not yet exist in this base. Keep target integration separate
  from M17 and the other first-party app branches. DF-01's `.dev` registry/state
  remains unchanged.
- **Next action:** When public target contracts are available, connect this
  provider and Action adapter without widening or bypassing the capability
  boundary. For host-side continuation, inspect the boundary between
  authorization and provider I/O for concurrent path replacement, then add a
  focused regression and fix only if a reproducible scoped-access bypass exists.

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
- **Tests run / acceptance tests passed:** 28 focused integration tests pass
  with pinned `nightly-2025-08-01` on `aarch64-apple-darwin`; all-target Cargo
  check and warning-free all-target Clippy pass; terminal host preview
  quick-create/save/close/reopen/show smoke passes across two processes. The
  host store rejects symlinked revision files on latest and historical reads.
  The initial root `./nagi dev status` command cannot load the absent
  `third_party/cc-nagi/Cargo.toml`; this is an
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

## First-party integration checkpoint

- **Integration branch:** `codex/first-party-integration`
- **Worktree:** `/Users/tozawa/Developer/NagiOS-first-party-integration`
- **Common base:** `be18b0287434f1a5e351fc3ef1fa0cc4bfa2eb85`
- **Integration implementation commit:**
  `36bf5ba4ad0b1d87060a451c49aaafd66b124a81` (pushed to
  `origin/codex/first-party-integration`).
- **Source heads retained as merge ancestors:** Activity/Wayback
  `ad532b5fb4c20ca8a467d41e7e72e26054bbc602`; Files
  `b2c9ee10533aee5f31cb07b0ecc25efa51b217fa`; Notes
  `c1353ffe4d9be9699e6d6ed778661b675a329cb1`; Home/Search
  `3968b42d10e6e05d387ea658638c5165e91d3e04`.
- **Host integration:** `PASS` for the isolated integration harness. Real
  Notes/Files/Activity/Wayback providers feed Home/Search; Notes and Files
  events append to the shared host Activity ledger; Notes revisions and
  supported read-authorized Files regular-file mutations create Wayback
  checkpoints; Home uses canonical app descriptors; search returns typed
  object/event/checkpoint actions. Files folders and unsupported snapshots
  truthfully report unavailable checkpoint capture.
- **Privacy/capability and locale checks:** `PASS (host)` for pre-candidate
  Notes `Get` filtering, Files scoped provider authorization, user-scoped
  Activity/checkpoint visibility, Agent fail-closed paths without delegated
  provenance, no note-body Activity exposure, no file snapshot text in Search,
  and principal Home/Search flows in `en-US` and `ja-JP`.
- **Focused results:** Activity/Wayback 41 isolated tests; Files 48 tests;
  Notes 27 tests; Home/Search 35 library + 1 preview test; cross-app harness
  11 tests. Integrated host preview shows Notes/Files in both locale catalogs,
  real provider search, typed actions, Activity, and file-level Wayback capture.
  Warning-free all-target Clippy and formatting checks pass for the integration,
  Files, and Home/Search packages. All are host results and do not establish
  target runtime acceptance.
- **Target/runtime state:** `NOT RUN` for native launch, authenticated target
  context, OS capability/object/workspace services, target provider wiring,
  durable Activity/Wayback stores, and target snapshot restore. Continue as
  `PARTIAL` until those runtime interfaces and target acceptance exist.
- **DF-01 `cc-nagi` issue:** root Cargo's pinned source is an ignored fetch
  artifact. `./nagi fetch` materializes it from the source lock; the safe
  development launcher now routes `./nagi dev` through `tools/nagi-bootstrap`
  so verify/status/resume work without it. After the supported fetch completed,
  root `cargo test -p nagi-history --lib --locked --offline` passed all 41
  tests. No placeholder or third-party source modification was made.
- **Broader root host checks:** `./nagi test` is `FAIL` on this arm64 macOS
  host because `user/libnagi` compiles x86_64-only inline registers (`rax`,
  `rdi`, and related registers) for the arm64 host target. The isolated
  first-party package tests and root Activity tests pass; no low-level ABI or
  M17 change was made. `./nagi fmt` is also `FAIL` because the current pinned
  rustfmt reports formatting diffs across fetched Servo files; it ran in
  check-only mode and no Servo source was changed. The DF-01 state now records
  the full-workspace arm64 compile failure as a `HOST_ENV` blocker while
  retaining the first-party integration status as `PARTIAL`.
- M17 status and worktree were not changed. All four source branches and
  worktrees remain intact.

### Host CI repair checkpoint

- Source run `36224698245` failed in Ubuntu Clippy on six Activity/Wayback
  issues and in the Windows host suite on an assertion that assumed
  `development-foundation` remained the active stream. The target job was
  skipped because both host jobs failed.
- History inputs are now grouped in typed pin and restore request structs;
  the developer-state tests follow the workstream status schema and current
  registered branch. No permissions, provenance validation, target services,
  restore behavior, or M17 acceptance conditions changed.
- The repaired commit is `6cc3255c5a14bcf8a1692c05eeaaa4d5dd2ddc1e`.
  Local verification passed for all first-party suites (41 Activity/Wayback,
  48 Files, 27 Notes, 35+1 Home/Search, 11 cross-app), `nagi-cli` (68 unit +
  20 CLI), the Activity/Wayback UEFI library check, root workspace Clippy,
  standalone first-party Clippy, and the integrated host preview. Changed Rust
  files pass pinned rustfmt. Full workspace format remains blocked by
  check-only diffs in fetched Servo sources; the full root host suite remains
  blocked on arm64 by x86_64-only syscall registers in `user/libnagi`.
- CI run `36230579035` is checking the repair. This workstream remains
  `PARTIAL`; target launch and runtime integration remain **NOT RUN**. M17 is
  unchanged and M18 remains gated on M17.
