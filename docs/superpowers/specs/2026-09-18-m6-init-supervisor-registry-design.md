# M6 Init, Supervisor, and Service Registry Design

## Goal

Extend the M5 user bootstrap with a bounded user-space supervisor and service
registry, then prove the complete path in the real guest by resolving and
calling `echo@1`.

## Constraints

- The kernel remains responsible for execution, memory, IPC, and authority.
- The registry and supervisor are user-space code; no service, manifest, file,
  or process-management syscall is added.
- No host filesystem, socket, process, or hard-coded acceptance response is
  used for guest behavior.
- M5's one-shot `nagi-init` process remains the bootstrap container. M6 adds
  the user-space service protocol and lifecycle state machine; general
  multi-process supervision and persistent manifests remain later work.
- All storage is fixed-capacity and `no_std`; registry calls require a valid
  generation-checked handle returned by name/version resolution.

## Components

### Service identity and manifests

`ServiceId` stores a bounded UTF-8-independent byte name and a `u16` API
version. `ServiceManifest` stores the identity, up to four dependency IDs, and
an explicit `RestartPolicy`. Names are compared as bytes and versions are
never inferred from executable names or PIDs.

### Registry

`ServiceRegistry` owns up to eight entries. Registration rejects duplicate
identities and capacity overflow. `resolve(ServiceId)` returns a
slot-plus-generation `ServiceHandle`; `call(ServiceHandle, request, response)`
rejects stale/invalid handles, unavailable health states, and response buffers
that cannot hold the handler result. A function pointer is the deliberately
small M6 service endpoint abstraction, so the wire boundary is still a
bounded byte request/response and can be replaced by Channel IPC later.

### Supervisor

`Supervisor` stores each manifest's lifecycle state and restart counter. It
computes a deterministic dependency order using a fixed-size Kahn pass,
rejecting missing dependencies and cycles. A service moves from `Starting` to
`Ready` only through the supervisor API, then to `Healthy` when its registered
endpoint is ready. Failed services enter `Restarting` while their finite
restart budget remains; exceeding it enters `CrashLoop` and cannot loop
forever. Host tests cover dependency order, missing/cyclic dependencies,
health transitions, and restart-budget exhaustion.

### Guest path

`nagi-init` creates the supervisor and registry, starts the `echo@1` manifest,
registers a real echo handler, marks the service healthy, resolves it by
`ServiceId`, and calls the returned handle with a bounded request. It verifies
that the response equals the request and emits ordered M6 progress markers
through the existing M5 console syscall. The final process exit remains the
existing native syscall; only after a successful user exit does its kernel
boundary emit the terminal `Nagi M6 acceptance PASS` marker. This keeps the
M6 result contingent on the real user-space registry check while preserving
the M5 acceptance path.

## Error handling

All registry and supervisor operations return explicit errors. No operation
silently replaces an existing service, strengthens a handle, ignores a
missing dependency, or converts `CrashLoop` back to healthy. The guest exits
with the existing nonzero process-exit path after printing an M6 failure
marker if any invariant fails.

## Verification

- `libnagi` host tests verify identifiers, handle generations, bounded calls,
  duplicate/capacity rejection, dependency ordering, health transitions, and
  finite restart policy.
- User-target and kernel/UEFI builds remain required.
- M6 PowerShell and Git Bash acceptance scripts invoke the existing `nagi run`
  flow and require the M5 markers through the FPU round-trip, the ordered
  `echo@1` discovery/call markers, the existing successful process-exit
  markers, and the terminal `Nagi M6 acceptance PASS` in the guest serial
  log.
