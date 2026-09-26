# Nagi Capability and Permission Foundation

This crate provides a host-side capability declaration model, a deterministic
policy evaluator, versioned policy serialization, and a persistence interface.
It does not connect to Nagi runtime services or grant kernel handles.

## Code map

- **src/ids.rs** validates stable capability, principal, and session identifiers.
- **src/declaration.rs** parses version 1 manifest declarations and binds them
  to a principal supplied by a trusted package or service host.
- **src/model.rs** defines principal metadata, resource scopes, grant effects,
  lifetimes, sources, and revocation/consumption state.
- **src/registry.rs** accepts explicit namespace registrations. It contains no
  built-in allow list or wildcard namespace.
- **src/store.rs** defines the atomic policy transaction boundary and a
  deterministic in-memory store. The JSON representation has an explicit
  schema version and rejects malformed data.
- **src/evaluator.rs** implements default-deny grant, deny, revoke, check, list,
  explain, and one-shot consumption behavior.
- **schemas/capability-declaration-v1.schema.json** is the JSON Schema for
  requested capabilities.
- **tests/permission_flow.rs** exercises manifest load through grant, check,
  revoke, and principal isolation.

## Namespace and identifiers

Capability identifiers are lowercase dotted namespaces such as
filesystem.read, network.client, and camera.capture. Syntax validation does
not register a capability. A trusted host must add a definition to
CapabilityRegistry; otherwise evaluation denies it. New identifiers are added
by a registry owner without changing the identifier grammar.

Principal IDs are stable host-issued identities. Principal kinds include
first-party apps, third-party apps, system services, AI-mediated actions, and
automation. AI is represented as an ordinary principal kind; it gains no
special authority. Publisher, package, and display metadata do not replace the
stable principal ID.

The declaration schema intentionally omits a trusted principal identity.
Manifest fields are attacker-controlled. A trusted package or service host
binds parsed requests to the principal it resolved from installed identity,
as shown by bind_declaration.

The current package manifest in `user/nagi-package` is line-based and remains
unchanged. This JSON declaration is a separately versioned permission
extension point; it is not yet embedded in or consumed by that manifest.

Version 1 rejects unknown fields and unknown schema versions. Future fields
require a schema version change and compatibility tests. Unknown capability
names remain parseable declarations but cannot pass policy evaluation unless
they are registered.

## Scope model

Scopes support unscoped access, logical filesystem paths, canonical network
origins, device classes, model-provider classes, automation targets, and
opaque namespace/resource pairs. Unscoped means only unscoped; it is never a
wildcard. Filesystem grants cover the granted path and its descendants using
path-segment boundaries. Other scopes match exactly. A narrower grant never
covers a broader request.

This model validates canonical input but does not perform host filesystem
resolution, DNS checks, URL routing, device discovery, provider discovery, or
automation target lookup. Those parsers and resource resolvers belong to their
service adapters and must pass the resolved scope back to the evaluator.

## Grant lifecycle and decisions

An explicit user, system, or policy source creates an allow or deny grant for
one principal, one registered capability, and one scope. A request alone never
creates a grant. A matching explicit deny takes precedence over any allow.
Revocation timestamps remain in the record for auditability; revoked grants
stop authorizing immediately.

One-shot allows are consumed in the same policy-store transaction that
returns Allow. Session grants require the matching session ID. Persistent
grants remain active until revoked or expired. Expiry uses caller-supplied Unix
seconds so tests stay deterministic. A grant is inactive before its recorded
grant time and at or after its expiry time. explain is read-only; check consumes
one-shot grants. One-shot deny records are rejected rather than left active
without a well-defined consumption rule.

Decision explanations are deterministic English diagnostics. They identify
the policy reason without treating a request, manifest, AI output, or grant
identifier as authority.

Grant, deny, and revoke mutate policy and belong only to a trusted permission
management service. Application code receives the narrow enforcer contract;
it must not receive policy-administration access or choose a trusted source.

## Persistence contract

PolicyStore is the persistence interface. Implementations must atomically
commit each transaction and must not report success before durable commit.
That requirement is security-critical for one-shot grants: concurrent checks
must not replay a consumed grant. InMemoryPolicyStore is for deterministic
host tests only; it is not a persistent product backend.

PolicyDocument uses schema version 1, validates duplicate IDs, grant state,
scope, timestamps, and the next-ID counter on load, and rejects unsupported
versions. Store read/transaction errors produce a deny decision from the
evaluator. No system settings database or filesystem store is selected here.

## Runtime integration boundary

CapabilityEnforcer is the future service-facing authorization contract.
This workstream provides no implementation for kernel, process launch,
compositor, Servo, filesystem, network, audio, camera, microphone, model
manager, or automation runtime hooks.

After Nagi 0.1 M30 PASS and an explicit 0.2 integration checkpoint, the
integration owner must:

1. Bind trusted process/app/service identity to PrincipalId.
2. Resolve each operation to a registered CapabilityId and canonical
   CapabilityScope.
3. Call the enforcer at the owning user-space service boundary before the
   side effect.
4. Preserve deny-by-default behavior on unknown capabilities, malformed
   policy, missing stores, stale sessions, and scope mismatch.
5. Define a durable PolicyStore adapter with atomic one-shot consumption and
   version migration before enabling persistent grants.
6. Add service-specific negative tests proving denied operations do not reach
   their backend.

No runtime operation is considered enforced by this host-side crate.

## Threat and misuse assumptions

- Package declarations, publisher metadata, display strings, AI output, and
  requested scopes are untrusted input.
- The service host, capability registry, trusted identity resolver, clock
  provider, and future durable store are trusted inputs and require separate
  review.
- The permission-management service is the only caller authorized to use
  grant, deny, or revoke operations; runtime callers receive only the
  authorization contract.
- Principal IDs are not secrets or bearer tokens. Possessing an ID does not
  itself grant authority.
- A runtime adapter must derive identity from a trusted execution context; it
  must never accept a manifest-supplied principal as the caller identity.
- Persisted policy is untrusted until its version and every grant validate.
- A storage error, unknown capability, malformed scope, missing grant, or
  mismatched principal is a denial.

## Verification

Run the crate in its isolated Cargo workspace:

    cargo fmt --manifest-path crates/nagi-capability/Cargo.toml -- --check
    cargo test --manifest-path crates/nagi-capability/Cargo.toml --locked
    cargo clippy --manifest-path crates/nagi-capability/Cargo.toml --all-targets --locked -- -D warnings

The isolated workspace is deliberate: DF-01 reserves the root Cargo.toml and
Cargo.lock for the integration owner. The integration checkpoint must add
this crate to the root workspace and reconcile the shared dependency lock
before consumers import it.
