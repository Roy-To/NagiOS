# Nagi 0.2 Workstream Contract

The machine-readable path and branch registry is `.dev/workstreams.json`.
This document defines its operating rules. It is integration-owned; workstream
branches do not edit it during feature implementation.

## Activation

The `development-foundation` stream is the only active stream before Nagi 0.1
M30 PASS. Every runtime/product stream stays `NOT_STARTED` until that gate and
an explicit 0.2 integration checkpoint. A workstream row is a future ownership
contract, not permission to start implementation.

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
