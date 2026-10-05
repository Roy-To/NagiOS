# ADR-0012: Clipboard Permission Identifiers

- Status: Accepted for Nagi 0.2 host foundations (runtime registration gated)
- Date: 2026-10-02

## Context

The 0.2 master specification (section 10.1) names `clipboard.read` as a
user/data permission. The CLIP-01 clipboard foundation also needs authority
for replacing and clearing clipboard content. It mapped those operations to a
*proposed* `clipboard.write` identifier and asked the capability-permissions
owner to accept it.

`nagi-capability` has no built-in permission catalog. Its `CapabilityRegistry`
holds definitions that a trusted host registers at runtime, and evaluation
denies any identifier that is not registered. "Registering" a new permission
before the runtime gate therefore means fixing the identifier and its meaning,
not adding a global allow list.

## Decision

Accept two distinct, unscoped clipboard permissions:

| Identifier | Covers (CLIP-01 operations) |
|---|---|
| `clipboard.read` | `ReadGeneration`, `ReadFormats`, `ReadRepresentation` |
| `clipboard.write` | `Write` (replace all content), `Clear` |

- They are independent. Neither implies the other, and holding both grants
  nothing beyond each one.
- Clear is part of `clipboard.write`, because clearing is a replacement with
  empty content. No separate `clipboard.clear` identifier exists.
- Both use `CapabilityScope::Unscoped` for the single system clipboard.
  Representation-level restriction (for example a format only some readers may
  see) is decided by the clipboard service's `ClipboardAuthorizer` policy, not
  by new identifiers.
- Neither permission grants authority over the source of a `Move` (cut)
  intent, over an `ObjectReference` payload's target object, or over clipboard
  history (none exists).
- Caller identity comes from the trusted service boundary, never from claimed
  origin data in clipboard content.

## Consequences

- `ClipboardOperation::required_permission` in `nagi-clipboard-core` returns
  these accepted identifiers instead of a proposal.
- `crates/nagi-capability/tests/permission_flow.rs` proves the identifiers are
  valid, default-deny until registered and granted, and independent.
- Runtime registration into a live `CapabilityRegistry`, manifest/consent UI,
  and binding the clipboard authorizer to the Capability enforcer stay gated on
  Nagi 0.1 M30 PASS and an explicit checkpoint naming CLIP-01.
- Whether Nagi should ship a canonical built-in catalog of standard permission
  identifiers is a separate, open decision. This ADR does not create one.
