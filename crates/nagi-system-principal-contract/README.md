# Nagi system principal contract

`nagi-system-principal-contract` is the C3-owned leaf contract consumed by
C1. It does not depend on C1 and does not represent an interactive `Session`,
consent grant, or app capability.

## Version 3

`SYSTEM_SERVICE_ENDPOINT_OWNER_PROOF_VERSION == 3` identifies
`SystemServiceEndpointOwnerProof`. The non-Clone proof binds:

- the opaque `SpawnedProcessIdentity` and a duplicated live pidfd;
- the canonical C3 service-slot ID and catalog generation, separately from the
  nonzero C3 Supervisor process-launch generation;
- the exact process and `SystemService` role;
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
`SystemServiceLaunchRecord::after_supervisor_system_service_slot_validation`
constructor only after its trusted launch path verifies the canonical service
slot, catalog generation, system-service role, process token, and C3 launch
generation. The generic `after_supervisor_record_validation` constructor does
not bind a service slot and cannot issue an endpoint-owner proof. C3 must store
the returned record in the corresponding Supervisor slot and route every
service-slot replacement/revocation through its unsafe
`replace_after_supervisor_system_service_slot_validation` or
`revoke_after_supervisor_record_validation` callback while holding the
canonical Supervisor lock. Replacement must keep the service ID constant and
match the exact prior catalog generation. Those callbacks mutate the actual
registry while the same shared launch gate is held. The unsafe constructors
preserve the C3 decision; they do not establish package trust or decide which
services are system services.

Dropping a proof closes only its own endpoint and pidfd. Explicit proof revoke
also affects only that proof; only C3's Supervisor owner revokes the shared
launch generation. A C1 caller must hold its Session gate first, then this C3
gate, then run the effect. The callback must not re-enter C1 lifecycle or C3
Supervisor record operations. C1 should use this concrete proof type for its
private system-service seal and must not translate it into an interactive
Session.

## Current integration boundary

The type, slot-aware proof, launch gate, and Linux x86_64 peer verifier are
implemented. The C3 Linux init path still needs a trusted system-service
launch callsite that owns the exact service slot, catalog generation, pidfd,
role, C3 launch generation, and transferred endpoint. The existing Supervisor
source in this checkout records interactive app launches and provides no
production system-service catalog or endpoint handoff. The C1 adapter must use
the callback gate and the exact same A-issued admission object. Until both
sides are wired and accepted, this contract is not evidence of production
READY or complete WS-04 acceptance.
