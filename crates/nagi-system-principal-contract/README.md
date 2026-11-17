# Nagi system principal contract

`nagi-system-principal-contract` is the C3-owned leaf contract consumed by
C1. It does not depend on C1 and does not represent an interactive `Session`,
consent grant, or app capability.

## Version 2

`SYSTEM_SERVICE_ENDPOINT_OWNER_PROOF_VERSION == 2` identifies
`SystemServiceEndpointOwnerProof`. The non-Clone proof binds:

- the opaque `SpawnedProcessIdentity` and a duplicated live pidfd;
- a nonzero C3 Supervisor launch generation, exact process, and
  `SystemService` role;
- a unique endpoint generation;
- the peer PID/UID/GID returned by Linux `SO_PEERCRED` for the owned
  `AF_UNIX` `SOCK_STREAM` endpoint.

`authenticate` compares the kernel peer PID to the exact spawn token and
compares UID/GID to the credentials selected by C3. It accepts no caller-made
peer-credential structure. It rejects an `InteractiveApplication` record.
`is_live` is a diagnostic snapshot only. Authority effects must run inside
`with_live_authority_effect`, which holds the shared C3 launch gate while
checking the current role, process, generation, pidfd, and endpoint, then keeps
that gate held through the effect callback.

C3 must call the unsafe
`SystemServiceLaunchRecord::after_supervisor_record_validation` constructor
only after its trusted launch path verifies the current Supervisor record,
role, generation, and exact process token. C3 must store the returned record in
the corresponding Supervisor slot and route every replacement/revocation
through its unsafe `replace_after_supervisor_record_validation` or
`revoke_after_supervisor_record_validation` callback while holding the
canonical Supervisor lock. Those callbacks mutate the actual registry while
the same shared launch gate is held. The unsafe constructors preserve the C3
decision; they do not establish package trust or decide which services are
system services.

Dropping a proof closes only its own endpoint and pidfd. Explicit proof revoke
also affects only that proof; only C3's Supervisor owner revokes the shared
launch generation. A C1 caller must hold its Session gate first, then this C3
gate, then run the effect. The callback must not re-enter C1 lifecycle or C3
Supervisor record operations. C1 should use this concrete proof type for its
private system-service seal and must not translate it into an interactive
Session.

## Current integration boundary

The type, launch gate, and Linux x86_64 peer verifier are implemented. The C3
Linux init path still needs a trusted system-service launch callsite that
owns the exact pidfd, Supervisor role/generation, and transferred endpoint.
The existing Supervisor source in this checkout records interactive app
launches and provides no system-service catalog or endpoint handoff. The C1
adapter must use the new callback gate, and its fixed 11063f snapshot has
independent review findings awaiting C1 owner repair. Until both sides are
wired and accepted, this contract is not evidence of production READY or
complete WS-04 acceptance.
