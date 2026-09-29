# Nagi OS 0.2 — Local Identity / User Session Foundation Workstream

## 1. Workstream

- **Requested ID:** IDENT-01
- **Registry ID:** ident-01
- **Name:** Local Identity / User Session Foundation
- **Proposed branch:** codex/0.2-ident-01
- **Assigned worktree:** /Users/tozawa/.codex/worktrees/ident-01/NagiOS (managed dedicated worktree)
- **State:** .dev/workstreams/ident-01/state.json
- **Target:** Nagi OS 0.2 offline-first, host-testable identity and session foundation
- **Initial status:** NOT_STARTED

The proposal baseline is commit 198b9f60b41eba14e74c2c04f51ccdcb4030adf9. At that baseline M17 is BLOCKED and M30 is NOT STARTED. This document does not activate a login/runtime service, register the workstream, or authorize edits to shared contracts.

## 2. Purpose and background

Define stable local user, profile, session, and principal references so Nagi remains fully usable offline and future identity providers can be added without changing Nagi's internal identity. Keep identity separate from applications, application sessions, execution instances, windows, and storage locations.

The existing Nagi model distinguishes logical AppSessionId from ExecutionInstanceId and SurfaceId. IDENT-01 must consume those contracts through adapters and must not collapse user identity into AppId or a process/window identity.

## 3. Scope

- Stable, validated newtypes or equivalent opaque identifiers for UserId, ProfileId, SessionId, and PrincipalId, plus provider-neutral provider/subject references where needed.
- Local user record with stable UserId and versioned, non-secret profile metadata.
- Guest/ephemeral session that has no durable login credential, remains isolated from persistent users, and is destroyed according to an explicit end-session policy.
- Profile metadata and versioned serialization contract with a forward-compatible, privacy-minimizing field set.
- Explicit session create/end lifecycle with idempotent end, reason codes, expiry abstraction, and stable current-user/current-session resolution.
- CurrentIdentityResolver adapter that does not depend on a cloud provider or global mutable singleton.
- User-scoped storage adapter that resolves an opaque, capability-scoped StorageRootHandle for a UserId/ProfileId. It must not expose a host absolute path as identity or authority.
- Capability principal mapping adapter that consumes the canonical capability contract when available and fails closed when mapping cannot be confirmed.
- Local login/logout hooks and provider-neutral identity-provider boundary. Local setup and guest use must work without network access or external account services.
- Persistence abstraction with versioned atomic commit, last-known-good recovery boundary, and explicit policy for which sessions may resume after restart.
- Profile isolation and malformed/corrupted state tests using fake or temporary storage.
- Idempotency and concurrency rules for duplicate session creation/end and current-session resolution.

## 4. Non-goals

- Microsoft, Google, Nagi Cloud, OAuth/OIDC, or any network authentication implementation.
- Password manager, password database, enterprise directory, remote account recovery, or cloud account linking UI.
- Replacing AppId, AppSessionId, ExecutionInstanceId, SurfaceId, or Capability Principal contracts.
- Guest or local identity becoming a universal privileged/root identity.
- Kernel login, user switching UI, setup UI, lock screen, or first-party app behavior.
- Storing credentials, authentication tokens, recovery secrets, or secret values in profile metadata.
- Changing user/nagi-init, SDK, shared IDL, or capability policy owned by another workstream.

## 5. Dependencies and activation gate

- **Required development contract:** development-foundation DF-01 ownership, state, and verification rules.
- **Capability contract:** capability-permissions owns authorization principals and decisions. Consume a published version through an adapter; do not define a competing authority model. Its current workstream state is PARTIAL, so tests may use a narrow fake and production authorization remains unintegrated until the owner contract is available.
- **Application/session contract:** M16 and the App SDK contract own AppId, AppSessionId, and ExecutionInstanceId semantics. The current App SDK contract state is PASS, but recheck its actual integration/base and never edit its files here.
- **Storage contract:** use the repository's approved storage/VFS/Object handle boundary if available. Otherwise define only a narrow adapter proposal; do not introduce a host-path or new shared storage API.
- **Before activation:** stable type/API proposals and an isolated host-only reference model with in-memory/fake providers may be prepared only on an assigned branch and in approved paths. A mock does not authorize product login, persistent identity service, or user-data migration.
- **Runtime/integration gate:** production session persistence, login/logout service wiring, capability enforcement integration, user-scoped storage integration, or target behavior requires Nagi 0.1 M30 PASS and an explicit Integration Owner checkpoint naming IDENT-01. A stricter registry gate takes precedence.

Hard registry dependencies must name only existing registered IDs. Future Identity consumers and unregistered service contracts remain compatibility proposals until their owners publish versioned interfaces.

## 6. Allowed and forbidden edits

Proposed allowed paths, subject to Integration Owner assignment:

- crates/nagi-identity-session/**
- tests/identity-session/**
- .dev/workstreams/ident-01/**
- this specification

Do not edit without explicit Integration Owner ownership:

- Root Cargo.toml, root Cargo.lock, .github/workflows/**, .dev/workstreams.json, .dev/schemas/**, shared IDL/ABI, or generated bindings.
- kernel/**, loader/**, user/**, sdk/**, third_party/**, out/**, target/**, docs/implementation_status.md, or another workstream's state/source/worktree.
- Existing app/session, capability, VFS, package, M18, or first-party workstream paths.
- Any file outside the assigned allowed paths.

A standalone host-only crate may include its own manifest under its allowed directory. If it needs root workspace membership, a shared type/schema, migration code, or CI wiring, submit a focused proposal under .dev/workstreams/ident-01/ and continue independent work without changing owner files.

## 7. Architecture and type/API policy

Keep internal identity provider-neutral, opaque, and distinct by purpose:

- UserId: stable local Nagi user identity; never derived from a provider's mutable display name or remote subject.
- ProfileId: identity for one isolated local profile and its storage scope; define whether a user may own multiple profiles without assuming a UI.
- SessionId: one authenticated local or guest session lifecycle; never reuse it after end or restart.
- PrincipalId: authorization principal reference. Reuse the canonical capability PrincipalId if it exists; otherwise put a proposed newtype behind an adapter and record the convergence decision rather than creating a second policy vocabulary.
- LocalUser: stable identity plus non-secret metadata and schema version; authentication material is outside this model.
- GuestSession: explicit ephemeral identity/profile scope with no authority inherited from another user.
- SessionRecord: session kind, user/profile references, creation/expiry metadata, lifecycle state, and schema version; omit secrets and ambient authority.
- IdentityProvider: provider-neutral create/resolve/link interface whose local implementation requires no network. Future federation maps external subjects to a stable local UserId instead of replacing it.
- CurrentIdentityResolver: resolves an authorized caller context explicitly and reports no-session, ended, expired, corrupt, and unavailable cases distinctly.
- UserStorageRoot: adapter returning an opaque storage root handle constrained to the exact profile; never treat a path string as a capability.
- CapabilityPrincipalAdapter: maps a session/profile context to a least-authority principal through the capability owner contract; denial or missing identity prevents the protected operation.
- IdGenerator / EntropySource: production IDs require an approved unpredictable source; tests inject deterministic IDs. Do not use predictable test IDs in production.

Do not equate UserId/ProfileId/SessionId/PrincipalId with AppId, AppSessionId, ExecutionInstanceId, NodeId, SurfaceId, PID, filesystem path, or a display name.

## 8. Failure and recovery

- Malformed, truncated, unsupported-version, or corrupt identity state is rejected before use. Preserve bounded evidence and leave the previous committed state intact.
- Recovery may return a safe no-current-session state or require explicit repair; it must not guess a user, profile, principal, or grant.
- Atomic persistence either commits a complete versioned snapshot or preserves the prior snapshot. Interrupted writes never expose a partial identity as valid.
- End-session is idempotent. An ended or expired session cannot be restored by replaying a stale handle.
- Guest sessions are never promoted implicitly to persistent local users; guest cleanup does not traverse another profile's storage root.
- Current identity lookup reports absent/ambiguous identity explicitly and never falls back to a universal owner principal.
- If principal binding or storage-root resolution fails, protected access is denied and no broader fallback is attempted.
- Concurrent create/end/resolve operations have deterministic conflict behavior and never return two current identities for one scope unless an explicit multi-session policy permits it.

## 9. Security and capability hooks

- Resolve the trusted caller before checking identity-scoped operations. A caller-provided UserId/ProfileId string is not proof of ownership.
- Apply separate authorization hooks for create local user, create guest session, resolve current identity, access profile root, end session, and recovery/repair.
- Deny by default when the principal or profile scope is missing, stale, or inconsistent.
- Guest authority is explicitly limited and cannot inherit another user's capabilities, storage handles, or active session.
- Revalidate authorization after recovery and after session creation; persisted session metadata does not itself grant authority.
- Keep metadata and diagnostics free of secrets; provide no cloud/network bypass.
- Profile isolation checks include attempted cross-user handle use and stale-session replay.

## 10. Diagnostics and observability hooks

Emit stable structured events for local-user creation, guest/session creation, current-identity resolution result, expiry, logout/end, persistence commit/recovery, corrupt-state rejection, principal mapping allow/deny, and storage-root resolution failure. Include opaque IDs and safe reason codes only. Do not record credentials, token material, full user data, or unrestricted paths. Sink failure must not make authentication succeed or invalidate an otherwise committed operation.

## 11. Acceptance Criteria

- [ ] UserId, ProfileId, SessionId, and PrincipalId are stable typed values with validation and documented non-interchangeability.
- [ ] Local user and guest/ephemeral session models work without network access.
- [ ] Profile metadata is versioned, minimal, and contains no credentials or secret values.
- [ ] Session creation, current-user/current-session resolution, expiry, and idempotent end are deterministic.
- [ ] User-scoped storage root adapter returns an opaque profile-constrained handle and prevents cross-profile access.
- [ ] Capability principal mapping uses the canonical contract or a documented adapter and denies missing/invalid mappings.
- [ ] Local login/logout hooks do not depend on cloud identity or network services.
- [ ] Persistence has an explicit restart/recovery boundary, atomic-write behavior, and safe malformed/corrupt-state recovery.
- [ ] Provider-neutral federation can map future providers to stable local identities without changing UserId.
- [ ] Profile isolation, stale session, corrupt state, interrupted write, guest cleanup, and principal denial cases are tested.
- [ ] No cloud authentication, M18, user/nagi-init, shared capability policy, or unrelated workstream code is changed.
- [ ] DF-01 state/proposal ownership is followed; schema/root manifest/CI files remain untouched. Any registry change is made only as a separate Integration Owner registration action, not by feature implementation.
- [ ] Focused checks, owned-branch commit/push, remote equality, clean worktree, and CI status are recorded when branch/gates permit.

## 12. Tests and CI

Cover ID validation and namespace collisions; local and guest session lifecycle; duplicate create/end; current resolver absent/expired/ended/corrupt cases; restart policy; profile A/B isolation; cross-profile root-handle denial; stale-session replay; principal map allow/deny; entropy-source failure; schema migration success/failure; interrupted atomic write; malformed/truncated/unknown-version state; guest cleanup; and offline operation. Use deterministic ID and storage fakes for tests only.

Run focused package tests and supported formatting/static checks offline. Do not run network authentication tests because none are in scope. Do not edit shared CI; propose a host-only job if needed. Runtime, target, and product-login acceptance remain gated and must not be represented by host reference tests.

## 13. State, commit, push, and report

Maintain only .dev/workstreams/ident-01/state.json using the current DF-01 schema. Record the live release gate, actual status, checked commit, exact commands/results, CI run/head SHA, failure evidence, attempted/prohibited fixes, acceptance checklist, dependency integration status, deferred items, and exact next action. If unregistered, prepare a focused registration proposal under .dev/workstreams/ident-01/ and record that the root verifier excludes the unregistered state. Do not claim PASS for unavailable production integration.

Commit only assigned files on the approved branch and push only that branch after a reviewable checkpoint. Verify remote HEAD equality and clean/dirty status of the owned worktree. If the gate, owner assignment, or remote permissions block push, complete permitted local preparation, record the precise blocker and Integration Owner action, and never push on an unrelated branch or main.

The final report must state workstream ID, branch, final HEAD SHA, remote equality, worktree clean/dirty, implemented identity/session contracts, Acceptance results, focused tests/CI, blockers, and exact Integration Owner next action.

## 14. Git and worktree safety

Inspect AGENTS.md, relevant specs, live state, Git status/diff, branch, HEAD, remote, and git worktree list before editing. Reuse and preserve a suitable dedicated identity worktree. If none exists, use only the Integration Owner-assigned branch/base and worktree procedure. Never reset, clean, force-checkout, rebase, or switch another workstream's worktree.
