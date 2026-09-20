# M4 Handles / VMO / IPC Design

## Goal

Implement the first reusable capability and IPC substrate without adding a
user-mode process before M5. M4 will model two independent kernel `Process`
objects with separate handle tables, connect them through a real bounded
Channel object, and verify handle transfer attenuation through the same code
used by the guest acceptance path.

## Architecture

### Handles and processes

`kernel/src/handles.rs` owns `Rights`, `Handle`, `ObjectKind`, `ObjectId`,
`ObjectRegistry`, `HandleTable`, and a bounded generic `Process`. A handle is
a 64-bit value containing a 32-bit slot and a 32-bit non-zero generation. Each
table slot stores an object identity, rights, and handle generation. Closing a
slot increments its generation, so an old value cannot resolve after reuse.
Object identities have an independent slot/generation and reference count in
the bounded registry; the last reference generations or retires the object
slot. Object IDs are created by the registry and cannot be constructed by a
user-facing caller. Creation grants one explicit creator reference; the typed
object manager releases it after publication, while handles, mappings, and
queued escrow own the references they retain.

The initial rights are `READ`, `WRITE`, `MAP`, `TRANSFER`, and `CONTROL`.
`begin_move` requires source `TRANSFER` and accepts only a requested rights
set that is a subset of the source rights. It removes the source capability
into a transfer token; there is no API that derives stronger rights from a
receiver handle. Channel send places that token in queue-owned escrow, and
receive installs it into the destination table only after destination
capacity has been checked. Queue-full, duplicate-disposition, and install
failure paths preserve the token without partial receiver authority. Table
operations return explicit errors for invalid, stale, closed, full, and
generation-exhausted handles.

`ObjectId` and object kind are small copyable identities, not authority
bypasses. All externally meaningful operations begin with a `Process` handle
resolve that checks expected kind and required rights. Raw object state is not
exposed as a substitute for a capability. VMO mapping, protection, read, and
write use Process-gated methods; Event/Timer mutation and wait-item creation do
the same. M5 can later attach these same table operations to real process
objects and syscalls.

### VMO and AddressSpace

`kernel/src/vmo.rs` provides page-aligned anonymous/shared VMO state and a
bounded `AddressSpace` mapping table. `Process::map_vmo`,
`protect_vmo`, `unmap_vmo`, `read_vmo`, and `write_vmo` resolve the VMO handle
at the operation boundary. Mapping and protection requests must be covered by
the resolved VMO capability and cannot add `WRITE` or `MAP` after creation.
Anonymous/shared backing uses bounded zeroed kernel-owned pages; successful
mapping retains the VMO object reference and unmapping releases it. Mapping
identity, offset, length, and maximum permissions are retained by the
AddressSpace. User page-table installation remains M5/M7 responsibility.

### Channel, Event, Timer, and waiting

`kernel/src/ipc.rs` provides a bounded `ChannelPair` with one queue per
endpoint. Messages have fixed-width header fields, a bounded inline payload,
and bounded outgoing handle-transfer descriptors. Sending resolves the
sender endpoint, rejects duplicate dispositions, checks queue capacity, moves
source capabilities into queue-owned escrow, and enqueues a real
`QueuedMessage`; receiver handles are not installed at send time. Receiving
resolves the receiver endpoint, checks READ right and table capacity, then
installs every escrow capability before dequeueing. Transfer tokens are
move-only queue escrow records, so a failed receive leaves the original queue
entry and its reference ownership intact. A successful send also notifies
registered READABLE waiters for the peer endpoint. Mutable object methods use
exclusive kernel references in M4; the later syscall/object manager wraps
them in the kernel's coarse lock with source-table -> channel -> destination-
table order.

`Event` is an explicit signaled/clear state. `Timer` is driven by a supplied
guest monotonic tick and never reads host time. Process-gated Event/Timer
operations require `SIGNAL`, `CONTROL`, or `WAIT` as appropriate.
`WaitItem` retains a typed source reference and refreshes readiness from the
Channel/Event/Timer state on both sides of registration. `Waiter` and
`WaitRegistry` provide bounded registration: wait_many checks readiness,
registers all interests, performs a source-backed post-registration recheck,
and marks the waiter BLOCKED when no source is ready. Registration reserves
space in the bounded wake budget; if no safe wake slot remains, wait_many
rejects the new wait instead of creating an unwakeable BLOCKED waiter.
Signal/expiry scans the registry, queues one wake record per matching waiter,
and `Waiter::sync` marks it RUNNABLE exactly once. This is the M4 kernel wait
protocol; M5 later
connects the state transition to a real process thread block.

### Guest acceptance

`kernel/src/m4.rs` creates independent Process A and Process B tables, creates
a ChannelPair, inserts a READ|WRITE|TRANSFER VMO capability in A, and sends
an actual header/payload plus a requested READ-only transfer to B. The guest
acceptance checks the received payload/header, resolves the transferred handle
in B, accepts READ, and rejects WRITE. It also exercises a VMO mapping,
Event wait, and Timer wait through real kernel data structures. No host
filesystem, socket, clock, process, or hard-coded response is used.

## Error handling and security

- Invalid, stale, wrong-kind, or closed handles fail closed and never reveal a
  slot's new object.
- Rights checks happen at the operation boundary and are not inferred from
  process ownership.
- VMO and IPC object operations exposed to the rest of the kernel use
  Process-gated methods; raw typed-state helpers remain private to their
  object module.
- Transfer source handles are moved into queue-owned escrow; receiver table
  slots are not touched until receive and all destination capacity is
  preflighted.
- Object identity generation is independent from handle-slot generation, and
  object references are retained by handles, mappings, and queued escrow.
- Queue, table, mapping, payload, and transfer counts are bounded constants.
- The implementation uses `core` only and adds no Linux/POSIX production
  dependency.

## Testing

Host unit tests cover handle encoding/decoding, generation invalidation,
attenuation, table exhaustion, VMO mapping/protection, Channel round-trip,
Event/Timer readiness, and `wait_many`. QEMU acceptance requires the guest
markers for the actual Process A -> Process B message and the rejected WRITE
strengthening attempt.
