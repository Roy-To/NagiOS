# First-Party Integration Contract Audit

This checkpoint records the contract audit performed before merging the four
source branches. The dedicated integration branch starts at common ancestor
`be18b0287434f1a5e351fc3ef1fa0cc4bfa2eb85`; the four source branches are
independent descendants of that commit. Their only overlapping changed path is
`docs/first_party/implementation_status.md`.

## Source revisions

| Workstream | Branch | SHA | Remote SHA | Relationship to base |
|---|---|---|---|---|
| Activity + Wayback | `codex/app-activity-wayback` | `ad532b5fb4c20ca8a467d41e7e72e26054bbc602` | same | descendant |
| Files | `codex/app-files` | `b2c9ee10533aee5f31cb07b0ecc25efa51b217fa` | same | descendant |
| Notes | `codex/app-notes` | `c1353ffe4d9be9699e6d6ed778661b675a329cb1` | same | descendant |
| Home + Search | `codex/app-home-search` | `3968b42d10e6e05d387ea658638c5165e91d3e04` | same | descendant |

## Contract decisions

| Contract | Existing/canonical meaning | Source differences | Integration decision |
|---|---|---|---|
| Object identity | `nagi_model::{ObjectId, AppId}`; ObjectId is environment identity, not path/inode | Notes already aliases canonical ObjectId. Activity re-exports it. Files uses `ResourceId(u128)`; Home uses canonical IDs. | Preserve canonical IDs. Files host identity must pass through an explicit resolver/adapter; never truncate a `u128` or claim a host path is a target ObjectId. |
| Workspace | `nagi_model::WorkspaceId`, device independent | Notes uses canonical WorkspaceId but also stores multiple workspace memberships. Home uses the canonical newtype. Files carries `Option<String>` context and string workspace hooks. | Preserve WorkspaceId and Notes multi-membership semantics. Map Files host workspace strings with an explicit resolver; target mapping awaits Workspace service. |
| Transaction | `nagi_model::TransactionId` | Files defines a local same-width transaction ID; Activity uses the shared ID. | Convert at the Files boundary and retain action/correlation identity. |
| Action | First-party spec requires stable IDs/schema and typed dispatch; there is no shared executable Action service in `nagi_model` | Home defines UI/search `TypedAction`; Notes and Files define domain-specific typed actions; Activity exposes semantic action categories. | Use Home's typed search activation contract at the Search boundary. Adapt to each domain's typed API. Do not treat a string action ID or UI availability as authority or execution. Add typed Activity/Wayback destinations only with their canonical IDs. |
| Capability | Platform/target authority remains an OS service boundary | Home's grants project UI availability; Notes uses an injected fail-closed action policy; Files has scoped host-preview grants; Activity reads require viewer policy. | Keep host policy distinct from target capability. Each provider filters before exposing text and activation. Missing target context remains `NOT RUN`/`BLOCKED`. |
| Search provider/result | Spec Search Record and provider are cross-app service contracts; Home owns a concurrent, ranked `SearchProvider`/`SearchResult` coordinator | Notes returns local records/hits, Files returns metadata records, Activity has temporal search, Home fixture providers are not production providers. | Adapt real package providers into Home's canonical coordinator. Preserve source identity, workspace, sensitivity and authorization. Replace fixtures only where adapters can call the actual host provider. |
| Activity | Existing M15 history remains intact; shared append-oriented Activity is in `user/nagi-history` | Notes and Files each define reduced local Activity events/sinks. Activity branch adds typed ledger events, provenance and policy-filtered reads. | Keep `nagi-history` as the host ledger contract. Convert successful semantic operations without note bodies. Agent events require valid delegated authority context; never invent missing authority. |
| Wayback/checkpoint | Checkpoint/revision references are separate from trash, undo, document revisions and system snapshots | Activity supplies typed checkpoint/revision/restore contracts. Notes has local immutable revisions but no checkpoint hook. Files has a before-operation string checkpoint hook. | Keep local Notes revisions and Files trash semantics distinct; adapt through typed checkpoint references and restore plans. Host preview remains sandbox-only. |
| App Registry | Home's `AppRegistry`/`AppId` projection and M16 `PackageService` adapter | Home preview uses a demo registry; Notes/Files have stable package AppIds but no registry registration. Activity is a framework library, not an installed app. | Feed first-party descriptors through the Home registry composition point; installed-package launch stays unavailable until the target launcher exists. Do not mark fixture descriptors as target packages. |
| Persistence | Physical files remain authoritative; history is separate; host data must be isolated | Notes has in-memory and selected-root host stores; Files has memory and directory-handle-rooted sandbox providers; Activity has fixed-capacity in-memory stores. | Preserve each provider boundary. Integrated host preview uses temporary app-owned storage. No host persistence is reported as target persistence. |
| Localization | English identifiers, `en-US` and `ja-JP` first-class, selected locale falls back to `en-US` | Home embeds catalogs; Notes/Files use `.properties`; Activity views use localized strings. Each has local resolver types. | Retain source keys and adapt presentation through each existing catalog for host integration. A shared runtime localization service is not present and remains a target dependency. |
| Cargo/dependencies | Root `Cargo.toml` has pinned path patches for fetched third-party sources | Notes, Files and Home/Search are standalone packages with own lockfiles; Activity is a root-workspace package. No source branch changes overlap in manifests. | Keep isolated package boundaries; do not add all app crates to the root workspace merely for integration. Use a focused integration harness with explicit path dependencies. |

## Clean-worktree `cc-nagi` finding

`third_party/cc-nagi` is intentionally ignored and absent from Git tracking and
submodules. Its pinned `cc` 1.4.6 source and Nagi patch are declared in
`third_party/sources.lock` and `third_party/cc-nagi-patches`; `./nagi fetch`
materializes it using the standalone `tools/nagi-bootstrap` workspace. The
ordinary `./nagi dev status|resume|verify` commands previously launched the
root Cargo workspace, whose `[patch.crates-io]` requires that generated
directory before Cargo can start. The safe tooling repair routes all `dev`
commands through the standalone bootstrap workspace. It does not change
third-party code or M17.

## Integration order

1. Activity + Wayback
2. Files
3. Notes
4. Home + Search

After merging, adapters and host integration tests will be added in the
dedicated integration branch. The source branches and worktrees are left
unchanged.

## Initial status

- Contract audit: `PASS` (read-only source and contract comparison).
- Branch ancestry and remote SHA check: `PASS`.
- `cc-nagi` clean-checkout developer-state path: `PASS`; launcher regression,
  `./nagi dev verify/status/resume`, shell syntax, and diff checks passed while
  the generated manifest remained absent.
- Provider/app integration: not started.
- Nagi target runtime integration: `NOT RUN`.

The Home/Search source worktree also contains an unrelated, pre-existing
untracked `libtarget_check.rlib`; it has been left untouched.

## Activity + Wayback merge checkpoint

- Merge commit: `600e76b3ef464ed65f6392dac138f32cd5cd2ce2`.
- Source SHA preserved as a merge parent: `ad532b5fb4c20ca8a467d41e7e72e26054bbc602`.
- Merge result: clean; no code conflicts. The source Activity implementation
  continues to use canonical `nagi_model` IDs and keeps M15 `HistoryService`
  separate.
- Focused suite: **41 passed** in an isolated copy of `nagi-history` plus
  `nagi-model`; the copy avoids the root workspace's required ignored source
  paths. Root `cargo test -p nagi-history --lib --locked --offline` was also
  attempted and stopped before compilation because `third_party/cc-nagi` had
  not been materialized.
- `x86_64-unknown-uefi` library check: **PASS**. This is compile evidence only,
  not target execution.
- `activity_wayback_preview` example: **PASS**; output labels the in-memory
  host sandbox and explicitly says it is not Nagi target restore.
- Nagi target persistence, event bridge, capability service and restore:
  **NOT RUN**.

## Files merge checkpoint

- Merge commit: `98a0d93859361e1ab8ebbb5c376d744bce36bc29`.
- Source SHA preserved as a merge parent:
  `b2c9ee10533aee5f31cb07b0ecc25efa51b217fa`.
- The shared status file merged automatically because the Activity and Files
  sections were disjoint. Files remains a standalone Cargo package with its
  own lockfile; it was not added to the root workspace.
- Focused Files suite: **48 passed** using the pinned Rust toolchain, the
  package lockfile, and an isolated Cargo target directory. The suite covered
  metadata-only search, scoped host grants, symlink/traversal rejection,
  operations, Trash/restore and checkpoint-hook failures. Its sandbox tests
  use temporary directories; no user files were read.
- Files → shared Activity and typed Wayback adapters are not connected yet.
  The branch's optional hooks remain app-local.
- Nagi target Files provider/capability service: **NOT RUN**.

## Notes merge checkpoint

- Merge commit: `27b43222d6b477c9b030ec008ba7977492162914`.
- Source SHA preserved as a merge parent:
  `c1353ffe4d9be9699e6d6ed778661b675a329cb1`.
- The shared status file merged automatically; Notes is still in its
  standalone package/workspace and keeps its own lockfile.
- Focused Notes acceptance suite: **28 passed**. This exercised host stores,
  authorization-policy failures, Activity redaction/failure handling,
  revisions/restore-as-copy, localization, and host preview path confinement,
  including rejection of symlinked revision files.
- Notes already uses canonical `ObjectId` and `WorkspaceId`. Its direct
  methods can bypass `NotesActionExecutor`, so an integrated UI/provider must
  not turn Home's UI grant projection into authority. Search indexing must
  filter authorized hits before exposing title/body snippets.
- Notes local Activity events omit content, but an Agent origin lacks the
  shared ledger's delegated authority receipt and authenticated Node/User
  context. The host adapter must accept valid execution context; target AI
  events remain blocked until the authority bridge exists.
- Note revisions remain local document versions. Wayback capture/restore
  needs an explicit adapter that records a shared checkpoint and performs the
  Notes restore through the app so the restore creates a new revision/event.
- Nagi target Notes storage/capabilities and authenticated Agent provenance:
  **NOT RUN**.

## Home + Search merge checkpoint

- Merge commit: `a35a978537e7de47c414fe1105386cec909ab997`.
- Source SHA preserved as a merge parent:
  `3968b42d10e6e05d387ea658638c5165e91d3e04`.
- All four source branch heads are now ancestors of the integration branch.
  The sole overlapping source path, `docs/first_party/implementation_status.md`,
  merged automatically because each branch changed a different app section.
- Focused Home/Search suite: **34 library tests + 1 preview test passed**.
- The merged Home/Search coordinator still uses fixture Notes, Files, and
  Activity providers. This is a source merge checkpoint, not cross-app
  provider integration. Its existing `TypedAction` has no typed Activity
  event/checkpoint destinations and has no dispatcher.
- Nagi target App Registry, Search provider service and Action dispatcher:
  **NOT RUN**.

## Integrated host checkpoint

The four branches are merged as actual history ancestors in this order:
Activity/Wayback, Files, Notes, then Home/Search. Merge commits are listed
above. Only the already-audited status document overlapped; Git resolved its
disjoint workstream sections automatically. Integration adapters live in the
separate `tests/first-party-integration` host-only package, not in the root
Cargo workspace. The implementation checkpoint is commit
`36bf5ba4ad0b1d87060a451c49aaafd66b124a81`, pushed to
`origin/codex/first-party-integration`.

### Canonical contracts and adapters

- `nagi_model::ObjectId`, `WorkspaceId`, `AppId`, and `TransactionId` remain
  the shared identity contracts. Files `ResourceId(u128)` is mapped by a
  reversible in-memory resolver; no truncation or path-derived identity is
  used. Workspace host context is currently absent and target Workspace
  resolution remains unavailable.
- Home/Search `TypedAction` remains the Search activation contract. It now
  has distinct `OpenActivityEvent(EventId)` and
  `OpenCheckpoint(CheckpointId)` actions. Actions are marked
  `HostPreviewOnly`; the host harness does not claim to dispatch target app
  launch or target open operations.
- Notes Search adapts the actual `NotesSearchProvider`, and evaluates the
  injected `Get` policy before candidate/snippet construction. Files Search
  adapts the actual `FilesSearchProvider` and `FilesService` with its scoped
  `CapabilitySet`. Activity and Wayback Search query the shared typed ledger
  and checkpoint stores after user visibility-policy checks.
- Notes user events enter the shared Activity ledger without note body text.
  Persisted revisions create canonical Wayback revision/checkpoint records
  whose backend references resolve back to `NoteStore::load_revision`.
- Files user events enter that same Activity ledger and preserve canonical
  transaction IDs, mapped targets, success/failure/denial result, and linked
  checkpoint IDs. Agent operations fail closed without delegated provenance.
  Read-authorized regular-file rename snapshots create actual in-memory
  Wayback revisions/checkpoints before mutation. Folder creation,
  multi-object/source-less operations, denied/unavailable read access, and
  snapshots above 16 MiB do not produce checkpoints. Folder creation still
  records its own successful Activity result. Checkpoint bytes remain in the
  host snapshot backend and never enter Activity or Search. Capacity is
  preflighted for both checkpoint and operation Activity events before the
  snapshot is stored.
- Home uses a canonical `RegistrySnapshot` built from stable Notes and Files
  `AppId`s. Names localize in `en-US` and `ja-JP`; the entries say `HostPreview`
  and do not claim target launch. Activity and Wayback remain framework
  services rather than fabricated installed apps.
- Host stores are memory-only and scoped to the test process. Package sandbox
  tests use temporary directories; this integration harness reads and writes
  no user data. UI-visible capability context remains an additional result and
  action filter, never the provider's authority source.

### Integrated verification

- Cross-app integration crate: **11 passed**, including real Notes/Files
  providers, canonical object resolution, Notes and Files Activity, Notes and
  Files Wayback paths, typed event/checkpoint actions, English/Japanese Home
  and Search, Files read authorization, Notes `Get` filtering, user-scoped
  Activity/Checkpoint reads, Agent fail-closed behavior, note body redaction,
  truthful folder checkpoint failure, and failed Files operation wording.
- Warning-free all-target Clippy (`-D warnings`): **PASS** for the integration,
  Files, and Home/Search packages. Rust formatting check: **PASS**.
- Files package: **48 passed** after snapshot-aware checkpoint-hook changes.
- Home/Search package: **35 library + 1 preview test passed** with the typed
  event/checkpoint actions and canonical shared ID dependency.
- Notes package: **27 passed** at the merged source revision. Its app crate
  has no integration-source changes.
- Activity/Wayback package: **41 isolated host tests passed**, and library
  checks for `x86_64-unknown-uefi` passed. The root invocation still stops
  before compilation while the ignored, pinned `cc-nagi` source is absent.
- Integrated host preview is an explicit host-only executable. Target runtime
  results are **NOT RUN**, including native launch, durable storage, target
  object/capability services, target Search service, and checkpoint restore.

### Post-fetch root workflow check

- `./nagi fetch`: **PASS**. It materialized the source-lock-pinned `cc-nagi`
  artifact and validated the pinned Surfman, tempfile, mozjs_sys, Servo, and
  Mesa sources. The generated trees are ignored fetch inputs; none was edited.
- With `cc-nagi` present, root
  `cargo test -p nagi-history --lib --locked --offline`: **PASS**, 41 tests.
  This resolves the earlier pre-compilation Cargo error for that package.
- Standard `./nagi test`: **FAIL** on this arm64 macOS host while compiling
  `user/libnagi`: inline registers `rax`, `rdi`, and related x86_64 registers
  are invalid for the arm64 host target. No syscall ABI or M17 changes were
  made. First-party standalone suites remain green.
- Standard `./nagi fmt`: **FAIL** because the pinned Servo checkout has
  rustfmt differences with the installed formatter. It is a check-only run;
  Servo source was not modified.
- `./nagi dev verify`, `./nagi dev status`, and `./nagi dev resume` pass with
  the updated fetch/test state. The workstream remains `PARTIAL`; DF-01 records
  the arm64-only full-workspace compile failure as a `HOST_ENV` blocker.

### Host CI repair checkpoint (2026-09-26)

- Source CI run `36224698245` failed before target execution. Ubuntu Clippy
  rejected six `nagi-history` style issues; Windows host tests also contained
  assertions tied to the old `development-foundation` active-stream status.
- The history implementation now uses a private timeline render request and
  named `CheckpointPinRequest` / `RestorePlanRequest` inputs for the shared
  checkpoint and restore APIs. `ObjectRefs::contains` uses slice membership,
  and the redundant render-error conversion is removed. These are host API
  input-shape changes only; mutation policy, validation, provenance, ledger
  capacity, and restore behavior are unchanged.
- The developer-state unit test now accepts the statuses defined by its
  schema. The CLI integration test checks that status reports a registered
  stream and branch rather than assuming one fixed active branch.
- Local verification after these repairs passed: Activity/Wayback 41 tests
  and its UEFI library check; `nagi-cli` 68 unit + 20 CLI tests; Files 48;
  Notes 27; Home/Search 35 library + 1 preview; cross-app integration 11;
  root-workspace Clippy and all four standalone-package Clippy checks with
  `-D warnings`; and the bilingual integrated host preview. Changed Rust files
  pass pinned rustfmt. Full `cargo fmt --all -- --check` still reports
  differences only in the fetched Servo checkout; no Servo source was edited.
- Follow-up CI run `36230579035` uses head
  `6cc3255c5a14bcf8a1692c05eeaaa4d5dd2ddc1e`; its result is pending. The
  earlier full root host test remains unavailable on this arm64 host because
  `user/libnagi` contains x86_64 syscall-register assembly. Target app launch,
  persistence, capabilities, Search/Action dispatch, and restore remain
  **NOT RUN**; the host preview does not advance M17 or M18.

### Remaining adapters and gates

The host adapters are deliberately process-local and do not claim to be
security boundaries. Target-side authenticated user context, capability
services, object/workspace resolvers, application launcher, Search/Action
services, durable Activity/Wayback stores, and target Files snapshot/restore
backend remain **BLOCKED on public Nagi runtime interfaces** or **NOT RUN**.
The source branches and worktrees remain unchanged; the target runtime and M17
state remain unchanged.
