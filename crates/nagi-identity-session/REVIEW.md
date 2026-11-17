# C1 proof-producer integration review

Review snapshot: C1 `801ed55951c5c0d0758101bdc50beec3c28d395b`; C2 `814799c59e723d084dbc186a203a99a113c2157b`; C3 `93424aa679b519c7cfafd11313943b96298200ba`.

## Maintained API findings

- C2's maintained `crates/nagi-nipc-v1-stream` provides NIPC frame and Unix-stream helpers (`read_frame`, `write_frame`, `unix::{bind, connect, receive, send, call}`). Its module documentation explicitly leaves peer authentication, dispatch, and service readiness to another layer. It exports no authenticated interactive peer or received-session type.
- C2's `LiveSessionAuthority` and `SessionLease` are in `user/nagi-init/candidates/linux-broker-stage25/stage25/pid1_live_peer_supplier.rs`, which that candidate's README says is not wired into maintained Linux init. `ReceivedInteractiveSession<'peer>` is referenced in comments, not declared as a type. C1 currently exports `InteractiveSessionContext` and `SessionLeaseRevalidationHandle`, but no `ReceivedInteractiveSession` type. The candidate trait is:

  ```rust
  fn capture_live_unlocked(
      &self,
      app_id: u64,
      app_session_id: u64,
  ) -> Result<SessionLease, PeerDenial>;
  fn is_live_unlocked(&self, lease: SessionLease) -> Result<bool, PeerDenial>;
  ```

  Its `SessionLease::from_live_session_authority` constructor is `pub(crate)`. That is appropriate for C2-owned lease issuance: C1 should not implement C2's trait or construct C2 leases. C2's implementation must resolve the numeric selectors against the exact accepted peer's C1 revalidation handle; those IDs alone are not authority.
- C3 exports the opaque `nagi_init_spawn_contract::{SpawnedProcessIdentity, SpawnedProcessPidfd}` for exact clone3 process-instance and pidfd ownership. These do not establish ownership of an accepted service endpoint or a C1 system principal. The `LinuxExecutionIdentity` in `user/nagi-init/src/linux_init.rs` is private UID/GID execution configuration, not a Nagi identity or endpoint-authentication proof. No maintained C1-compatible authenticated system endpoint principal is exported.

## Cycle-free dependency direction and minimum API

Use producer leaf crates so C1 can depend on concrete proof types without depending on either lane's runtime root:

```text
nagi-identity-session ──> C2 peer-auth producer leaf
nagi-identity-session ──> C3 endpoint-owner producer leaf
C2 broker/runtime      ──> nagi-identity-session + C2 peer-auth producer leaf
C3 init/service runtime──> nagi-identity-session + C3 endpoint-owner producer leaf
```

The C2/C3 producer leaves must not depend on `nagi-identity-session`. The C2 leaf could be `nagi-nipc-v1-stream` only if it is extended to own the real peer-authentication producer while remaining C1-independent; otherwise use a separate C2 peer-auth crate. The C3 leaf could build on `nagi-init-spawn-contract` only if the exact endpoint owner and kernel peer validation live there too; otherwise split a C3 endpoint-owner crate below the C3 runtime root. An opaque data type whose only real issuer remains in a root that depends on C1 does not break the cycle.

If these edges should be optional, they can be C1 product features that enable only the real producer leaf dependencies and their seal impls. The test fixtures must remain exclusively behind `cfg(test)` and must not be activated by a product feature. Since the private seals live in `nagi-identity-session`, its crate must own the concrete impls; a sibling adapter crate cannot implement them.

Minimum C2 leaf API: an opaque `AuthenticatedInteractivePeer<'connection>` issued only from the maintained accepted-connection path after the real peer/endpoint checks. It must expose the trusted peer key resolved from the launch/owner record, a unique accepted-connection context, and a bounded live/revocation check. None may be supplied by NIPC payload IDs, PID/UID/GID alone, or a session notice. C1 depends on this leaf and implements its private interactive proof for that concrete type. It maps the trusted peer/context to C1 caller IDs; the C1 runtime separately verifies the actual local `AccountRecord` credential and issues the durable User/Profile Session. This peer proof is transport provenance, not successful user login.

The C2 broker/runtime root depends on both C1 and its peer-auth leaf. It pairs each accepted connection with the C1 `InteractiveSessionContext` or owned `SessionLeaseRevalidationHandle`, implements its own `LiveSessionAuthority`, and issues its own C2 `SessionLease`. `capture_live_unlocked(app_id, app_session_id)` may use those numeric values only as selectors into the exact connection-bound handle. Before protected effects, C2 revalidates the C1 durable/current Session; zero-wait observations remain maintenance-only. Logout, expiry, recovery, and restart must invalidate the old handle. This keeps C2 lease construction inside C2 and avoids C1 depending on C2's runtime root.

Minimum C3 leaf API: an opaque `OwnedServiceEndpointPrincipal<'endpoint>` issued by the actual process/endpoint owner. It must bind the exact `SpawnedProcessIdentity` and retained pidfd, launch record, owned endpoint generation, and kernel-verified accepted peer connection, with owner-tied liveness/revocation. C1 depends on this leaf and implements its distinct private system-principal proof for that concrete type. C3's runtime root depends on both C1 and the leaf. Numeric UID/GID, PID, or an interactive C1 Session cannot construct this proof.

At the C2/C3 source snapshots above, no C2 peer-auth or C3 endpoint-owner producer type was present. The fixed leaf source delivered later is described below; it still lacks the owner gates and caller/session binding required before C1 can seal either proof. C1 must not add a generic trusted-caller constructor, open its seals, or make a fixture stand in for either producer.

The current `LinuxSessionRuntime::start` accepts trusted `Vec<LocalAccountBinding>` input and checks that its User/Profile exists and is owned by that User. It keeps the verifier record in memory and does not persist the account-to-User/Profile mapping; the runtime already assigns that mapping's authoritative source and persistence to the account/provisioning owner. No C2/C3 API supplies that owner, so C1 must not duplicate its mapping store.

## C1 coverage and limits

The runtime manifest has `autotests = false`, so its standalone `cargo test` does not discover `tests/durable_auth.rs` (5 scenarios); the adapter manifest likewise does not discover `tests/routing.rs` (15 scenarios). `cargo metadata --no-deps` reports only the library target for each standalone manifest. Both scenario files are explicitly included as `cfg(test)` modules in `nagi-identity-session/src/lib.rs`; the canonical C1 test entry is `rustup run nightly-2025-08-01 cargo test --offline --manifest-path crates/nagi-identity-session/Cargo.toml`, which executes all 5 runtime and 15 adapter scenarios inside the identity test harness. This keeps role-specific fixtures able to implement crate-private seals without exposing any test constructor or seal to product consumers.

The C1 manifest has no declared product features (`cargo metadata --no-deps` reports `features: {}`). The seal re-export and scenario modules are `#[cfg(test)]`; the seal itself is `pub(crate)`. `cargo check --all-features` for the identity, runtime, and adapter manifests passed, and the 8 external compile-fail doctests reject caller spoofing/role confusion. Thus the test-only fixture proofs are not reachable through a product dependency or feature.

Follow-up after the fixed de678984 review, starting from C1 HEAD `91cef42c10d5a331f50fb2afaf60a221b8180b88`:

- `contract::durable_expiry_stays_revoked_after_restart` uses the Linux durable snapshot store with a controllable test clock. At the expiry boundary it persists `Ended(Expired)`, revokes the retained handle, and confirms restart does not restore that Session as current.
- `contract::cancelling_orphaned_session_preserves_another_current_session_for_the_caller` reproduced F1 before the fix: after orphan B was cancelled while Session A was current, resolving A returned `Ended`. `end_record` now updates the caller's current/last maps only when the ended Session is current or no Session is current; cancelling B leaves A's lease and current mapping intact.
- `rustup run nightly-2025-08-01 cargo test --offline --manifest-path crates/nagi-identity-session/Cargo.toml cancelling_orphaned_session_preserves_another_current_session_for_the_caller` failed before the fix with `ResolutionError::Ended`, then passed after the fix (1 passed).
- `rustup run nightly-2025-08-01 cargo test --offline --manifest-path crates/nagi-identity-session/Cargo.toml` passed: 109 unit tests and 9 doctests, including the runtime, session-end adapter, durable-expiry, and external compile-fail scenarios.
- `rustup run nightly-2025-08-01 cargo fmt --manifest-path crates/nagi-identity-session/Cargo.toml -- --check` passed.

The earlier standalone runtime/adapter and `cargo check` results remain prior evidence; they were not repeated for this follow-up. These Host fixtures do not prove C2 peer authentication or C3 OS endpoint ownership.

Target acceptance remains unverified. The later fixed leaf source now supplies transport and endpoint proof types, but its accepted-session binding, C2 effect gate, C3 exact-generation gate and production launch callsite remain incomplete; no QEMU/guest run was performed. Host results are not Target acceptance.

## Linux durable commit-failure follow-up after 7669ae

Coverage audit: `store::linux::tests::unknown_commit_outcomes_poison_the_instance_and_reopen_never_reuses_a_generation` tests the Linux file-store poison boundary without an issued identity. The recovery-failure authority tests in `tests/identity-session/contract.rs` use `SharedStore`, an in-memory implementation. Neither exercised a retained C1 handle against a real Linux store after its independent high-water advanced and envelope replacement failed.

Added `service::durable_store_failure_tests::poisoned_durable_commit_denies_old_authority_and_recovers_after_restart`. It uses `LinuxIdentitySnapshotStore` on a private temporary directory and the existing `cfg(test)` `Fault::AfterHighWater` seam. With a live Session handle, a profile commit fails after high-water persistence; the old handle is denied, same-issuer write retry and recovery fail closed, and a fresh issuer recovers from `LastKnownGood`, persists the old Session as ended, and does not restore its authority. The fault selector remains test-only; no production authentication entry or seal was added. The fixture validates C1 store/lease failure behavior and does not represent a C2/C3 producer.

Host results for this follow-up:

- `rustup run nightly-2025-08-01 cargo test --offline --manifest-path crates/nagi-identity-session/Cargo.toml poisoned_durable_commit_denies_old_authority_and_recovers_after_restart`: PASS, 1 passed.
- `rustup run nightly-2025-08-01 cargo test --offline --manifest-path crates/nagi-identity-session/Cargo.toml`: PASS, 110 unit tests and 9 doctests.
- `rustup run nightly-2025-08-01 cargo fmt --manifest-path crates/nagi-identity-session/Cargo.toml -- --check`: PASS.

## Fixed C2/C3 leaf-source and revoke/effect follow-up

The internal Library source bundle was materialized and read. `SOURCE.txt` pins C2 `ae57e21da4c6c6d361d19827ae1274e6f1becd1e` and C3 `f40cc5c18ec18e6b94c809be71dcfda158542b12`; the 109,385-byte ZIP SHA-256 was `f7639946018736daec72a822a7b719f8ee0fcf75c19666fff0a82af6eaf62ef4`. It contains the three leaf crates, C2 supplier and C3 process reference sources, and no Git metadata or runtime outputs. The source was kept in a separate untracked input directory.

Source inspection confirms:

- C2's `KernelAuthenticatedUnixPeer` authenticates `SO_PEERCRED` and `SO_COOKIE`; `PeerConnectionId` is a connection selector. Neither proves accepted launch ownership nor a C1 Session. The supplied `BoundPeerSessionRevalidation::check_current(SessionLease) -> Result<(), PeerDenial>` is a point-in-time check with no lease returned or callback held through effect commit. It is called before the C2 owner-gate/effect path, so logout/revoke can race after the check.
- C3's `SystemServiceEndpointOwnerProof` (`SYSTEM_SERVICE_ENDPOINT_OWNER_PROOF_VERSION == 1`) binds a process pidfd, launch generation, endpoint generation and kernel peer credentials. `is_live() -> io::Result<bool>` checks an atomic revocation flag plus pidfd/endpoint poll; `revoke` changes the atomic flag and closes descriptors. This is not serialized with the exact current Supervisor record or a later effect. The unsafe `SystemServiceLaunchRecord::after_supervisor_record_validation` is used only by the reference tests in this bundle; there is no production trusted runtime callsite.
- The C2 leaf `cargo test --offline` passed in an isolated build copy (1 unit test and 1 compile-fail doctest). The C3 `nagi-init-spawn-contract` `cargo test --offline` passed (2 unit tests); `nagi-system-principal-contract` `cargo check --offline` passed and its library source contains no embedded unit tests. The C2 supplier and C3 process reference files are not standalone crates, and their production integration/QEMU tests were not run.

C1 added `SessionLeaseRevalidationHandle::with_live_session_effect`. It revalidates the durable/current Session and holds C1's existing read lease for a bounded synchronous callback; `end_record` takes the matching write lease before persisting logout/revocation. This gives a producer adapter a C1-side critical section to hold across its own gate and effect commit. A Linux durable-store concurrency test confirms logout cannot finish while that callback is held and a stale handle does not invoke a later callback. The method adds no caller constructor, proof type, seal, or external dependency.

The exact owner-gate work remains on C2/C3. C2's minimum compatible boundary is an object-safe operation such as `with_current(lease, &mut dyn FnMut() -> Result<(), PeerDenial>)`: the C1 adapter runs the callback inside `SessionLeaseRevalidationHandle::with_live_session_effect`, and C2 must put its own owner gate, final lease check, effect, and effect commit inside that callback. The lock order is C1 Session gate before C2 owner gate; C2 must not hold its owner gate while waiting for C1 logout/revocation. A one-shot `check_current` cannot implement this. C2 must also bind the kernel peer to its accepted launch record and exact C1-issued Session handle. C3 needs an owner-provided callback/guard for the exact current Supervisor generation that validates generation and holds that owner gate through authority use; record replacement and revoke must take the same gate. Its trusted runtime must issue launch records only after validating the exact current record, role and process token. C1 will not seal either proof as production authority until these gates and the accepted-session binding are available. Required race tests are: revoke before admission, revoke while an effect holds both owner gates, and exact-record replacement while a use is waiting; each must prove the effect commit and revocation have one serialized order. The supplied fixed leaves remain unchanged and no Target acceptance is claimed.

Host C1 results after this handoff:

- `rustup run nightly-2025-08-01 cargo test --offline --manifest-path crates/nagi-identity-session/Cargo.toml live_session_effect_serializes_with_logout_and_stale_handle_denies_effect`: PASS, 1 test.
- `rustup run nightly-2025-08-01 cargo test --offline --manifest-path crates/nagi-identity-session/Cargo.toml`: PASS, 111 unit tests and 9 doctests.
- `rustup run nightly-2025-08-01 cargo fmt --manifest-path crates/nagi-identity-session/Cargo.toml -- --check`: PASS.
