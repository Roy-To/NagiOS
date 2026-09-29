# IDENT-01 owner assignment and contract-boundary record

Status: **assigned to `codex/0.2-ident-01` and registered in this branch by a separate Integration Owner action; host-reference status remains PARTIAL and production/runtime activation remains gated.**

## Owner assignment and Git boundary

- Accountable workstream owner: `Codex IDENT-01 workstream (assigned by Codex integration owner)`.
- Owner branch: `codex/0.2-ident-01`, created at base
  `198b9f60b41eba14e74c2c04f51ccdcb4030adf9`, which was both local and
  `origin/codex/integration-next-phase` HEAD when checked.
- Dedicated managed worktree:
  `/Users/tozawa/.codex/worktrees/ident-01/NagiOS`.
- The registration row is an Integration Owner change, isolated in its own
  commit after the host-reference implementation commit. The row is present on
  this candidate branch; it is not merged into `codex/integration-next-phase`
  or `main` by this task.
- M17 is `BLOCKED`; M30 is `NOT STARTED`. Host-only reference contracts and
  deterministic fakes/tests may proceed. Production session service wiring,
  persistent runtime identity/storage, user/nagi-init, capability enforcement,
  and target integration remain closed until M30 `PASS` and an explicit Nagi
  0.2 Integration Owner checkpoint naming IDENT-01.

## Host-reference contracts transferred

- `UserId` adapts the existing `nagi_model::UserId` while validating non-zero
  values. `ProfileId`, identity `SessionId`, and `CallerContextId` are distinct
  validated values. Capability `PrincipalId` is re-exported from
  `nagi-capability`; the capability crate's separate `SessionId` is not reused
  for user-session lifecycle.
- Local user records and persistent/guest profiles have bounded, versioned,
  non-secret metadata. Guest profiles are ephemeral. Provider subject input is
  transient and redacted from debug output; no cloud/network authentication is
  implemented.
- Local-login orchestration and guest creation are offline and injected behind
  a trusted authorizer. Current identity resolves explicitly per
  `(PrincipalId, CallerContextId)`; AppId, AppSessionId, ExecutionInstanceId,
  SurfaceId, PID, path, and display name are not substituted for identity.
- Session create/end, expiry, idempotency, recovery-ended sessions, and
  current-identity leases are modeled explicitly. Leases are rechecked for
  protected storage operations.
- The storage adapter exposes a non-cloneable, non-serializable opaque handle
  constrained to one `ProfileId`; it revalidates caller, live session, and
  exact-profile scope on each operation. Only in-memory test backends exist;
  there is no host path or production VFS root.
- The capability adapter maps to the canonical `PrincipalId`, creates no
  grants, and fails closed. It does not implement policy or enforcement.
- Snapshot-store and migration interfaces define version checks, atomic
  replacement, last-known-good recovery, explicit converters, and safe session
  recovery. Only the in-memory fake exists; no durable runtime store is added.

## Dependency owner and version findings

| Registered dependency | Current owner/status | Contract boundary used here |
| --- | --- | --- |
| `development-foundation` | `Codex foundation integrator`; registry row and DF-01 v1 tooling are present | State and registration follow DF-01; the final branch verifier will validate this state after registration. |
| `capability-permissions` | `Codex capability/permission workstream`; state is `PARTIAL`; crate is `nagi-capability` 0.2.0 | Canonical validated `PrincipalId` and versioned policy/declaration models are reused. No published, versioned binding from a user session/trusted caller to a runtime principal or enforcement API is present; adapter tests use injected fakes. |
| `app-sdk-contract` | `Codex App SDK contract workstream`; state is `PASS` for host contract acceptance | AppId, AppSessionId, and ExecutionInstanceId remain semantically distinct. IDENT-01 does not change SDK files. |
| `platform-apis` | Registered row exists but owner is `unassigned`; no state file is registered/present | Existing `nagi-model` 0.1.0 exposes `UserId(u64)`, which IDENT wraps with validation. This is source-level reuse, not an accepted/versioned 0.2 identity contract. |
| Storage/VFS | **Unresolved: no Storage/VFS owner row is registered in the 18-row base registry, and no owner can be inferred.** | `user/libnagi/src/storage.rs` provides block-backed file handles, not a caller-authorized profile-root contract. IDENT keeps only its narrow opaque adapter proposal and does not edit storage/VFS implementation. Integration Owner must designate an existing owner or create a separately scoped owner/contract before persistent profile roots are integrated. |

The hard registry dependencies name only registered IDs. Storage/VFS remains a
proposal-level unresolved dependency because no corresponding registered owner
or versioned profile-root contract exists.

## Host-reference evidence and remaining acceptance

The package contains deterministic host fakes and negative contract tests for
cross-user/profile denial, caller/principal binding, stale/expired sessions,
malformed/corrupt/unsupported snapshots, interrupted commit, guest cleanup,
principal denial, migration failure, and store-version conflict. These tests
are evidence for the host reference only. They do not satisfy production
persistence, product login, real Storage/VFS authorization, runtime capability
enforcement, target behavior, or full IDENT-01 acceptance.

The state file records exact focused test, format, Clippy, root verifier, CI,
remote-head, and worktree results. Do not change M17/M30 status or represent
host-only tests as production acceptance.

## Next Integration Owner operations

1. Keep production/runtime work gated until M30 `PASS` and an explicit 0.2
   checkpoint names IDENT-01.
2. Ask the Capability owner to publish the versioned trusted-caller/session to
   `PrincipalId` binding and enforcement contract; integrate it only through
   the adapter after owner acceptance.
3. Assign or establish a Storage/VFS owner and a versioned opaque,
   capability-scoped profile-root contract. Do not infer ownership from
   `user/libnagi` or implement that contract in IDENT-01.
4. After those contracts and the activation gate pass, review persistence and
   runtime-service ownership, then run production/target acceptance outside
   this host-only checkpoint.
