# C1 proof-producer integration review

Review snapshot: C1 `de678984abe7c9718be7d3299c0741acb7099168`; C2 `814799c59e723d084dbc186a203a99a113c2157b`; C3 `93424aa679b519c7cfafd11313943b96298200ba`.

## Maintained API findings

- C2's maintained `crates/nagi-nipc-v1-stream` provides NIPC frame and Unix-stream helpers (`read_frame`, `write_frame`, `unix::{bind, connect, receive, send, call}`). Its module documentation explicitly leaves peer authentication, dispatch, and service readiness to another layer. It exports no authenticated interactive peer or received-session type.
- C2's `LiveSessionAuthority` and `SessionLease` are in `user/nagi-init/candidates/linux-broker-stage25/stage25/pid1_live_peer_supplier.rs`, which that candidate's README says is not wired into maintained Linux init. `ReceivedInteractiveSession<'peer>` is referenced in comments, not declared as a type. The candidate trait is:

  ```rust
  fn capture_live_unlocked(
      &self,
      app_id: u64,
      app_session_id: u64,
  ) -> Result<SessionLease, PeerDenial>;
  fn is_live_unlocked(&self, lease: SessionLease) -> Result<bool, PeerDenial>;
  ```

  Its `SessionLease::from_live_session_authority` constructor is `pub(crate)`. A separate C1 crate therefore cannot implement this candidate trait and construct the required return value. Numeric app/session IDs also cannot stand in for C1's caller-bound Session proof.
- C3 exports the opaque `nagi_init_spawn_contract::{SpawnedProcessIdentity, SpawnedProcessPidfd}` for exact clone3 process-instance and pidfd ownership. These do not establish ownership of an accepted service endpoint or a C1 system principal. The `LinuxExecutionIdentity` in `user/nagi-init/src/linux_init.rs` is private UID/GID execution configuration, not a Nagi identity or endpoint-authentication proof. No maintained C1-compatible authenticated system endpoint principal is exported.

## Exact producer contract needed

1. C2 must publish a non-forgeable received interactive peer/session value from the maintained accepted-connection path. It must bind the actual connection instance to the trusted authentication result and expose trusted caller identity plus a revocable current-session generation. The value must not be constructible from NIPC payload IDs, PID/UID/GID, or a session notice. It needs a bounded revalidation operation whose unavailable/revoked result denies use; a zero-wait observation must remain distinct from admission-time revalidation.
2. C2 must expose the live-session bridge in maintained code with a usable cross-crate contract. If C1 is to implement `LiveSessionAuthority`, the returned lease needs a constructor/issuer accessible only through an authenticated producer contract, or the bridge must instead accept a C1-owned opaque revalidation handle. The current candidate's crate-private `SessionLease::from_live_session_authority` does not permit a C1 consumer implementation.
3. C3 must publish an opaque service-endpoint ownership proof from its real launch/endpoint owner. It must bind the exact `SpawnedProcessIdentity`/pidfd and launch record to the owned endpoint and the kernel-verified accepted peer connection, and expose revocation/liveness tied to that owner. UID/GID values alone and an interactive C1 Session must not create this proof.

Once these maintained types exist, C1 can implement its private seals only for those concrete producer values, map trusted caller context through the existing account/provisioning owner, obtain `IdentityService::interactive_session`, and pass a checked `SessionLeaseRevalidationHandle` to connection owners. Every protected use must revalidate the durable/current Session; logout, expiry, recovery, and restart must deny the old connection. C1 must keep the system-principal proof separate from interactive Session issuance. No producer type currently exists against which that adapter can be compiled without inventing a substitute.

The current `LinuxSessionRuntime::start` accepts trusted `Vec<LocalAccountBinding>` input and checks that its User/Profile exists and is owned by that User. It keeps the verifier record in memory and does not persist the account-to-User/Profile mapping; the runtime already assigns that mapping's authoritative source and persistence to the account/provisioning owner. No C2/C3 API supplies that owner, so C1 must not duplicate its mapping store.

## C1 coverage and limits

The runtime and session-end adapter scenarios are compiled into the C1 crate's `cfg(test)` build so role-specific test fixtures can implement the crate-private seals without making any test constructor or seal available to product consumers. This restores 5 runtime durable-auth scenarios and 15 adapter routing/lifecycle scenarios. The 9 doctests include 8 external compile-fail cases for caller spoofing/role confusion and one positive public revalidation-handle contract. The normal runtime and adapter library crates also compile as external consumers.

Host verification on this Linux executor: `cargo test --offline --manifest-path crates/nagi-identity-session/Cargo.toml` passed (107 unit tests, 9 doctests); `cargo check --offline` passed separately for `crates/nagi-c1-session-runtime/Cargo.toml` and `crates/nagi-c1-session-end-adapter/Cargo.toml`; `cargo fmt --manifest-path crates/nagi-identity-session/Cargo.toml -- --check` passed. These unit fixtures do not prove C2 peer authentication or C3 OS endpoint ownership.

Target acceptance remains unverified: there is no maintained C2 authenticated peer producer or C3 system endpoint principal to connect, and no QEMU/guest run was performed. The C1 integration is therefore blocked at those exact producer APIs; Host results are not Target acceptance.
