# CFG-01 Settings / Configuration Foundation: Registration Proposal

## Request to Integration Owner

Register workstream ID settings-configuration, owner "Codex Settings / Configuration Foundation", branch codex/0.2-settings-configuration, and state file .dev/workstreams/settings-configuration/state.json.

The source definition is docs/workstreams/NagiOS_0.2_Settings_Configuration_Foundation_Workstream.md. It recommends /Users/tozawa/Developer/NagiOS-0.2-settings-configuration. This task uses the isolated managed worktree at /Users/tozawa/.codex/worktrees/settings-configuration/NagiOS; decide during registration which path the registry should recommend.

## Activation audit

The gate is closed. docs/implementation_status.md records M17 as BLOCKED, M18 as NOT STARTED, and M30 as NOT STARTED. ./nagi dev status on codex/integration-next-phase requires M30 PASS and an explicit release-boundary checkpoint before 0.2 runtime/product work. No M30 PASS or explicit checkpoint is recorded in the checked-in status or active integration state.

This branch therefore contains preparation only. It does not implement Settings runtime/API code or runtime tests.

## Candidate registry fields

- ID: settings-configuration
- Owner branch: codex/0.2-settings-configuration
- State file: .dev/workstreams/settings-configuration/state.json
- Dependencies: development-foundation, capability-permissions, and diagnostics (diagnostics remains optional)
- Activation gate: M30 PASS plus an explicit 0.2 integration checkpoint; until then, proposal/state preparation only
- Merge boundary: codex/integration-next-phase after activation and owned acceptance; never merge to main without explicit user direction

Owned paths after activation:

- crates/nagi-settings-config/**
- tests/settings-config/**
- .dev/workstreams/settings-configuration/**
- docs/workstreams/NagiOS_0.2_Settings_Configuration_Foundation_Workstream.md

Keep root Cargo manifests/lockfile, shared CI, common IDL/ABI, .dev/workstreams.json, and .dev/schemas/** with the Integration Owner. Submit any required shared changes as a proposal before editing.

Explicit exclusions: tools/nagi-cli/src/config.rs, localization catalogs, Settings UI, secret-vault implementation, app features, M18/browser code, and every other workstream's implementation, state, or worktree.

## Contract and review items

When activated, implement and test the supplied CFG-01 definition:

1. Typed keys, schemas, validators, stable errors, and validated defaults.
2. Exact resolution order: System Defaults, Machine, User, App, Session Override; include provenance, invalid-value handling, and inherit/reset behavior.
3. Persistence abstraction and in-memory backend; deterministic versioned migration; atomic multi-key update; evidence-preserving invalid-config recovery.
4. Versioned export/import with preview, validation before write, all-or-nothing import, and secret redaction.
5. Bounded ordered change subscriptions, deterministic cancellation, and explicit overflow behavior.
6. Secret separation through opaque references or provider interface. Secret bytes must never enter ordinary persistence, exports, watch events, or diagnostics.
7. Operation/scope-specific capability hooks. Capability policy and vault ownership stay with their owners.
8. Focused tests, formatting, Clippy, failure evidence, state updates, then commit/push on the owned branch.

No registry/schema edit is requested here. The Integration Owner should register the row and any needed validator support in a separate checkpoint, then record the explicit activation decision before runtime work starts.
