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
