# ADR-0011: M27 Boot Readiness Handoff

**Status:** Accepted for the M27 readiness slice

**Date:** 2026-09-30

## Context

M27 persists an A/B trial before selecting a candidate slot, but no guest path
can confirm a candidate after it has become usable. `BootInfo` currently
contains no attempt identity, and UEFI variables are writable only while the
loader is running or through UEFI Runtime Services. The guest VFS cannot write
the loader's FAT/UEFI state. A serial marker parsed by the host would not be a
trusted readiness signal.

ADR-0007 keeps the kernel independent of UEFI Runtime Services. M27 needs a
narrow exception to that boundary; it does not need general firmware access.

## Decision

- The versioned `BootInfo` carries an immutable trial context only when the
  loader selected a pending slot: slot, attempt number, journal generation,
  and the address of the UEFI Runtime Services `SetVariable` entry point.
  The loader supplies that pointer from the firmware runtime table; the kernel
  validates it against the UEFI runtime-code memory descriptors before use.
- The kernel exposes one no-argument `SYS_BOOT_READY` request. It derives the
  slot and generation from trusted boot context, accepts the request once, and
  writes a small versioned, checksummed readiness record under Nagi's existing
  UEFI variable vendor GUID. It does not expose firmware pointers or accept a
  caller-selected slot, variable name, or payload.
- The system init requests readiness only after M6/M7 and VFS checks have
  passed and M10 has successfully presented its first desktop surface. The
  record binds readiness to the exact slot, attempt, and journal generation.
- On the next loader entry, the loader consumes a record only when it matches
  the still-pending journal state, then calls the existing
  `BootControlJournal::mark_boot_success`. Missing, malformed, stale, or
  mismatched records never promote a slot. Failure to persist the readiness
  record leaves the trial pending for the existing retry/rollback policy.
- In the current bootstrap, init is the sole user process. Before process
  isolation and a production supervisor exist, `SYS_BOOT_READY` is limited to
  that bootstrap path. Later multi-process systems must bind the request to a
  kernel-authenticated system-supervisor capability.

This changes ADR-0007 only for the single runtime `SetVariable` operation
needed by M27. The kernel does not link the UEFI crate, call Boot Services, or
expose general UEFI Runtime Services to user space.

## Readiness scope

M10's first successful surface presentation is the strongest implemented
guest readiness point. Nagi 0.1 does not yet have an account login/authentication
flow, so this slice proves desktop readiness after core service and storage
checks; it does not claim the specification's full login-and-desktop gate.

## Consequences

- A valid candidate can be promoted based on guest-observed readiness rather
  than a host-parsed serial string.
- A candidate that fails before the first desktop present remains pending and
  consumes the bounded retry budget.
- The direct firmware variable bridge is specific to the current UEFI
  reference target. A future platform needs its own kernel-owned boot-control
  adapter without widening user-space authority.
- QEMU acceptance must cover a ready candidate that becomes confirmed after
  restart, an injected pre-readiness failure that rolls back, and the existing
  malformed-payload rejection path.

## Alternatives considered

- A guest-written file mailbox cannot currently work because the guest has no
  writable path to the loader's FAT/UEFI journal.
- A general kernel UEFI runtime-services interface would expose unnecessary
  firmware authority and would weaken the M1 boundary.
- Host-side log parsing would let development tooling, rather than the guest,
  decide that a slot is healthy.
