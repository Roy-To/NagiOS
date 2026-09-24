# Capability and Permission Core

`nagi-security` provides reusable user-space policy types for Nagi services,
the Rust SDK, and package manifests. It is a deterministic policy layer; it
does not mint kernel handles, execute actions, or replace the existing M11
login/session broker.

## Request model

A request names an actor, a canonical capability ID, a resource scope, and an
invocation context. Capability IDs use stable English names such as
`files.read`, `network.access`, and `ai.action.execute`. The built-in catalog
assigns a risk class. Unknown names are denied even if an old store row exists.

Actors distinguish users, system services, first-party apps, third-party apps,
AI agents, and background automation. System identity and first-party status
are descriptive only; every actor needs an explicit policy row. A typed
invocation context distinguishes direct user actions, apps, AI suggestions,
delegated AI actions, automation, and system services.
Capability IDs are classified as resource permissions, platform participation,
or system authority; the classification does not grant access.

Scopes are `Any`, an opaque object or directory ID, a resolved object within
an attested directory, a domain, a device class, localhost, or a specific
device. A request must be contained by its grant. Object and directory IDs
match exactly. A `Directory(root)` grant also covers an
`ObjectWithinDirectory { root_directory: root, object }` request. The trusted
resource service must resolve the object and attest its directory membership
when constructing that scope; the policy core does not infer ancestry from
paths or accept caller-provided path relationships as evidence. An object
grant for the same opaque object ID also covers that object request regardless
of the containing directory. Domain scopes can narrow from a granted domain
to one of its subdomains.
Domain names are bounded to 127 ASCII bytes in this preview; longer names are
rejected during parsing.

## Decisions and evaluation

`PermissionDecision` is `Allow`, `Deny`, or `Ask`. No matching row denies.
For overlapping rows, `Deny` overrides `Ask`, and `Ask` overrides `Allow`.
Background requests require a grant with background use enabled. Destructive
and privileged built-in capabilities return `Ask` even after a matching
stored allow; a future trusted UI can supply action-specific confirmation.

Evaluation is read-only and deterministic. The in-memory reference store
implements the `PermissionStore` interface for current-decision lookup, set,
revoke, and grant enumeration. A persistent service can implement the same
interface. Permission and delegation records have versioned, bounded binary
encodings; unsupported versions and malformed records fail closed.

## App manifest

The existing package manifest keeps its identity and package fields and may
include up to 8 repeated capability declarations:

```text
capability=files.write|required|selected-object|notes.permissions.write_document
```

The four fields are capability ID, `required`/`optional`, scope template, and
localization reason key. Initial templates are `any`, `selected-object`,
`selected-directory`, `domain:<domain>`, `localhost`, and
`device-class:<class>`. Older
manifests with no declarations remain valid. Malformed or duplicate
declarations fail parsing. A syntactically valid but unknown capability can
be represented for forward compatibility; policy still denies it until Nagi
registers that capability.

## AI delegation and audit

AI suggestion-only requests cannot execute. An AI agent acting for a user
needs a stored, revocable delegation bound to that exact agent, user,
capability, scope, expiry, and background-use setting. The evaluator also
requires an Allow row for the delegating user. Delegation does not inherit or
strengthen the user's authority. High-risk operations still return `Ask`.

Each result can be converted to an `AuditEvent` containing actor, capability,
requested scope, decision, reason, timestamp, and correlation/action ID. An
`AuditSink` interface lets a trusted service forward it to Activity Ledger
without making this crate depend on the ledger or its storage.

## Extension points

Add capabilities through the built-in catalog with an explicit risk class;
add scope forms only when their containment rule is deterministic; and
implement durable storage behind `PermissionStore`. Kernel enforcement,
trusted confirmation UI, app-signature trust, and Activity Ledger persistence
remain separate integrations.
