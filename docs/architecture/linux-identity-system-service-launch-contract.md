# Linux Identity System-Service Launch Contract

**Status:** A-owned receiving contract. No production Identity service catalog entry, credential mapping, C1 seal adapter, or C3 Linux process-owner callsite is selected here.

## Boundary

PID 1 needs a strict descriptor receiver and a launch sequence that joins the trusted catalog, immutable executable bytes from the installed signed package, C3's exact process/endpoint proof, and C1's private SystemService principal mapping. The descriptor is input data; parsing it does not authenticate its source or make it launchable. A launch entry may return only a C1 runtime handle. It emits no `READY` result.

The descriptor carries a canonical package manifest ID and entry, not an independent numeric `AppId`; the existing package API derives `AppId` from the ID. `PackageSecurityProvider::inspect` authenticates an installed `.xapp` and its app manifest; it does not establish a system-service role or map a service ID to a C1 principal. A must not use a fixture app or account `LaunchRecord` as a SystemService record.

## Descriptor schema, version 1

Unknown and duplicate fields are rejected. Every field is required except that the lifecycle value chooses whether a previous generation exists.

```json
{
  "schema_version": 1,
  "service_id": "<canonical ID from the trusted system catalog>",
  "role": "system_service",
  "package": {
    "id": "<canonical package manifest id>",
    "canonical_entry": "<canonical manifest entry>"
  },
  "executable": {
    "artifact_path": "<catalog-selected path>",
    "expected_sha256": "<64 lowercase hexadecimal characters from the catalog>",
    "target": "x86_64-unknown-linux-musl"
  },
  "credentials": { "uid": "<catalog-supplied u32>", "gid": "<catalog-supplied u32>" },
  "generation": "<nonzero catalog generation>",
  "lifecycle": { "mode": "initial" },
  "endpoint": {
    "transport": "unix_stream",
    "owner": "c3_pidfd_supervisor_generation",
    "handoff": "connected_owned_fd",
    "generation_policy": "unique_per_connection",
    "expected_peer_uid": "<must equal credentials.uid>",
    "expected_peer_gid": "<must equal credentials.gid>"
  }
}
```

For replacement, `lifecycle` is `{ "mode": "replace", "previous_generation": <nonzero lower generation> }`. This is shape notation: placeholder strings are explanatory and are not valid values; numeric fields are `u32`/`u64`. A supplies no service ID, package ID, entry, digest, UID, GID, generation, or endpoint values. UID/GID zero is not independently forbidden; the trusted catalog policy must select values.

`PackageSecurityProvider::inspect` authenticates package bytes but has no expected-ID or expected-entry argument. A compares the returned manifest ID, derived `AppId`, canonical entry, and embedded executable digest with the same catalog descriptor. It copies the exact embedded executable into an immutable owned byte slice and hashes that copy. It then uses `PackageServiceSessionOwner::capture` and `lease_for_dispatch`; the retained package-generation lease and byte snapshot stay held for the A runtime lifetime. C3 receives a path-free launch metadata value plus this package binding and must load only `package.executable_bytes()`. The external package consumer regressions are in `tests/service-package-trust-consumer`; their Developer Preview signing key is test-only. These checks still do not establish a production catalog signer or the SystemService role.

`SystemServiceLaunchMetadata` carries the validated catalog generation and lifecycle transition to the C3 owner, but never the artifact path. `Initial` is admissible only when the exact service slot is empty. `Replace { previous_generation }` is admissible only when that exact prior catalog generation currently owns the slot. C3 must check and reserve this under its serialized service-slot gate, and reject stale or mismatched replacements before creating a child or Supervisor record. A's catalog generation and C3's per-connection launch generation are separate values; the proof and lifecycle binding must preserve both without treating one as the other.

The endpoint contract is a C3-owned Unix stream endpoint handed to the caller as a connected owned descriptor, with a unique generation per connection. C3 obtains peer credentials from the live kernel endpoint (`SO_PEERCRED` in the current Linux contract); descriptor fields and caller-supplied UID/GID are only expected values. The returned proof must bind that endpoint to the exact pidfd-backed process identity and C3 launch generation, report the kernel-observed UID/GID, and remain live while authority is used. A rejects a missing, stale, mismatched, or non-live proof and revokes the exact C3 slot. C1 admission then uses the authenticated catalog mapping; neither PID nor Linux credentials are a source for Nagi `PrincipalId` or `CallerContextId`.

`AuthenticatedSystemServiceCatalogBinding` is opaque outside this crate. Its constructor is currently compiled only for host test fixtures. A production constructor may be added only with an A-owned adapter that compares the complete validated descriptor against one canonical, authenticated catalog record. The crate has no such production adapter or record today, so downstream callers cannot mint a binding and the launch entry remains unavailable for production.

## Artifact preflight

The adopted helper request is exactly:

```json
{"schema_version":1,"artifact_path":"...","expected_sha256":"...","target":"x86_64-unknown-linux-musl"}
```

`tools/service-artifact-preflight` validates regular-file and no-symlink path handling, the static x86-64 ELF conditions, and the exact catalog-supplied SHA-256. Its report is artifact inspection only. It proves no signature, package identity, SystemService role, authentication, policy authorization, installation, or runtime readiness.

The helper's retained FD preserves inode identity but does not freeze bytes against another writer. The A adapter consumes the helper's bounded report, checks its digest against the descriptor, then checks that digest against the immutable executable snapshot copied from the installed signed package. C3 receives only path-free metadata and the package binding; it does not receive the artifact path or a mutable file descriptor. A therefore does not depend on a Rust FD-transfer adapter or the inspected inode remaining unchanged after preflight. The Python helper itself is unchanged and remains a point-in-time host preflight tool.

## C1 and C3 interface pins

C3 owns process creation, the exact Supervisor record/generation, connected endpoint transfer, `SO_PEERCRED`, and registry revocation. Its concrete proof is `nagi_system_principal_contract::SystemServiceEndpointOwnerProof` v2. The A-local contract crates are byte-identical to the source snapshot at C3 commit `5b257f139fade408bc0d4d5e9df19c680be08701`; this is an API dependency only, not the maintained C3 process-owner implementation. The relevant C3 signatures are:

```rust
unsafe fn SystemServiceLaunchRecord::after_supervisor_record_validation(
    process: &SpawnedProcessPidfd,
    generation: NonZeroU64,
    role: SupervisorLaunchRole,
) -> SystemServiceLaunchRecord;

fn SystemServiceEndpointOwnerProof::authenticate(
    process: &SpawnedProcessPidfd,
    launch: &SystemServiceLaunchRecord,
    endpoint: OwnedFd,
    expected_uid: u32,
    expected_gid: u32,
) -> io::Result<SystemServiceEndpointOwnerProof>;

fn SystemServiceEndpointOwnerProof::with_live_authority_effect<R>(
    &self,
    effect: impl FnOnce() -> R,
) -> io::Result<R>;
```

Only C3 may call the unsafe record constructor. A checks proof version, launch generation, peer UID/GID, endpoint presence, and liveness; it asks C3 to revoke the exact record on a mismatch or C1 rejection.

C1 alone creates the private SystemService seal and maps the catalog service ID to canonical `PrincipalId` and `CallerContextId`. Linux UID/GID and proof fields do not derive those IDs. Candidate commit `001e3e0f24326419d5c2ba98382e57621f592a28` is not adopted: its reported guest result of 118 does not establish the current Session-effect gate. A later fixed C1 journal commit `72899eafe2aa7ab9baf1ed3ef5d5fa2f4f27095f` is also not adopted as an A producer. Its independent static review identified an admission/read-gate race, a terminal-reconciliation condition that does not by itself invalidate older handles on a still-live C1 service, and separate C1 binding/C3 gate inputs that are not tied to one sealed token. The C1 owner reported a focused guest result of 81 passed, 49 failed, and 7 ignored for that SHA; A has not independently rerun it, and the reported run failed. The candidate's `SessionLeaseRevalidationHandle::with_live_session_effect` callback is synchronous and bounded; the IdentityService owner must remain outside the callback and must not be dropped from it. A's C1 bootstrap recovery is scoped to system-principal setup independent of any user Session and must not perform user-session-protected effects. Those effects need a separate adapter that takes the C1 Session gate before the C3/C2 owner gate, then performs the effect without capturing the issuing IdentityService owner. If bootstrap recovery cannot meet that scope, the current launch API is not sufficient and must remain unused.

The A launch adapter signatures are pinned in `crates/nagi-system-service-launch`. C3's latest source-focused guest run is reported as 37/37, but its exact fixed commit is still pending. Its maintained PID 1 callsites remain disabled; that focused guest result is not production launch acceptance. The maintained init full build is separately blocked by the missing `third_party/tokio-servo` manifest, and parallel orphan-reaper wait ownership remains with C3. The C1 system-service seal adapter and maintained C3 Linux process-owner implementation are still required before any production launch callsite can be enabled.

## Runtime shutdown and recovery quarantine — 2026-11-18

`SystemServiceRuntime::shutdown(self)` is the reportable shutdown path: it drops the C1 runtime behind a panic boundary, finishes the exact C3 lifecycle, and returns the same `SystemServiceRecoveryAdmission` if either outcome is uncertain. If callers use `Drop`, the runtime cannot return that token, so it must keep the in-process service fence and package-generation quarantine set, persist the C1 recovery quarantine, and persist the exact C3 service-slot quarantine when C3 termination is unresolved or panics. `SystemServiceLaunchOwner::persist_recovery_quarantine` must bind the durable slot record to the admission's service, catalog generation, and C3 launch generation. These are adapter requirements; no production C1/C3 adapter currently implements them.

The C1 adapter must enter recovery through C1's operation-bound intent path and preserve the exact admission through terminal reconciliation. That one sealed admission is the source for both C1's journal binding and C3's exact-generation slot gate; neither side may independently reload or reconstruct the mapping/generation. A generic successful callback is not a terminal receipt. A terminal result must also invalidate older handles on the same live C1 service, or carry a sealed proof that their authority remains valid. `PackageServiceSessionOwner` therefore exposes no caller-supplied quarantine-clear callback. `reconcile_identity_service` keeps the process fence set and returns `PackageQuarantineReleaseUnavailable(admission)` after C1/C3 reconciliation whenever the package owner remains quarantined. Clearing that package-generation fence remains unavailable until an exact operation-bound terminal receipt is consumable by the same package owner. Host tests use mock C1/C3 owners and establish only these receiver-side fail-closed rules.

## Verification and current blockers

Focused host negatives cover missing, malformed, duplicate, unknown, or contradictory descriptor data; preflight report mismatch; catalog rejection; and C3 launch failure. The launch orchestration stops before later owners when an earlier gate fails. These host tests prove the receiving contract only. They do not supply a production catalog, C1 seal, C3 process owner, service values, or Linux-image acceptance.

No production catalog entry or service artifact is available in A. The artifact snapshot handoff is an A host contract; its C3 Linux consumer is not connected. The fixed C1-to-Notes glue is not enabled: E still needs `OutcomeUnknown` for a panic after a partial save, a retry/reconcile guard, and a canonical dependency-unified joint type check plus review. Until the trusted catalog, C1 private adapter, and C3 Linux callsite are fixed and reviewed, the normal Linux `run()` remains unconnected and fail-closed.

## Production input audit — 2026-11-18

- The only product app manifest present under `user/nagi-isolated-app/manifests` is `org.nagi.files.manifest`; it is not an Identity system-service manifest. No canonical Identity service ID, package ID/entry, executable digest, UID/GID assignment, or production catalog record is selected in this branch. DEC-083 keeps the Linux credential mapping and implementation provisional.
- `nagi-package::TRUSTED_SIGNING_PUBLIC_KEY` is explicitly the RFC 8032 test-vector Developer Preview signer; product trust-store provisioning is still a later milestone. It cannot be used as production signer evidence. The external package-trust consumer tests use fixture signers and make no production trust claim.
- The A receiver can call the existing public package APIs with a caller-supplied trusted key, compare the signed manifest ID/entry/executable digest against one authenticated catalog descriptor, retain the installed package-generation lease, and pass C3 an immutable executable snapshot through a path-free launch metadata interface. Launch metadata also preserves `Initial` versus exact-prior-generation `Replace`; the C3 owner must enforce it atomically before child creation. The actual production key, catalog, service artifact, C1 seal/recovery adapter, and maintained C3 Linux process owner remain unconnected.
- A's launch result has no `READY` API. The current launch host suite passes 20 tests and the Linux musl target check passes, but these establish only the receiving contract and API compatibility. No production service readiness or Linux guest acceptance is claimed.
