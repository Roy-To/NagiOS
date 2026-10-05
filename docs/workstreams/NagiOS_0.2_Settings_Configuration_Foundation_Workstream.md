# Nagi OS 0.2 — Settings / Configuration Foundation Workstream

## 1. Workstream

- **Workstream ID:** `CFG-01`
- **Name:** Settings / Configuration Foundation
- **Proposed branch:** `codex/0.2-settings-configuration`
- **Recommended worktree:** `/Users/tozawa/Developer/NagiOS-0.2-settings-configuration`
- **State:** `.dev/workstreams/settings-configuration/state.json`
- **Target:** Nagi OS 0.2 host-testable foundation
- **Initial status:** `NOT_STARTED`

No Settings/configuration foundation stream or OS config crate currently owns this scope. `tools/nagi-cli/src/config.rs` is developer-tool configuration and must not be repurposed as the user/system Settings store. Nagi 0.2 runtime work remains gated on Nagi 0.1 M30 `PASS` and an explicit integration checkpoint.

## 2. Purpose

Define a typed, versioned, validated configuration foundation for Nagi services and applications. Provide deterministic defaults, layered resolution, persistence/migration boundaries, atomic updates, recovery, import/export, and change notifications while keeping secret values outside ordinary settings storage.

This is a storage and contract foundation, not a Settings UI or the implementation of every user preference.

## 3. Scope

- Typed setting keys and schemas with stable English identifiers, value types, defaults, validators, ownership, and sensitivity classification.
- Layered precedence exactly: `System Defaults → Machine → User → App → Session Override` (highest present valid value wins).
- Typed resolution with source/provenance metadata and explicit reset/inherit behavior.
- Persistence abstraction with in-memory test backend and a future durable backend boundary.
- Schema versioning and deterministic migrations.
- Atomic multi-key update/transaction semantics; publish changes only after successful commit.
- Invalid configuration detection, safe fallback to validated defaults, and bounded quarantine/reporting that preserves recoverable user data.
- Versioned export/import with validation, dry-run/preview result, and all-or-nothing application of an import batch.
- Watch/change notifications with bounded subscriptions, ordering, and cancellation/unsubscribe.
- Sensitive-value separation using opaque secret references or a dedicated secret-provider interface; ordinary config export/watch/log paths never reveal secret bytes.

## 4. Non-goals

- Settings UI, screens, widgets, onboarding, or preference-management application.
- Localization catalog, locale formatting, input method, IME, Albert conversation behavior, or language selection implementation; consume the existing localization contract only.
- A universal filesystem/database backend chosen without the integration/storage decision.
- Credential/vault implementation, secret key derivation, or exporting secret material.
- Network/cloud sync, multi-device conflict resolution, policy management, or permission prompts.
- Editing developer CLI configuration or M18/browser settings.

## 5. Dependencies and repository boundaries

- `docs/architecture/language-architecture.md`: canonical internal keys/errors are English and UTF-8; system language, region, input language, and AI conversation language are separate settings.
- `docs/architecture/unified-device-application-model.md`: persistent settings are durable data; session overrides are session-local; do not conflate settings with execution-local or presentation-local state.
- Capability / Permission: narrow read/write/export/import/watch authorization adapter; this workstream does not own policy.
- Diagnostics: stable codes and redacted metadata through an optional sink.
- Activation gate: Nagi 0.1 M30 `PASS` plus an explicit Integration Owner checkpoint for runtime/product adoption.

Allowed after activation: `crates/nagi-settings-config/**` (or an approved equivalent), `tests/settings-config/**`, `.dev/workstreams/settings-configuration/**`, and this workstream document. Do not edit `tools/nagi-cli/src/config.rs`, shared root Cargo files, shared CI, `.dev/workstreams.json`, `.dev/schemas/**`, common IDL/ABI, another stream's state, localization catalogs, or app/UI feature code unless the Integration Owner grants ownership. Put shared requirements in a proposal and proceed on owned files.

## 6. Architecture and API/data model

Use typed keys rather than stringly typed get/set calls:

- `SettingKey<T>`: stable key ID, schema version, scope, default, validator, sensitivity, and optional migration identifier.
- `SettingScope`: system defaults, machine, user, app namespace, and session override.
- `ResolvedSetting<T>`: validated value plus winning layer, schema version, and safe provenance.
- `ConfigStore` / `PersistenceBackend`: load snapshot, stage update, commit atomically, and enumerate schema version; backend errors are typed.
- `Migration`: pure, version-to-version transformation with validation and explicit failure result.
- `ChangeEvent<T>` / `WatchHandle`: key/scope, old/new safe value or redacted marker, revision/sequence, and source; cancellation/unsubscribe is deterministic.
- `SecretRef`: opaque identifier handled by a dedicated provider, never a secret value embedded in exportable settings.

Resolution order is fixed and tested. Invalid higher-layer values must not shadow a valid lower layer silently: return a structured validation issue, apply the documented fallback policy, and preserve the invalid raw record for recovery/quarantine when safe. Merging is not implicit; a schema must declare a typed merge strategy if one is needed. Empty, missing, deleted, and “inherit lower layer” states must remain distinguishable.

Updates stage and validate every changed key, persist the transaction atomically, then advance the revision and notify watchers. A failed commit leaves the prior snapshot intact and emits no success notification. Import validates the full versioned document and all target scopes before an all-or-nothing commit. Export excludes secret values by default and may include only opaque references when explicitly supported.

## 7. Failure and recovery

- Unknown schema versions fail explicitly; supported migrations are deterministic and idempotent.
- Migration failure preserves the original bytes/record and serves validated defaults or the last known-good snapshot according to documented policy.
- Invalid per-key values do not corrupt unrelated valid settings; report stable key/error IDs without exposing values.
- Atomic update failure leaves the previous committed snapshot readable and emits no change event.
- Partial/corrupt persistence is detected; recover the last complete snapshot or safe defaults without truncating the only recoverable copy.
- Import rejects malformed, incompatible, unauthorized, or secret-bearing content before changing any value.
- Watcher failure, overflow, or cancellation cannot roll back a committed update or block writers indefinitely; overflow semantics are explicit.

## 8. Security and capability hooks

Authorize by operation and scope: read, write, watch, export, and import are distinct hooks. App-scoped callers may access only their app namespace unless a public policy contract explicitly grants broader access. Machine/system layers require privileged service authority, not an app-supplied scope string. Fail closed when principal or scope cannot be validated. Keep secret values in a separate provider; do not serialize them into settings files, events, diagnostics, or normal exports.

## 9. Diagnostics hooks

Provide stable codes for unknown key/schema, validation failure, migration failure, atomic commit failure, recovery/default fallback, import rejection, export redaction, authorization denial, watcher overflow, and persistence corruption. Events may include key ID, layer, schema version, revision, and stable error code; never include secret bytes or unrestricted setting values. Diagnostic failure must not invalidate an otherwise successful committed update.

## 10. Acceptance Criteria

- [ ] Typed keys, schema validation, defaults, and stable errors are implemented.
- [ ] The five-layer precedence is implemented exactly and tested, including invalid-value and inherit/reset behavior.
- [ ] Persistence is behind an abstraction with an in-memory reference backend.
- [ ] Schema migration is versioned, deterministic, and preserves source data on failure.
- [ ] Multi-key updates are atomic; failed updates leave the old snapshot unchanged.
- [ ] Invalid configuration recovery preserves evidence and resolves to safe validated values.
- [ ] Export/import is versioned, validates before write, and applies atomically.
- [ ] Change notifications occur only after commit and have bounded watcher behavior.
- [ ] Sensitive values are separated, redacted from export/watch/diagnostics, and covered by negative tests.
- [ ] Capability hooks distinguish operations/scopes and deny unauthorized access.
- [ ] Existing localization and app identity contracts are consumed without taking their ownership.
- [ ] No M18 or unrelated workstream files are modified.
- [ ] Workstream state and registration proposal follow DF-01 rules.
- [ ] Focused checks pass; changes are committed and pushed only on the owned branch.

## 11. Tests and CI

Test type mismatch, invalid keys/values, defaults, each layer and precedence edge, app namespace isolation, session override removal, migration success/failure/replay, atomic multi-key rollback, corrupt/unknown schema recovery, import dry-run/rejection/atomicity, export redaction, secret reference behavior, authorization allow/deny per operation, watcher ordering/unsubscribe/overflow, and diagnostics redaction. Use deterministic in-memory fixtures; no network dependency.

Run focused package tests, formatting, Clippy with warnings denied where supported, and relevant schema checks. Do not edit shared CI/workspace configuration; submit a narrow Integration Owner proposal if registration is required. Avoid M18/browser builds as an unrelated check.

## 12. State, commit, and push

Update only `.dev/workstreams/settings-configuration/state.json` in the current DF-01 format, recording actual activation gate, status, verified SHA, commands/results, migration and recovery evidence, blockers, acceptance checklist, dependencies, and next exact action. If the stream is not registered or validation rejects its state, add a focused registration proposal under `.dev/workstreams/settings-configuration/`, record the exact CLI/schema limitation, and leave `.dev/workstreams.json` unchanged. Do not claim `PASS` while owned acceptance remains incomplete.

Commit only owned files on the assigned branch, push reviewable checkpoints, and verify the pushed SHA, remote equality, clean worktree, and CI outcome. Do not merge to `main` without explicit direction.

## 13. Git and worktree safety

Before editing, inspect branch, HEAD, remote, dirty state, and `git worktree list`. Reuse and preserve an existing dedicated configuration worktree. Do not reset, clean, force-checkout, rebase, or change branches in another workstream's worktree. If no dedicated worktree exists, use the Integration Owner-approved setup and never invent a base SHA.
