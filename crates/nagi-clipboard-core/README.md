# Nagi Clipboard Core (host-only reference, CLIP-01)

This crate defines the CLIP-01 clipboard / data-transfer content model, a
host-testable service contract, and a deterministic in-memory reference. It is
deliberately outside the root Cargo workspace.

The in-memory reference is **not** production persistence and keeps no
history. Nothing here proves compositor clipboard ownership, IPC transport,
target/QEMU behavior, or first-party adoption.

## Identity and authority boundaries

- Canonical `AppId`, `AppSessionId`, `ExecutionInstanceId`, and `ObjectId` are
  re-used from `nagi-model`. This crate defines no app, session, profile,
  principal, object, or storage-handle identity.
- `CallerContext` is the only source of caller identity. It is passed to every
  operation separately from the request and must be built by the trusted
  transport / Capability adapter after authenticating the channel peer. In
  this host crate its constructor is a documented contract, not a sealed
  token; runtime sealing belongs to the system-service IPC binding.
- `ClaimedOrigin` and metadata inside content are untrusted writer claims.
  They are stored and returned verbatim for display only. `AuthorizationRequest`
  has no field for them, so they cannot influence a decision.
- `VerifiedOrigin` is recorded by the service from the caller at write time.
- `ClipboardAuthorizer` is the injected seam for future Capability
  integration. `Deny` and `Unavailable` fail closed with no state change.
  `DenyAllAuthorizer` is the safe default. No permissive default exists.
- `ClipboardOperation::required_permission` maps reads to `clipboard.read`
  and write/clear to `clipboard.write`, as accepted in
  `docs/decisions/ADR-0012-clipboard-permission-identifiers.md`. Registering
  them in a live `CapabilityRegistry` happens in the trusted host after the
  runtime gate.

## Content model

- `ClipboardContent` is one replacement unit: ordered `ClipboardItem`s, each
  with alternative `Representation`s in writer preference order, a
  `TransferIntent` hint, an untrusted `ClaimedOrigin`, and bounded untrusted
  metadata (deterministic key order).
- `MediaType` accepts only canonical lowercase `type/subtype` (RFC 6838
  restricted names), with no parameters or wildcards. Non-canonical spellings
  are rejected, not normalized.
- `Payload` is `Text` (UTF-8, required for and only for `text/*`), `Binary`,
  or `ObjectReference(ObjectId)`. A reference grants no access to the object.
- `TransferIntent::Move` (cut) is a non-destructive hint. This crate never
  deletes or mutates source data.
- The same content type serves as a generic data offer so a future
  drag-and-drop owner can reuse it.

## Semantics

- Every operation checks `CLIPBOARD_CONTRACT_VERSION` (1), then authorizes,
  then inspects state. A denied caller learns nothing about current content.
- `write` validates the whole content first and then replaces all previous
  content atomically. Nothing is truncated: oversized content is rejected.
- The generation starts at 0 and advances by exactly one on every successful
  write and on every clear that removes content. Clearing an empty clipboard
  is an idempotent no-op. Failed operations never advance it, and it never
  wraps (`GenerationExhausted`).
- `expected_generation` on write, clear, and read rejects stale requests with
  `StaleGeneration { current }`, so a paste never mixes formats from two
  different copies.
- `formats` lists only representations the caller may read
  (`ReadRepresentation` is authorized per format), with kind and size but no
  payload. Restricted formats are neither listed nor readable.

## Bounds

`ClipboardLimits::DEFAULT`: 16 items, 8 representations per item, 1 MiB per
representation, 4 MiB total, 8 metadata entries of at most 256 bytes. Hard
caps (`hard_caps`) bound any configuration. Metadata keys are at most 64 bytes;
the origin label is at most 256 bytes; media types are at most 255 bytes.

## Privacy

- `Debug` output never contains payload bytes, metadata values, or the origin
  label.
- Diagnostics are off unless an owner installs a sink. Events carry only a
  stable `ClipboardEventCode` and the generation. Sink failures never change
  an operation's outcome and are counted in `dropped_diagnostics`.
- Nothing is sent to Activity, Wayback, AI context, telemetry, cloud, or other
  devices.

## Versioned envelope

`encode_content` / `decode_content` implement envelope v1 (`b"NCLP"`, `u16`
version). Unknown versions, bad magic, truncation, trailing bytes, unknown tags,
duplicate metadata keys, invalid UTF-8 or media types, and bound violations are
rejected. Declared lengths are checked before allocation. This envelope is a
content encoding for a future bulk transfer buffer, not the system-service IPC
message format.

## Verification

```text
cargo test --manifest-path crates/nagi-clipboard-core/Cargo.toml --locked --offline
cargo fmt --manifest-path crates/nagi-clipboard-core/Cargo.toml -- --check
cargo clippy --manifest-path crates/nagi-clipboard-core/Cargo.toml --all-targets --locked --offline -- -D warnings
```
