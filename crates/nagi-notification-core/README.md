# Nagi Notification Core (host-only reference)

This crate defines the NOTIFY-01 notification model and a host-testable service
contract. It is deliberately outside the root Cargo workspace. Its in-memory
adapter is test-only reference persistence; it does not prove guest persistence,
service registration, delivery, or UI acceptance.

## Identity and localization boundaries

`NotificationAdapters` resolves the caller's profile from trusted context,
authenticates app/service identities, revalidates a source during action
activation, and authorizes each operation. Profile and source types are opaque
generics, so this crate does not define another `AppId`, `ServiceId`, or
`ProfileId`. The current integration uses test-only string identities. An
approved owner adapter must map canonical identity types when their contracts
are registered and integrated.

Localized content carries the shared `nagi-localization::MessageId` plus
bounded typed arguments. An adapter validates catalog membership and renders
using the selected `LocaleContext`; this crate adds no message catalog entries
or locale policy. User text must carry an explicit sensitivity classification.

## Model and ordering

- Priority order is `Low < Normal < High < Urgent`; severity is separate and
  describes meaning, not scheduling or authority.
- Created and expiry times come from an injected clock. A notification expires
  when `now >= expires_at`.
- Query order is grouping key ascending (ungrouped first), priority descending,
  creation time descending, then NotificationId ascending.
- State is split into read/unread and active/acknowledged/dismissed. Acknowledge
  and dismiss mark an item read. Repeating the same transition is idempotent;
  dismissal retains the record and audit metadata.
- Policy may permit, defer to a future instant, suppress while retaining until a
  future instant, or reject. Provider failure follows the configured reject or
  bounded defer rule; it never becomes implicit permit.
- Action descriptors contain a validated stable action ID, optional localized
  label, and bounded typed parameters. They contain no executable code or
  authority. Activation revalidates the source and capability before checking
  provider availability; the notification service does not execute the action.

## Persistence and privacy

The persistence interface is profile-scoped, versioned, and compare-and-swap
atomic. A failed commit leaves the previous snapshot intact. The reference
store enforces profile count, per-notification bytes, per-profile item/byte/group
limits, total bytes, and query size. At capacity it removes expired entries,
then oldest acknowledged/dismissed entries, then oldest read entries; unread
items are never silently evicted. If it still cannot fit, publish is rejected.

Non-public text and arguments are redacted before storage. Query, UI, and export
apply the redaction hook again. Diagnostics have a typed metadata-only event
shape and cannot carry message content or action parameters. Diagnostics sink
failure does not change notification results. The service attempts at most 128
diagnostic events per 1,000 monotonic milliseconds, exposes a dropped-event
count, and emits a rate-limit summary when the next window opens.

Corrupt or unsupported snapshots enter a read-only recovery state. Only an
explicit migration adapter can move an older snapshot to the current schema.
The in-memory implementation and mock adapters are for contract tests only.

## Deferred integration

NOTIFY-01 is registered in `.dev/workstreams.json` on the dedicated
`codex/0.2-notify-01` branch. M17 is `BLOCKED` and M30 is `NOT STARTED`.
Production persistence, runtime identity/capability/settings wiring, service
registration, app adoption, UI, push, email, SMS, and root workspace changes
remain deferred until M30 passes and the Integration Owner records an explicit
activation checkpoint naming NOTIFY-01. Shared CI changes remain owned by the
CI/integration owner; a narrow host-test proposal is recorded in
`.dev/workstreams/notify-01/registration-proposal.json`.
