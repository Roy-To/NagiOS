# C1 Session workstream status

Status as of 2026-11-17. This workstream remains in progress.

## Implemented in this branch

- Preserved the `ce0ec0b` / `2239981` source-only closure in the initial branch commit. It contains no inherited Git history.
- Added `InteractiveSessionContext`, issued by the current C1 service and rechecked against its trusted caller, live lease, expiry, and committed snapshot. The type does not implement `Send`, `Sync`, or serialization.
- Added the session-end adapter with opaque recipient-bound delivery, a stop latch before lifecycle callbacks, restricted retry after known pre-effect errors, and one-shot cleanup request followed by polling through the same owner.
- Bound an end owner to the `IdentityService` instance that issued its context. Dropping that service invalidates its existing contexts.
- Kept the shared-thread proxy explicitly blocking and experimental; it does not implement C2's nonblocking `LiveSessionAuthority` or replace a C3 Supervisor launch-record check.
- Added `LinuxIdentitySnapshotStore`, which uses the integrating service's private directory descriptor, a single-writer lock, atomic whole-envelope replacement, a separately persisted high-water revision, exact LKG source versions, and fail-closed handling of missing/corrupt committed state. A stale but valid envelope is recovery input only; it cannot lower the CAS generation or supply current authority.
- Added a Linux C1 runtime adapter that re-verifies the existing `libnagi::credential::AccountRecord` through the current `AccountStore` verifier, discards the legacy `libnagi` Session, and issues only a durable C1 Session for an explicitly configured, already persisted User/Profile mapping.
- Kept system-service recovery caller and interactive caller as separate injected trusted contexts. The C1 adapter does not invent an OS service principal or accept User/Profile IDs from login requests.

## Verification

- Latest focused run with the pinned `nightly-2025-08-01` toolchain and locked offline dependencies: `nagi-identity-session --all-targets` passed 19 unit and 51 contract tests, including older-envelope rollback and interrupted high-water reservation; `--doc` passed 2 compile-fail type-boundary doctests.
- `nagi-c1-session-end-adapter --all-targets` passed 2 proxy unit and 15 routing tests.
- `nagi-c1-session-runtime --all-targets` passed 3 clock/ID unit tests and 5 integration tests, including real AccountRecord verification, C1 durable Session issuance, logout revocation, restart reauthentication, LKG-only recovery, corrupt-current-plus-LKG failure, and User/Profile persistence.
- Host `./nagi test` initially exposed two CLI integration failures because the locked nested dependency `serde_spanned v0.6.7` was absent offline. After fetching the locked dependency, `./nagi test` passed. Direct Host all-targets `cargo test --workspace --exclude nagi-kernel --all-targets --locked` also passed. These root-workspace commands do not include the stand-alone C1 crates; their latest results are listed above.
- The adapted tests exercise current service-issued Sessions; results from source bundles were not reused.

## Still incomplete

- `LocalAccountBinding` is trusted integration input and is not persisted by this adapter. The account-to-User/Profile mapping source and first-boot provisioning flow still need an owner-approved C3/E integration.
- No authentication method or Session expiry duration was selected. The adapter uses the existing `AccountRecord` verifier and requires the caller to supply an expiry.
- The sidecar high-water detects stale or missing `issuer.json` while `issuer.highwater` remains. Restoring both files together from an older whole-store backup is not detectable without an external monotonic anchor; none is supplied by this adapter.
- The adapter is not wired into the normal UI, the actual C3 launch/FD path, or the actual C2 peer API. The root Cargo manifest and Linux image integration remain A-owned.
- Linux Target acceptance has not been run. QEMU service readiness, OS service-principal credentials/endpoint ownership, C2/C3 wiring, and lease revalidation through the real launched process remain unverified. Host acceptance does not establish Target acceptance.
- The Cloud environment configuration tool was unavailable. The existing Cloud environment configuration was retained; no Mac absolute-path provisioner was applied.
