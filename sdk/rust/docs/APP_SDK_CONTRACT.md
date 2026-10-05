# Nagi App SDK and First-Party App Contract v1

## Purpose and boundary

`nagi-sdk::app_contract` defines host-testable contracts shared by Nagi-owned
and third-party apps. It is transport independent and currently runs only in
host tests and the `nagi-pkg manifest validate` command. It does not launch an
app, access host files or devices, allocate capabilities, or connect to Nagi's
runtime.

The contract is additive to M16. M16 `.xapp` package signing, side-loading,
atomic replacement, `application@1` IDL, generated bindings, and its current
manifest parser remain in place. Manifest v1 is a separate contract document;
no existing package is silently reinterpreted. Actual app migration waits for
Nagi 0.1 M30 PASS and an explicit 0.2 integration checkpoint.

## Validate an SDK fixture

The host loader consumes the JSON v1 manifest and validates it against the
typed public SDK contract:

```sh
cargo run --locked --manifest-path tools/nagi-pkg/Cargo.toml -- \
  manifest validate sdk/rust/fixtures/manifests/notes.json
```

The command only validates metadata. It never launches the declared entrypoint.
`nagi-pkg` rejects manifests larger than 1 MiB, malformed JSON, unknown
unscoped properties, unsafe package-relative entrypoints, unsupported versions,
duplicate declarations, and invalid locale/resource/route metadata.

For independent JSON Schema Draft 2020-12 validation of the examples:

```sh
python3 -m venv /tmp/nagi-app-sdk-schema
/tmp/nagi-app-sdk-schema/bin/pip install -r tools/nagi-pkg/requirements-test.txt
/tmp/nagi-app-sdk-schema/bin/python tools/nagi-pkg/schema_validation/validate_fixtures.py
```

The four examples are SDK-owned fixtures only. They do not copy or alter Albert,
Files, Notes, Search, or Activity app source.

## Manifest v1

The canonical JSON schema is
[`schemas/app-manifest.schema.json`](../schemas/app-manifest.schema.json).
Every manifest has:

| Field | Meaning |
| --- | --- |
| `schemaVersion` | Serialized manifest shape; v1 accepts exactly `1`. |
| `sdkContractVersion` | Required public API major/minor. Host requires the same major and a host minor at least this high. |
| `id` | Stable reverse-domain ASCII app identifier, independent of display names and package paths. |
| `version` | App's semantic version. |
| `origin`, `publisherId` | First-party/third-party classification and optional publisher identity; first-party metadata includes a publisher. |
| `displayName` | `en-US` name and optional `ja-JP` name. `ja-JP` must be present when Japanese is declared supported. |
| `supportedLocales` | Unique BCP-47-style tags. `en-US` is required as the fallback locale. |
| `entrypoint` | `native` or `portable` artifact locator relative to the package. It is not an OS path and not app identity. |
| `icon`, `resources` | Optional `appres://` package resource references. They never grant filesystem access. |
| `intents` | Unique intent identifier, schema version, payload type, and optional one-segment route ID. |
| `requestedCapabilities` | Declarative capability IDs and optional localized purpose keys; declarations do not grant rights. |
| `backgroundServices` | Reserved, validated metadata for future host support; v1 does not schedule or execute services. |
| `stateCompatibility` | Current persisted state version and the oldest version this app can read/migrate. |
| `extensions` | Optional namespaced vendor data. Root fields remain strict; additions go here. |

The manifest's exact stable `id` string is the canonical package-facing
identity. Existing M16 `AppId` is retained and derived from that string for
compatibility with current model handles. It is a 64-bit non-cryptographic
identifier, so registration must compare canonical identifiers and reject
collisions before using numeric IDs as keys; numeric IDs do not grant authority.

## Identity and localization

`AppIdentity` contains stable identifier, app version, origin, optional
publisher identifier, and the existing model `AppId`. Display strings,
filesystem locations, app sessions, execution instances, nodes, and surfaces
are separate values. Its fields are private and construction validates the
canonical identifier, derived `AppId`, version, and publisher. Changing locale
or display name cannot change identity.
Publisher identity can be absent for a third-party app; the manifest loader
requires it for first-party manifests.

`DisplayName::resolve` resolves `en-US` directly. It resolves `ja-JP` directly
when the manifest declares it supported and includes its display string.
Unknown or unsupported locales fall back to `en-US` and set
`used_fallback = true`. English and Japanese are both explicit first-class v1
strings; unknown locales do not alter app identity. The contract does not
provide a translation database or infer a user's system, region, keyboard, or
conversation language.

## Lifecycle

`LifecycleState` is current state; `LifecycleEventKind` describes an event or
request. `LifecycleMachine::apply` validates transitions and leaves state
unchanged on invalid events. Launch advances through `Registered` and
`Launching`; the app cannot become `Ready` until the capability boundary has
resolved. Foreground/background, suspend/resume checkpoint context, graceful
termination request/completion, and abnormal termination are represented.
Abnormal termination enters terminal `Crashed` state.

Events carry `AppSessionId` and optional `NodeId` plus optional sequence
metadata. One lifecycle machine binds to its first accepted session ID. If
sequence metadata is used, later events must include a strictly increasing
sequence. Node context is informational and may change across host placement;
it is not a lifecycle identity or affinity rule. This permits future
multiple-session/node adapters without defining or implementing a multi-device
runtime. A host adapter should deliver
`TerminationRequested`, invoke the app's `GracefulShutdownHook`, and report
`ShutdownHookCompleted` only if the callback succeeds. The state machine
rejects `Terminated` until that completion event. Hook failure or timeout must
be reported as failure/abnormal termination; timeout/crash policy belongs to
the host.

## State persistence

`AppStateNamespace` separates durable app state (`session_id: None`) from
session-scoped state (`Some(session)`). `AppStateBackend` owns load/save/reset;
its implementer must save atomically and make reset idempotent. The SDK does not
choose a filesystem or provide history/undo storage.

`restore_state` distinguishes missing and corrupt state, rejects invalid,
future, and below-minimum-readable versions, returns same-version data, and
invokes a `StateMigrator` for readable older state. It persists the new version
only after successful migration. Migration failure does not overwrite stored
bytes. The application declares `currentVersion` and
`minimumReadableVersion`; it should only lower that minimum when its migration
implementation can safely read every intervening format.

Host tests use deterministic memory backends. A runtime storage adapter must
key state by stable app identity and the requested app/session namespace, keep
failure/corruption distinct from missing data, and use atomic replacement.
Wayback, Activity, and undo ledgers remain separate owners.

## Intents and deep links

An `Intent` carries identifier, version, payload type and opaque payload bytes,
source app, optional target app, and `CorrelationId`. The target validates it
against declarations from its already validated manifest. Unsupported IDs,
versions, payload types, and mismatched targets fail closed.

The optional human/shareable route form is
`nagi://<stable-app-id>/<route-id>`. It is limited to one declared route
segment. Query, fragment, percent-encoded data, external schemes, and path
traversal are rejected. Object IDs and user data travel in the typed payload,
not in a route string. An intent without a deep-link route is delivered without
a URI.

## IPC and common errors

`IpcEnvelope` describes request, response, event, and error messages without
choosing sync/async delivery or a transport. NIPC v1 is a little-endian binary
encoding with magic `NIPC`, explicit version, source/destination, message kind,
correlation and optional request IDs, optional timestamp/sequence, payload type,
and payload. Payloads are capped at 1 MiB. Request and response messages
require a request ID; events and errors may omit it. Decoders reject unknown
versions, kinds, flags, inconsistent optional fields, malformed lengths, and
trailing bytes. Timestamp/sequence fields provide metadata only; the transport
adapter defines ordering and delivery guarantees.

`ErrorEnvelope` carries a stable numeric `ErrorCode`, correlation ID,
retryability, and optional localization key. It contains no localized text.
Codes cover invalid manifests/versions/lifecycle, unavailable or corrupt state,
migration, intent/routes, IPC, permission denial, unavailable runtime,
bounded-buffer failures, duplicate app registration, numeric identity
collision, and registration capacity. Adapters can propagate
`PermissionDenied`; SDK code does not decide policy. `RuntimeUnavailable`
describes the unconnected runtime boundary. No absent adapter reports success.

## Host-side registration

`AppRegistry` is a small fixed-capacity, caller-backed table for deterministic
host validation. It accepts only validated `AppIdentity` values, detects a
duplicate canonical identifier, and rejects two different identifiers that
derive to the same numeric `AppId`. It does not discover packages, persist an
installed-app database, launch apps, or assign authority. The host manifest
loader validates the complete document before a consumer reconstructs and
registers its identity.

## Capability boundary

The manifest's requested capability list is converted to
`CapabilityRequest` values. `CapabilityResolver` is an interface owned by the
future host/capability service. `resolve_capabilities` hands the request list
and app identity to it, propagates denial, and rejects malformed requests or an
invalid resolver result. It does not create policy, issue handles, widen rights,
or assume app owner authority. The lifecycle only reaches `Ready` after a
resolved or empty request set is reported.

## Independent version policy

These numbers have separate purposes and migration rules:

| Version | v1 policy |
| --- | --- |
| Manifest schema | Exact supported shape `1`; unsupported shape fails closed. Additions use the namespaced `extensions` object. |
| Manifest semantic validation | `nagi-pkg manifest validate` runs typed semantic checks after JSON parsing. The JSON Schema handles structural constraints; duplicate declaration IDs and `minimumReadableVersion <= currentVersion` are additionally enforced by the semantic validator because portable JSON Schema cannot express these checks. |
| SDK contract | Same major; requested minor must be no greater than host minor. Breaking APIs increment major. |
| IPC protocol | Exact protocol `1`; transport adapters negotiate/support a version explicitly and return mismatch otherwise. |
| App state | Per-app `currentVersion` and `minimumReadableVersion`; migrate only readable older state and never silently downgrade future or corrupt state. |

Changing one version does not implicitly change the others. A contract change
requires a version/migration note, fixtures, negative cases, and consumer
compatibility tests before integration.

## Runtime adapter points

Runtime connection remains blocked until M30 PASS and an explicit integration
checkpoint. At that checkpoint a host owner can implement the following
adapters against the public contract:

| Runtime responsibility | Contract point |
| --- | --- |
| Register/validate package metadata | Manifest v1 schema and `AppManifestContract`; preserve M16 package verification. |
| Deliver lifecycle events and await shutdown hooks | `LifecycleMachine`, session/node event context, host process supervision. |
| Deliver IPC | Encode/decode `IpcEnvelope`; a separate transport supplies delivery/order guarantees. |
| Persist state | `AppStateBackend` and atomic app/session namespaces. |
| Resolve permissions | `CapabilityResolver` implemented by the independently owned permission service. |
| Present a window/surface | Future host mapping from app session to compositor `SurfaceId`; the SDK does not allocate windows. |
| Execute declared native/portable code or services | Future host-owned process/runtime policy; manifest metadata alone never starts code. |

Until those adapters exist, this crate and `nagi-pkg manifest validate` are
contract tools only. `RuntimeUnavailable` is a failure result, not a fake
launch or rendering response.

## First-party adoption guide

Albert, Files, Notes, Terminal, Activity, Search, and other first-party apps
can adopt this contract after the integration gate without changing the owner
of their runtime responsibilities:

1. Keep each existing signed `.xapp` package and M16 manifest/parser intact.
   Add a manifest-v1 document beside it during the compatibility period; do
   not use localized names, install paths, or window IDs as app identity.
2. Declare the app identifier, publisher/origin, semver, entrypoint locator,
   `en-US`/`ja-JP` display strings, resource references, intents, requested
   capabilities, and state compatibility range. Use the SDK-owned fixture
   closest to the app as the initial contract test.
3. Replace app-local lifecycle enums with lifecycle events delivered by the
   host adapter, while keeping UI-specific view state private. Complete the
   shutdown hook before acknowledging graceful termination.
4. Replace arbitrary app-owned persistence paths with the injected
   `AppStateBackend`; define versioned state and migrations before changing its
   encoding. Keep semantic undo/history in its owning service.
5. Express cross-app operations as typed intents and keep object IDs in payload
   data. Declare needed capabilities, handle asynchronous denial, and never
   assume a declaration is a grant.
6. Add consumer tests for manifest load, locale fallback, lifecycle denial,
   state corruption/migration, intent rejection, and IPC mismatch. Validate
   signed-package behavior separately with the existing M16 acceptance.
7. Migrate only in an approved runtime integration checkpoint after M30 PASS;
   do not expose these contracts to a production app host before that gate.

The fixtures under `fixtures/manifests/` demonstrate Files-like resource open,
Notes-like note open plus deferred service metadata, Search-like route-free
query, and Activity-like entry display. They contain metadata only and do not
ship or run application binaries.
