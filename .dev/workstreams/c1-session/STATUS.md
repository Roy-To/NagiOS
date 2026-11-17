# C1 Session workstream status

Status as of 2026-11-17. This workstream remains in progress.

## Implemented in this branch

- Preserved the `ce0ec0b` / `2239981` source-only closure in the initial branch commit. It contains no inherited Git history.
- Added `InteractiveSessionContext`, issued by the current C1 service and rechecked against its trusted caller, live lease, expiry, and committed snapshot. The type does not implement `Send`, `Sync`, or serialization.
- Added the session-end adapter with opaque recipient-bound delivery, a stop latch before lifecycle callbacks, restricted retry after known pre-effect errors, and one-shot cleanup request followed by polling through the same owner.
- Bound an end owner to the `IdentityService` instance that issued its context. Dropping that service invalidates its existing contexts.
- Added `LinuxIdentitySnapshotStore`, which uses the integrating service's private directory descriptor, a single-writer lock, atomic whole-envelope replacement, a separately persisted high-water revision, exact LKG source versions, and fail-closed handling of missing/corrupt committed state. A stale but valid envelope is recovery input only; it cannot lower the CAS generation or supply current authority.
- Added a Linux C1 runtime adapter that re-verifies the existing `libnagi::credential::AccountRecord` through the current `AccountStore` verifier, discards the legacy `libnagi` Session, and issues only a durable C1 Session for an explicitly configured, already persisted User/Profile mapping.
- Added distinct `AuthenticatedInteractiveCaller` and `AuthenticatedSystemServicePrincipal` API bounds. Login, guest Session creation, current identity resolution, logout, and profile storage require the interactive peer context; recovery requires the OS service-principal context. C1 does not invent either proof: C2/Broker must supply the context after real peer authentication, and C3 must bind service credentials to endpoint ownership. The marker implementation itself is a trusted adapter boundary.
- Removed the fallback that reconstructed an in-process lease from an active durable record. If a C1-issued lease is absent, current identity resolution fails closed.
- Added a nonblocking `LiveSnapshot` API with an in-process lease generation and Live/Revoked/Expired status for connection maintenance. It does not read the durable store and is not authority; protected operations must use the existing full `check_live`, which re-reads the current committed snapshot. Lease generations are process-local, unique until exhaustion, never persisted or recovered, and old contexts are revoked when their issuer drops.
- Clarified `LocalAccountBinding`: C1 persists User/Profile records and Session lifecycle; it does not duplicate the existing verifier record or account-to-User/Profile mapping. The authoritative mapping source and any mapping persistence remain with its trusted account/provisioning owner.
- Kept the shared-thread proxy explicitly blocking and experimental. It does not implement C2's `LiveSessionAuthority` or replace a C3 Supervisor launch-record check.

## Verification

- Focused tests with pinned `nightly-2025-08-01` and locked offline dependencies: `nagi-identity-session` passed 19 unit, 53 contract, and 4 compile-fail doctests. New cases cover plain-trusted-ID rejection at the type boundary, exact expiry, logout/recovery revocation, new lease generation, and busy/unavailable/unsupported nonblocking clock results.
- `nagi-c1-session-runtime` passed 4 clock/ID unit tests and 5 integration tests, including real AccountRecord verification, C1 durable Session issuance, logout revocation, restart invalidation and generation change, LKG-only recovery, corrupt-current-plus-LKG failure, and User/Profile persistence.
- `nagi-c1-session-end-adapter` passed 2 proxy unit tests and 15 routing tests with the authenticated interactive caller bound.
- Host `./nagi test` previously passed after the locked nested dependency `serde_spanned v0.6.7` was fetched. Direct Host all-targets `cargo test --workspace --exclude nagi-kernel --all-targets --locked` also passed previously. These root-workspace commands do not include the stand-alone C1 crates; they were not rerun for this API change.
- The adapted tests exercise current service-issued Sessions; results from source bundles were not reused.

## Still incomplete

- Actual C2/Broker peer-authentication and C3 OS service-principal implementations are not connected; the marker traits define the type-level seam but do not authenticate by themselves. `LocalAccountBinding`'s account-to-User/Profile mapping source and first-boot provisioning flow still need an owner-approved C3/E integration.
- No authentication method or Session expiry duration was selected. The adapter uses the existing `AccountRecord` verifier and requires the caller to supply an expiry.
- The sidecar high-water detects stale or missing `issuer.json` while `issuer.highwater` remains. Restoring both files together from an older whole-store backup is not detectable without an external monotonic anchor; none is supplied by this adapter.
- The adapter is not wired into the normal UI, the actual C3 launch/FD path, or the actual C2 peer API. The C1 status API is only a point-in-time observation; the shared-thread proxy remains blocking and does not implement C2's `LiveSessionAuthority`. The root Cargo manifest and Linux image integration remain A-owned.
- Linux Target acceptance has not been run. QEMU service readiness, OS service-principal credentials/endpoint ownership, C2/C3 wiring, and lease revalidation through the real launched process remain unverified. Host checks do not establish Target acceptance.
- The Cloud environment configuration tool was unavailable. The existing Cloud environment configuration was retained; no Mac absolute-path provisioner was applied.
