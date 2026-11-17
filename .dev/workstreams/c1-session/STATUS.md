# C1 Session workstream status

Status as of 2026-11-17. This workstream remains in progress.

## Implemented in this branch

- Preserved the `ce0ec0b` / `2239981` source-only closure in the initial branch commit. It contains no inherited Git history.
- Added `InteractiveSessionContext`, issued by the current C1 service and rechecked against its trusted caller, live lease, expiry, and committed snapshot. The type does not implement `Send`, `Sync`, or serialization.
- Added the session-end adapter with opaque recipient-bound delivery, a stop latch before lifecycle callbacks, restricted retry after known pre-effect errors, and one-shot cleanup request followed by polling through the same owner.
- Bound an end owner to the `IdentityService` instance that issued its context. Dropping that service invalidates its existing contexts.
- Kept the shared-thread proxy explicitly blocking and experimental; it does not implement C2's nonblocking `LiveSessionAuthority` or replace a C3 Supervisor launch-record check.

## Verification

- `nagi-identity-session --all-targets --locked --offline`: 6 unit and 51 contract tests passed.
- `nagi-identity-session --doc --locked --offline`: 2 compile-fail type-boundary doctests passed.
- `nagi-c1-session-end-adapter --all-targets --locked --offline`: 2 proxy unit and 15 routing tests passed.
- The adapted tests exercise current service-issued Sessions; results from source bundles were not reused.

## Still incomplete

- The current `IdentitySnapshotStore` has no durable production adapter in this checkout. A Linux-backed persistent store and its crash/corruption tests remain required.
- Authentication remains an injected host decision; no authentication method or Session expiry policy was selected here.
- The adapter is not yet wired into the normal UI, the actual C3 launch/FD path, or the actual C2 peer API. The root Cargo manifest and Linux image integration remain A-owned.
- Full root-workspace Host all-target acceptance and Linux Target acceptance have not been run. QEMU service readiness, real credential-to-Session flow, persistence, and existing lease revalidation remain unverified.
- The Cloud environment configuration tool was unavailable. The existing Cloud environment configuration was retained; no Mac absolute-path provisioner was applied.
