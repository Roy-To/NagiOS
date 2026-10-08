# Nagi 0.2 Workstream Contract

The machine-readable path and branch registry is `.dev/workstreams.json`.
This document defines its operating rules. It is integration-owned; workstream
branches do not edit it during feature implementation.

## Activation

The `development-foundation` stream is the only active stream before Nagi 0.1
M30 PASS. Every runtime/product stream stays `NOT_STARTED` until that gate and
an explicit 0.2 integration checkpoint. A workstream row is a future ownership
contract, not permission to start implementation.

### Approved host-only checkpoint BP-SBOM-HOST-20261008

The user's explicit instruction approves the bounded Integration Owner checkpoint
on `codex/0.2-integrate-provenance-sbom`. Authority and edit paths are recorded in
`docs/0.2/integration-proposals/provenance-sbom/checkpoint-proposal.json` and the
`provenance-sbom-integration` registry row. It authorizes existing Build
Provenance/Legal host tooling adoption, shared host CLI/CI wiring, and explicit
standalone host-core activation for `calendar-core-01` (Codex ②),
`writer-core-01` (Claude ②) and `sheets-calc-01` (Claude ①).

The three core owners may start only their registered host paths and tests,
using the dedicated MDs and public contract seams. Optional tests paths are
approved; root workspace, shared CI, other owners' code/State and real runtime
providers are outside their boundaries. The exact Writer/Sheets Claude branches
are narrow schema/parser exceptions, not authorization for other Claude branches.
Registration does not claim app implementation or host acceptance; each owner
creates its own State and evidence. M30 PASS plus an explicit release boundary
remains mandatory for every 0.2 guest/runtime integration.

## Required workstream record

Each stream entry declares:

- stable ID and accountable owner (initially unassigned for future streams);
- exact owner branch and recommended worktree path;
- dependencies and contract versions/acceptance boundaries;
- allowed and forbidden paths;
- state-file path;
- activation gate and merge boundary.

An active stream records milestone/checkpoint, status, last verified commit,
commands and test outcomes, CI run, failure class and evidence, attempted and
prohibited fixes, acceptance criteria, deferred items, dependencies, and the
next exact action in its own state JSON.

## Shared-file policy

Only the integration owner changes `Cargo.toml`, `Cargo.lock`, shared IDL/ABI,
`.github/workflows/**`, `.dev/workstreams.json`, and `.dev/schemas/**`. Other
streams submit a focused change plus its contract impact for integration.
Generated bindings are changed through their source and generator, not edited
by hand. A stream must not write another stream's state file.

For an unavoidable shared-file change, the integration owner creates a small
integration checkpoint first, records the expected consumers and migration
plan, then updates fixtures/acceptance and each dependent stream. Do not use
merge conflict resolution as an implicit architecture decision.

## Branch, worktree, and merge boundary

Use one isolated worktree per stream with the registry's exact branch. Keep
commits small enough to resume, and do not rebase or merge another active
worktree. Integrate through a dedicated integration branch after dependency
contracts and tests pass. The 0.2 integration branch does not merge to `main`
without explicit user direction.

## Dependency contract

Before parallel work starts, publish a versioned contract, fixtures, and
negative cases. Consumers may implement against the contract and a test-only
mock; production paths must still exercise real providers and enforce
permission, authority, persistence, and failure semantics. Contract changes
increment/version the interface, document migration, and run consumer
compatibility tests before integration.
