# Nagi App SDK / First-Party Contract Foundation

## Problem and outcome

M16 provides a bounded `.xapp` package parser, generated `application@1` bindings, a stable `AppId`, and basic session/presentation types. It does not define a common manifest v1, lifecycle, durable app state, inter-app intent, transport-independent IPC envelope, or a contract-level capability handoff. Add these contracts as an additive SDK layer that can be validated on the host.

## Scope

- Add an independently versioned App Manifest v1 schema and SDK contract types for identity, localization, host-side registration, lifecycle, state, intent/deep links, IPC envelopes, common errors, and capability request handoff.
- Provide deterministic host fixtures/tests and a strict manifest loader/schema-validation path.
- Document adapter interfaces and first-party adoption steps.
- Keep every implementation change inside `sdk/**`, `tools/nagi-pkg/**`, and this workstream's state/handoff directory.

## Compatibility and non-goals

Keep M16 package signing, side-loading, atomic replacement, `AppId`, `AppSessionId`, presentation types, `application@1`, and generated bindings unchanged. The v1 manifest contract is additive; it does not replace or silently reinterpret the existing key/value package manifest. Manifest schema, SDK contract, IPC protocol, and persisted-state versions evolve independently.

Do not change kernel ABI, runtime app launch, real applications, Capability policy, Wayback, M17, M18-M30, root Cargo manifests/lock, shared IDL, workflows, or third-party sources. Do not bind a transport or host filesystem. No runtime adapter returns synthetic success: an absent adapter reports `RuntimeUnavailable`. Runtime connection remains gated on M30 PASS and an explicit 0.2 integration checkpoint.

## Design

Use the existing logical `AppId` plus stable reverse-domain app identifier and optional publisher identity; display strings are localized data, never identity. The caller-backed host registry detects duplicate IDs and numeric collisions without adding persistence or package discovery. Keep lifecycle state separate from events, bind each machine to one session, and require explicit validated transitions and increasing sequence metadata when used. Represent capability declarations as identifiers and handoff results only; a resolver adapter owns policy. Expose app-scoped, versioned state through a backend trait with explicit missing/corrupt/reset outcomes and a migration hook. Model intents and routes as typed identifiers, payload type/version metadata, source/optional target, and correlation IDs. Define IPC v1 as a bounded binary envelope independent of transport and reject unsupported protocol versions.

For language lookup, `en-US` and `ja-JP` are explicit supported locale tags; lookup prefers an exact supported locale and falls back to required `en-US`. Unknown locale tags use the same fallback and report that fallback occurred.

## Error handling

Use stable English machine-readable error codes for invalid manifests, incompatible contract versions, lifecycle transition errors, missing/corrupt/unavailable state, migration failures, unsupported intents, IPC mismatch, permission denial, and unavailable runtime adapters. Permission denial is received and propagated, never decided by this SDK layer.

## Verification

Unit tests cover manifest load/rejection, identity/locales, lifecycle transitions, state round-trip/corruption/migration/reset, intent routing/validation, IPC round-trip/version mismatch, and errors. Integration fixtures exercise registration, launch, capability handoff, ready, intent, state save, suspend/resume/restore, and termination entirely on the host. Validate every JSON fixture against the schema, run focused Rust tests, fmt, Clippy, existing M16 package regression tests, and DF-01 `dev status`, `dev resume`, and `dev verify`.

## Implementation decisions

- The implementation is in `nagi-sdk::app_contract`, with the versioned schema,
  four SDK-owned fixtures, a strict host manifest loader in `nagi-pkg`, and a
  host consumer flow. The M16 `.xapp` parser remains separate and unchanged.
- Manifest v1 supports package-relative native/portable entrypoint locators,
  `appres://` resources, strict root fields plus namespaced extensions, and a
  1 MiB host-loader size limit. Unsupported schema shapes fail closed.
- JSON Schema validates structural constraints; the host semantic validator
  additionally checks cross-field rules such as minimum readable state version
  not exceeding current state version.
- The canonical app ID remains the manifest string; the existing 64-bit
  `AppId` derivation is retained for M16 compatibility. A fixed-capacity
  `AppRegistry` rejects duplicate registrations and numeric collisions before
  a host uses an `AppId` as a package key.
- State restoration receives both `currentVersion` and
  `minimumReadableVersion`; corrupt, future, and too-old state is reported
  rather than silently reset or downgraded.
- Two integration-owned boundaries are carried only as focused feature-branch
  proposals: the appended `.dev/workstreams.json` row needed to track this
  stream, and `tools/nagi-pkg/Cargo.toml` / its lock update for the host JSON
  parser. Before integration, the owner should create a checkpoint, confirm
  the dependency and registry consumers, and record any migration/review plan.
  No shared file was changed on main or the DF branch.
