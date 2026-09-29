# Nagi OS 0.2 — System Notification Foundation Workstream

## 1. Workstream

- **Requested ID:** NOTIFY-01
- **Registry ID:** notify-01
- **Name:** System Notification Foundation
- **Proposed branch:** codex/0.2-notify-01
- **Recommended worktree:** /Users/tozawa/.codex/worktrees/notify-01/NagiOS
- **State:** .dev/workstreams/notify-01/state.json
- **Target:** Nagi OS 0.2 host-testable notification service contract and reference store
- **Initial status:** NOT_STARTED

The proposal baseline is commit 198b9f60b41eba14e74c2c04f51ccdcb4030adf9. At that baseline M17 is BLOCKED and M30 is NOT STARTED. This is a candidate service foundation; it does not authorize a notification-center UI, production service startup, or 0.2 runtime integration.

## 2. Purpose and background

Define one shared notification model and service boundary for first-party apps and system services. Keep storage, policy, source identity, capability checks, and action descriptors independent of any notification UI or push transport.

The existing language architecture requires user-facing notification strings to use shared localization. Existing app/service identity and capability contracts remain owned by their respective workstreams and must be consumed through adapters rather than redefined here.

## 3. Scope

- Stable NotificationId and source app/service identity reference.
- Typed title/body content that can carry a localized message key plus bounded typed arguments, or explicitly classified user-provided text.
- Priority and severity with documented ordering and policy semantics.
- Created-at and optional expiry timestamps from an injected clock.
- Read/unread, acknowledge, and dismiss lifecycle with idempotent operations.
- Action descriptors with stable action identity and bounded parameters, without UI callbacks, executable payloads, or embedded authority.
- Grouping/threading key and deterministic group ordering.
- User/profile-scoped persistence abstraction and versioned migration boundary.
- Quiet/focus policy hook that can defer, suppress, or permit a notification according to an external policy result without owning Settings.
- Capability hooks for publish, list/read, acknowledge/dismiss, and action resolution.
- Sensitive-content classification and redaction policy before persistence, display adapter, diagnostics, or export.
- Bounded notification store with explicit quota, retention, and eviction/rejection behavior.
- In-memory mock implementation and future UI adapter boundary.
- Structured diagnostics for create, update, policy decision, read/ack/dismiss, expiry, eviction, capacity rejection, and action authorization.

## 4. Non-goals

- Notification center, banner, toast, shell, lock-screen, or system settings UI implementation.
- Push notification cloud transport, mobile push, email, SMS, or delivery provider.
- Owning app/service IDs, user/profile identity, capability policy, localization catalog, or Settings quiet-mode values.
- Executing actions or granting authority from a notification payload.
- Durable exactly-once delivery, cross-device synchronization, remote notification queues, or network services.
- Modifying M18 main/A/B/C, first-party app behavior, or another service's state.

## 5. Dependencies and activation gate

- **Required development contract:** development-foundation DF-01 ownership, state, and verification rules.
- **Source identity:** consume existing AppId and canonical service identity through adapters. SVC-IPC and app lifecycle contracts may be separate or unintegrated in the current checkout; do not introduce a competing AppId/ServiceId.
- **User/profile scope:** use the canonical identity/profile contract if available. Until IDENT-01 is registered and integrated, use an opaque test adapter and avoid persistent production user state.
- **Capability:** capability-permissions owns principal, authorization, and denial behavior. Its current state is PARTIAL; use a deny-by-default adapter/fake until a versioned contract is integrated.
- **Quiet/focus policy:** consume a versioned settings/policy provider when available. The Settings workstream document exists but its current registration/integration must be verified; do not write Settings-owned files or guess a setting key.
- **Diagnostics/localization:** use narrow sink and localized-message adapters. Diagnostics and language architecture own shared event and user-facing string contracts.
- **Before activation:** a service contract, in-memory store, mock policy, and test-only source/action adapters may be prepared only on an assigned branch and approved paths. No UI or product service is activated.
- **Runtime/integration gate:** production persistence, service registration, app adoption, settings/capability enforcement wiring, or target behavior requires Nagi 0.1 M30 PASS and an explicit Integration Owner checkpoint naming NOTIFY-01. A stricter registry gate takes precedence.

Keep hard registry dependencies to registered IDs only. Treat unregistered Identity, Settings, IPC, or app lifecycle workstreams as versioned adapter targets, not implicit dependencies or permission to edit their paths.

## 6. Allowed and forbidden edits

Proposed allowed paths, subject to Integration Owner assignment:

- crates/nagi-notification-core/**
- tests/notification-core/**
- .dev/workstreams/notify-01/**
- this specification

Do not edit without explicit Integration Owner ownership:

- Root Cargo.toml, root Cargo.lock, .github/workflows/**, .dev/workstreams.json, .dev/schemas/**, shared IDL/ABI, localization catalogs, or generated bindings.
- kernel/**, loader/**, user/**, sdk/**, third_party/**, out/**, target/**, docs/implementation_status.md, or any other workstream's source/state/worktree.
- Settings, Identity, Capability, Diagnostics, IPC, app lifecycle, first-party app, M18, or UI implementation paths.
- Any file outside assigned allowed paths.

A standalone host-only crate may keep its own manifest inside the owned path. Required shared source identities, notification schemas, root workspace dependencies, CI jobs, localization message catalogs, or settings keys must be focused proposals under .dev/workstreams/notify-01/.

## 7. Architecture and API/data model

Keep a typed core service with adapters around policy, identity, persistence, diagnostics, localization, and UI:

- NotificationId: validated opaque ID.
- NotificationSource: an adapter-backed reference to an existing app or service identity, plus a safe display attribution. It is not a new app/service registry.
- NotificationContent: localized message reference with typed bounded arguments or classified user text; never use an untrusted payload as a localization key without validation.
- Severity / Priority: separate severity (meaning to the user) from delivery priority (policy scheduling); define stable ordering and tie behavior.
- NotificationPayload: bounded non-executable application data, content classification, action descriptors, grouping key, creation/expiry, and locale context only where appropriate.
- ActionDescriptor: stable action ID and bounded typed parameters. An action handler resolves the ID through an authorized provider at activation time and performs a fresh capability check; serialized payloads never contain closures or grants.
- NotificationState: unread/read plus acknowledged/dismissed lifecycle. Define allowed transitions and idempotency; dismissing a message must not erase required audit metadata.
- NotificationStore: per-profile bounded store, versioned snapshots, atomic mutation, query by source/group/time/state, expiry cleanup, and stable capacity results.
- NotificationPolicy: injected quiet/focus decision returning permit, defer-until, suppress-with-retention, or reject with reason. The notification core does not own settings values or silently discard a notification.
- RedactionPolicy: classifies title/body/action data for storage, display adapters, diagnostics, and export; sensitive content is redacted or omitted at each boundary.
- NotificationService: publish, query/list, mark-read, acknowledge, dismiss, and resolve-action-descriptor APIs with explicit caller context.
- InMemoryNotificationStore and mock adapters: exercise the same public contract without claiming persistence or UI acceptance.
- UIAdapter: future boundary that receives already-authorized, policy-processed, redacted view models and cannot mutate service authority.

Bound per-notification bytes, number of stored items per profile, total store bytes, group count, and query result size. Eviction order is deterministic: remove expired items, then dismissed/acknowledged items, then oldest read items. If capacity remains exhausted, reject the incoming item with a stable capacity error; do not silently evict unread content. If a later approved policy permits unread eviction, it must be explicit, observable, and separately tested.

## 8. Failure and recovery

- Invalid IDs, unsupported schema versions, oversize payloads, invalid localization arguments, and malformed action descriptors are rejected before persistence.
- A failed atomic update preserves the prior snapshot and does not emit a success event.
- Corrupt persisted state is quarantined or reported through the persistence adapter while the service enters a safe read-only/empty recovery state; never invent authority or profile ownership.
- Expired notifications are excluded from normal queries and removed through bounded cleanup.
- Capacity rejection and eviction follow the documented deterministic policy and emit an observable event.
- Quiet/focus policy provider failure follows a configured fail-safe rule, documented per operation; it must not silently grant action authority.
- Action resolution re-checks capability and source availability at activation. A missing or denied provider returns a typed error without executing the action.
- Diagnostics failure cannot change publish, read, dismiss, persistence, or authorization results.
- No network retry or remote delivery fallback exists in this workstream.

## 9. Security, privacy, and capability hooks

- Authenticate the source identity through its owning adapter; a caller-supplied source name is not proof.
- Apply distinct authorization for publish, list/read, mark-read, acknowledge, dismiss, and action resolution.
- Scope queries and mutations to the caller's authorized user/profile and source. Deny missing or inconsistent context by default.
- Re-check permissions when an action is activated; action descriptors carry no capability, token, or executable callback.
- Apply sensitivity redaction before UI adapter delivery, diagnostics, exports, and any non-owner query.
- Enforce title/body/argument/action/store limits before allocation or persistence.
- Never log full sensitive message content by default. Define whether lock-screen/summary adapters may display sensitive content only through an explicit policy decision.
- Cross-profile tests must show no read, update, action, or dismissal leakage.

## 10. Diagnostics and observability hooks

Emit stable structured event codes for publish accepted/rejected, policy permit/defer/suppress, query denied, state transition, acknowledge/dismiss, expiry, migration/recovery, eviction, capacity rejection, redaction, action resolution allow/deny, source unavailable, store failure, and sink failure. Include NotificationId, safe source reference, profile-scoped correlation, and stable reason/severity codes. Do not capture message body, secret action parameters, or unrestricted user data. Bound diagnostic event volume and redact before recording.

## 11. Acceptance Criteria

- [ ] NotificationId, source identity, content, priority/severity, timestamps, state, grouping, and action descriptor types are documented and validated.
- [ ] Publish/query/read/acknowledge/dismiss APIs have typed outcomes and idempotent state transitions.
- [ ] Created/expiry handling uses injected time and deterministic expiry cleanup.
- [ ] Actions are UI-independent descriptors and require fresh provider and capability resolution at activation.
- [ ] Persistence is abstracted, per-profile scoped, versioned, and atomic; in-memory store is clearly test-only.
- [ ] Unread/read state, grouping/threading, and deterministic query order are tested.
- [ ] Quiet/focus policy hook supports permit/defer/suppress decisions without owning Settings.
- [ ] Capability checks deny absent or cross-profile authority; source identity is not caller-asserted.
- [ ] Redaction/sensitive-content policy applies at storage/query/UI/diagnostics/export boundaries.
- [ ] Store quotas and eviction/rejection order are bounded, deterministic, and observable.
- [ ] Diagnostics cover lifecycle, policy, persistence, capacity, privacy, and action outcomes without sensitive message bodies.
- [ ] Mock/in-memory implementation and future UI adapter boundary share the same contract and do not claim product UI/push acceptance.
- [ ] No app/service identity, Settings, Capability, Diagnostics, localization, M18, UI, or unrelated workstream implementation is modified.
- [ ] DF-01 state/proposal ownership is followed; formal registration changes only the `notify-01` registry row, while schemas, root manifests, and shared CI remain untouched.
- [ ] Focused checks, owned-branch commit/push, remote equality, clean worktree, and CI status are recorded when branch/gates permit.

## 12. Tests and CI

Cover ID/source validation, bounded content, localization argument validation, publish/query ordering, unread/read/ack/dismiss transitions, duplicate idempotent operations, expiry at boundary times, group ordering, per-profile isolation, action allow/deny and stale provider, policy permit/defer/suppress/provider failure, redaction by sensitivity class, atomic mutation and corrupt-state recovery, migration, every quota and eviction/rejection case, diagnostics sink failure, and in-memory contract conformance. Use deterministic clock, policy, capability, identity, and persistence fakes; test with network disabled.

Run focused host tests and supported formatting/static checks. Do not edit shared CI. Provide a narrow host-only CI proposal if needed. No notification UI, push, mail/SMS, target, or QEMU acceptance is in scope.

## 13. State, commit, push, and report

Maintain only .dev/workstreams/notify-01/state.json using current DF-01 schema. Record the live activation gate, actual status, checked commit, commands/results, CI run/head SHA, failure evidence, attempted/prohibited fixes, acceptance checklist, exact dependency versions and integration status, deferred items, and next action. If unregistered, prepare a focused registration proposal and record that the root verifier does not include it. Host mocks cannot be recorded as production persistence, UI, or delivery PASS.

Commit only assigned files on the approved branch; push only that branch and verify remote HEAD equality, clean/dirty status, and CI conclusion. If assignment, gate, or remote access blocks the push, finish authorized local work, document the exact blocker and Integration Owner action, and do not push to main or another workstream branch.

The final report must state workstream ID, branch, final HEAD SHA, remote equality, worktree clean/dirty, notification model/API changes, Acceptance results, tests/CI, remaining blockers, and exact Integration Owner next action.

## 14. Git and worktree safety

Before editing, inspect AGENTS.md, this specification, relevant source contracts and states, registry, implementation, Git status/diff, branch, HEAD, remote, and git worktree list. Reuse an existing dedicated NOTIFY-01 worktree if present. If absent, use only the approved owner-assigned branch/base procedure. Preserve every other worktree and never reset, clean, force-checkout, rebase, or switch another stream.
