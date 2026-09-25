# Nagi 0.2 Development Foundation

Status: development-process specification; Nagi 0.2 product roadmap remains a
candidate, not a commitment.

## Purpose and release-line boundary

Nagi 0.2 development must be resumable from repository state alone. A Codex
session, host computer, worktree, or chat history is not durable project state.
This foundation adds only documentation and host-side development tools. It
does not start M18–M30, replace the M17 Servo work, or change Nagi runtime
behavior.

The checked-in 0.1 baseline remains M17 Servo Bootstrap `BLOCKED`; M18 and
later 0.1 milestones remain `NOT STARTED`. M18–M30 must finish and the 0.1
release boundary must be explicitly established before a 0.2 runtime feature
workstream activates. The 0.2 foundation and tooling may be prepared in
parallel, in isolated branches/worktrees. Never edit, clean, reset, rebase,
merge, or cherry-pick into another active worktree as a convenience step.

The existing `docs/implementation_status.md` remains the authority for 0.1.
The 0.2 state lives under `.dev/workstreams/`; neither status is copied into
the other.

This foundation was checked against
`docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`,
`docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md`,
`docs/architecture/language-architecture.md`,
`docs/implementation_status.md`, and the M17 records under
`docs/decisions/`. The roadmap below records candidate dependencies and
migration questions from those contracts; it does not supersede them.

## Durable state and ownership

`.dev/workstreams.json` is the integration-owned registry of workstream IDs,
branch names, dependencies, path ownership, activation gates, and merge
boundaries. Each active workstream owns exactly one
`.dev/workstreams/<id>/state.json`. That file is the single source for its
checkpoint, status, verified commit, tests, CI, blocker, evidence, tried and
prohibited fixes, next action, acceptance criteria, deferred work, and
cross-workstream dependencies.

The state JSON Schema under `.dev/schemas/` documents the format.
`./nagi dev verify` parses the registry and state and rejects unknown statuses/failure
classes, malformed commit SHAs, missing evidence fields, duplicate branch or
workstream IDs, unsafe state paths, and missing dependencies. The registry is
not a second status database: a registered stream with no state file is
`NOT STARTED`. Workstream status belongs only in its own state file.

The CLI is read-only except `dev diagnose`, which writes a report only when
given `--output`. `dev status` reads the current Git branch and HEAD directly;
those live values are not duplicated in state. `last_verified.commit_sha` is
the last commit whose recorded checks completed successfully. Git HEAD is the
current commit, including the commit that carries the state update; this avoids
a self-referential commit hash.

## Resume protocol

Start with:

```sh
./nagi dev status
./nagi dev resume
./nagi dev verify
git status --short --branch
git log -5 --oneline --decorate
```

Then read, in order:

1. `AGENTS.md` and the active workstream's state file;
2. the relevant 0.1 or 0.2 specification and linked contracts;
3. the last verified test and CI evidence;
4. the exact blocker evidence and `next_action`.

Confirm the current branch/worktree and inspect diffs before editing. Never
infer that a test passed from a status label alone. If state and evidence
disagree, preserve the raw evidence, classify the mismatch, and correct state
before continuing.

## Autonomous implementation and checkpoint loop

For each bounded acceptance item:

1. Read the owning workstream state and choose its exact `next_action`.
2. Inspect relevant implementation, specifications, tests, status, and diff.
3. Implement the smallest complete change within owned paths.
4. Run format/static validation, then the cheapest focused test, then target
   build/acceptance as required by the acceptance criteria.
5. Preserve complete logs, process exit status, stage, artifact metadata, and
   failure inventory before retrying a costly command.
6. Update only the owning workstream state with evidence and the next exact
   action.
7. Commit a coherent, resumable checkpoint. Push only that branch when its
   checkpoint is reviewable. Never merge it to `main` without explicit user
   direction.
8. Inspect CI by immutable run ID and head SHA. Record the URL, conclusion,
   failing job/stage, and diagnostic summary in the owning state.
9. Repair a failure only after recording its class and what differs from the
   previous attempt.

One checkpoint should let another developer resume without reconstructing
chat history. Do not combine unrelated docs, implementation, and CI changes
into one oversized commit; do not create a commit for every trivial edit.

## Failure classes and retry budget

Use the machine-readable classes:

`SOURCE`, `BUILD`, `LINK`, `ABI`, `RUNTIME`, `BOOT`, `DEVICE`, `STORAGE`,
`GRAPHICS`, `NETWORK`, `MODEL`, `PERMISSION`, `ACCEPTANCE`, `CI_INFRA`,
`HOST_ENV`, `UNKNOWN`.

Record the exact command, stage, exit code or termination reason, evidence,
previous attempt, changed hypothesis, attempted fixes, prohibited repeats, and
next experiment. A heuristic diagnostic class is a suggestion; the owner
confirms the class from evidence. Never retry a compiler/link/runtime failure
unchanged. Transient network or runner infrastructure failures may be retried
up to three times with backoff. The repository's existing limit of ten
meaningful repair attempts applies to a persistent implementation blocker;
then record `BLOCKED` with the next experiment instead of looping.

## Diagnostics and acceptance evidence

Retain unfiltered stdout and stderr where the harness provides them, the full
serial/runtime log, exit code, failed stage, and termination reason. A summary
must not replace or truncate source logs. Link diagnostics inventory all
undefined symbols and candidate providers rather than showing only the first
few. For files relevant to a result, record path, byte length, SHA-256, and
format when known. Diagnostic reports identify the checked-out source commit.
The current M17 diagnostic images are evidence only, not reusable build
artifacts; a complete build fingerprint is required before artifact reuse is
introduced.

`./nagi dev diagnose` produces a machine-readable report with source commit,
log digests, complete error/undefined-symbol inventories, bounded tails for
quick reading, suggested failure class, process status, and requested artifact
metadata. It does not rewrite input logs or mark an acceptance as passing.
Acceptance criteria remain owned by the existing acceptance command and are
never weakened to improve diagnostics.

## CI stages, cancellation, and cost

Keep cheap host formatting, static checks, unit tests, and launcher tests
separate from expensive target build and QEMU acceptance where workflow
structure permits. The current workflow gates the expensive target job on both
host jobs so a quick host regression does not also spend a target-runner cycle.
A target acceptance run should not be canceled merely
because a later commit arrived: allow one in-progress target run to finish and
retain at most the newest pending run. Cheap host jobs may cancel stale work.
Release acceptance uses an immutable commit and a non-canceling concurrency
group. Every result is attached to its exact head SHA.

A push or pull request that changes only a workstream `state.json` or the
authoritative 0.1 status handoff does not dispatch the full build workflow;
owners run `./nagi dev verify` before state-only checkpoints. Mixed status and
source changes still run CI. The host jobs also execute the verifier, so
registry/schema/tooling changes remain covered.

The current target job still performs dependency/bootstrap/build and M17
acceptance in one job. Splitting it is a later optimization gated on a measured
artifact manifest that includes source and patch revisions, compiler and
toolchain, target, flags, feature set, and all generated inputs. A separate
acceptance rerun may consume only a verified immutable artifact with matching
manifest and checksum. Clean-environment build verification remains a distinct
release gate.

Do not add broad caches or reuse `target/`, Mesa, Servo, generated sources, or
images by a partial key. First measure which stage dominates. Any cache key
must include pinned source revision, Nagi patch digest, compiler/toolchain,
target, build flags, and enabled features. Cache misses rebuild; cache hits
must validate manifests and checksums. No stale or host-built artifact may
produce target acceptance PASS.

## Workstreams and cross-stream contracts

The registry defines one branch/worktree and allowed-path boundary per stream.
Each active stream updates its own state file, avoiding concurrent edits to a
global status document. `Cargo.toml`/`Cargo.lock`, `.github/workflows/**`,
shared IDL/ABI definitions, and the workstream registry/schema are integration
owned: feature branches propose changes, but only the integration checkpoint
changes those shared files. Generated summaries, if introduced later, are
outputs and must not become hand-edited state.

Workstreams depend on versioned contracts and fixtures, not another stream's
internal implementation. A fixture or mock transport is valid for contract
or orchestration tests only; it cannot be presented as product success.
Contract changes require a version/migration note, compatibility tests, and an
integration owner. Capability checks, permission ordering, Activity/Transaction
recording, and AI validation remain deterministic acceptance boundaries.

All 0.2 runtime streams are gated on 0.1 M30 PASS and an explicit release
boundary. The registry may be prepared earlier, but a planned branch is not
authorization to start its runtime implementation.

## Existing implementation audit: KEEP / ADAPT / REPLACE / DEFER

| Area | Classification | 0.2 preparation guidance |
|---|---|---|
| Capability kernel, user-space services, M11 Permission Broker | KEEP | Preserve authority boundaries; new platform operations continue through policy and permission checks. |
| M15 history, persistence, undo | ADAPT | Extend the bounded file history toward semantic Activity, Transaction, Revision, Checkpoint, provenance, and restore contracts; do not call debug logs Activity. |
| M16 package, IDL, SDK | ADAPT | Preserve signing, side-loading, and generated bindings; stabilize public permission, Action, Context, app/session, and surface contracts before ecosystem expansion. |
| M10 desktop widgets and current identity model | ADAPT | Keep desktop as the first surface; separate AppSession, ExecutionInstance, Node, and Surface identities before multi-surface behavior. |
| Sample SDK surface/node mapping | REPLACE narrowly | Replace the sample's derived SurfaceId and hardcoded NodeId before treating it as a real multi-device service; assess AppId collision and migration compatibility first. |
| `nsh` and minimal Servo Albert embedder | ADAPT | Reuse them for richer Terminal surfaces and Albert platform integration only after 0.1 acceptance gates. Servo remains the browser engine. |
| Full mobile parity, remote desktop, real-time office collaboration, cloud-required AI | DEFER | Keep semantic contracts device-independent; these are not prerequisites for 0.2 foundation or 0.1 release. |

This is an audit disposition, not a claim that the listed platform services or
first-party applications are already implemented.

## Candidate roadmap (not a committed schedule)

| Candidate | Dependencies / 0.1 prerequisite | Likely stream | Acceptance and migration | Compatibility risk and parallelization |
|---|---|---|---|---|
| Platform API stabilization | M16 package/IDL/SDK PASS; M30 PASS | Platform APIs | Freeze versioned IDL/API contracts; migrate consumers with compatibility fixtures | ABI/schema changes affect all apps; fixtures can precede implementations |
| Resource, Document, Object, Action, Intent, Context | Stable Resource/Object identity, Permission Broker, M16; M30 PASS | Platform APIs | Contract tests cover identity, revisions, permission denials, and Action validation; migrate path references to stable IDs | Preserve paths as locators, not identities; independent contract tests after IDL boundary freezes |
| Activity, Transaction, Checkpoint, Wayback | M15 history PASS plus revision/restore contracts; M30 PASS | OS Core + Platform APIs | Test append-only provenance, recovery, partial failure, and supported restore; migrate bounded snapshots incrementally | Current bounded snapshots are not general restore; irreversible effects must never claim undo; storage and UI can use a versioned contract |
| Workspace, Device, Surface, Continuity | M16 session identity; M30 PASS | Platform APIs | Validate local reconstruction from semantic state and explicit remote permission failures; version session migration | Remote nodes never inherit local authority; semantic state excludes window coordinates; transport can follow local fixtures |
| Search | Resource IDs and permission filtering; M30 PASS; semantic provider optional | Platform APIs + AI runtime | Test lexical results, permission-filtered results, deletion/index migration, and offline behavior | Secret leakage and index migration are risks; lexical acceptance does not depend on models |
| First-party software maturation | Shared contracts, Activity/Wayback/Search; M30 PASS | First-party apps | App acceptance proves shared services are reused and user-visible actions are recorded; migrate each prototype behind stable contracts | Existing apps are limited slices; avoid duplicate services; app-specific UI can follow contract stabilization |
| SDK, app ecosystem, and app isolation | M16 packaging baseline and public contract versions; M30 PASS | SDK + Platform APIs | Validate signed package compatibility, capability denial, and upgrade/migration fixtures in out-of-tree apps | Preserve signatures and side-loading; isolate capabilities; frozen fixtures enable independent samples |
| AI provider abstraction and future Jev/System One readiness | Existing provider-neutral architecture, permission/action executor boundary; M30 PASS | AI runtime | Run provider conformance, deterministic validation, denial, and offline fallback tests | Jev/cloud is optional; model choice never grants authority; conformance does not need a cloud service |
| Multi-device foundations and Continuity | Stable Workspace, Device, Surface, session and identity contracts; M30 PASS | Platform APIs + OS Core | Test semantic state migration across local device profiles and explicit authority boundaries before transport | Avoid window-coordinate coupling; remote execution cannot inherit local grants; local contract work can precede networking |
| Performance, security, accessibility hardening | M30 PASS plus real feature baselines and representative workloads | OS Core + CI/Acceptance | Establish repeatable representative workloads, threat cases, and accessibility acceptance before tuning | Avoid synthetic-only wins; retain fail-closed checks; workload suites can follow stable acceptance contracts |
| Developer tooling / CI improvements | This foundation; product-facing tooling after M30 PASS | Developer tooling + CI/Acceptance | Validate resumable state, full diagnostic artifacts, immutable fingerprints, and clean-environment rebuilds | Measure cache/artifact cost before reuse; documentation and read-only resume infrastructure can evolve before runtime work |

Candidate ordering is provisional. The Nagi 0.1 milestone sequence remains
authoritative until M30 passes and a 0.2 product decision is made.
