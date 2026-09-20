# ADR-0010: M4 bounded capability, VMO, and Channel substrate

- Status: Accepted
- Date: 2026-09-17

## Decision

M4 provides no-heap, bounded kernel-library primitives for independent process
handle tables, 64-bit slot/generation handles, an object registry with its own
generation/refcount, rights attenuation, bounded anonymous/shared VMO backing,
AddressSpace mappings, Channel/Event/Timer objects, and waiter registration.
Channel messages carry fixed-width headers, bounded inline data, and explicitly
attenuated handle transfers held in queue-owned escrow until receive.

The M4 guest acceptance uses two independent kernel `Process` instances until
M5 introduces user address spaces and syscall entry. This is the real object
and capability path, not a host-side simulation or a fake response.

## Reason

M4 must establish the security and IPC contracts before user processes and
services depend on them. Fixed bounds make the Developer Preview testable
without introducing an allocator or high-level POSIX semantics into the
kernel.

## Consequences

Generation counters prevent stale-handle and stale-object reuse. Object
creation has an explicit creator reference that is released by the typed
object manager after publication, and receiver
rights can only be equal to or weaker than the sender's explicitly transferred
rights. Transfer tokens are move-only and remain queue escrow until a
transactional receive succeeds; a successful send notifies peer READABLE
waiters. Wait registration reserves from the bounded wake budget and rejects
admission when it cannot guarantee a future wake, so saturation cannot strand
a newly blocked waiter. VMO backing is bounded and zeroed; user page-table installation remains
later work. Channel queues and wait registrations are bounded, with a real
blocked/runnable waiter protocol. VMO and Event/Timer operations are reached
through Process handle gates, and VMO mappings retain/release object references.
M5 connects the waiter state transition to process thread blocking and syscall
entry.
