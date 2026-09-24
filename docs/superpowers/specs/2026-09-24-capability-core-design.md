# Capability Core Implementation Design

## Problem and intended outcome

Nagi already has a bounded M11 user-space Permission Broker in `libnagi`, a
fixed-format package manifest in `nagi-package`, and shared application
identity types in `nagi-model`. M11 proves a narrow login/session permission
path, but it does not provide a reusable capability vocabulary, scoped grants,
AI delegation, a persistent-store contract, or audit records for future apps
and services. This change adds those reusable semantics while leaving M11's
guest behavior and in-progress OS/application work untouched.

## Scope and non-goals

- Add a `no_std`, allocation-free `nagi-security` core crate for capability
  identities, actors, scopes, decisions, policy evaluation, delegation,
  versioned record encoding, an in-memory reference store, and audit events.
- Extend the existing `nagi-package` manifest with bounded, repeatable
  capability declarations. Existing manifests without declarations remain
  valid; malformed declarations fail parsing.
- Re-export the public model through `nagi-sdk` and provide one standalone
  Notes manifest fixture.
- Add focused unit, codec, evaluator, and package-manifest tests plus a short
  architecture document and a capability-core-only status entry.
- Do not change kernel, M11 runtime behavior, Servo/Mesa/relibc, compositor,
  graphics, or first-party application implementations.

## Proposed design

The new crate is separate from `nagi-model`: that crate owns shared identity
and presentation data, while capability policy is a distinct security
boundary. The core stays `no_std` and uses bounded fixed-size values so the
same policy types are usable by host tests and target services without a new
runtime dependency.

Capability IDs use validated canonical ASCII names and are matched exactly.
An extensible built-in registry contains the Nagi 0.1 capability names;
unregistered names and requests with no matching policy are denied. A scope is
one of unrestricted, an object, a directory, a resolved object within an
attested directory, a domain, localhost, a device class, or a device.
Matching is conservative: exact opaque IDs are required for object,
directory, and device scopes; an object-in-directory request is covered by a
directory grant only when the trusted resource service attests that same root.
The core does not infer filesystem ancestry from paths. Domain requests may
only narrow beneath a granted domain.

Policy rows are keyed by a typed actor, capability, and scope and contain
Allow/Deny/Ask plus a background-use bit. Evaluation is deterministic and
read-only. Deny overrides Ask, Ask overrides Allow, and missing policy denies.
The store trait supports lookup, set, revoke, and enumeration; a bounded
in-memory implementation supplies a reference backend. Grant/delegation
records have a versioned, bounded binary encoding with round-trip tests.

Actors distinguish users, system services, first-party apps, third-party
apps, AI agents, and background automation. AI suggestions cannot execute.
AI execution requires a separately stored user delegation that matches the
exact agent, capability, scope, and expiry, plus an Allow policy for the
delegating user. Background execution must be enabled by both policy and
delegation. Destructive and privileged requests return Ask even when a stored
grant matches, leaving action-specific confirmation to a future trusted UI.
No actor class receives implicit authority.

The existing package manifest remains the source of application identity and
gains repeatable declarations in this bounded line format:

```text
capability=<capability-id>|required-or-optional|<scope-template>|<reason-key>
```

Supported initial templates are `any`, `selected-object`,
`selected-directory`, `domain:<domain>`, `localhost`, and
`device-class:<class>`. Reasons
are localization keys, not user-facing strings. Capability declaration count,
field lengths, syntax, and duplicate identities are bounded and validated;
each package may declare up to eight capabilities.
The package parser continues to accept old manifests with no capability rows.

Audit events contain actor, capability, requested scope, decision, reason,
timestamp, and correlation/action ID. They are emitted as values only and do
not depend on Activity Ledger storage.

## Compatibility and error handling

Existing package IDs, package version, and M11 behavior remain unchanged.
Unknown manifest keys, malformed capability rows, invalid scope/domain
syntax, duplicate capability declarations, unsupported record versions, and
truncated/oversized encodings fail closed. Grants are never widened during
matching or decoding.

## Verification plan

- Run focused `cargo test -p nagi-security` and `cargo test -p nagi-package`.
- Run `cargo test -p nagi-sdk` and `cargo check -p nagi-init` to validate the
  shared SDK/package edges without building the blocked M17 target.
- Run formatting and Clippy for the new crate and changed shared crates.
- Run the repository's safe `./nagi doctor` if its read-only host checks are
  available; do not run M17 build/acceptance or alter its blocker.
- Review final worktree and commit only the dedicated capability-core branch.
