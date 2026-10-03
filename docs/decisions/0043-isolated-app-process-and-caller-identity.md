# ADR 0043: Bounded isolated app process and kernel-stamped caller identity

Status: accepted for the M18–M23 shared service-identity prerequisite
Date: 2026-10-03
Milestones: M4 (Channel), M5 (user process), M6 (Supervisor), M19–M23 consumers

## Context

The M18–M23 audit (implementation status, M19 workstream, ADR-0002) found the
same blocker for every production service boundary: the bootstrap runs a
single shared-address-space `nagi-init` process (PID 1). Every Channel message
is therefore from PID 1, and services can only use fixture caller identities.
ADR-0002 states that authentication requires isolated processes and a trusted
supervisor that binds kernel Process IDs, endpoint delivery, and launch records
to application/session policy.

Specification §12.1 requires each process to own an independent address space,
handle table, and security/session context. AGENTS.md requires
spawn-oriented process creation, no high-level kernel syscalls, and no
capability bypass.

## Decision

1. **Spawn-oriented, init-only process creation.** Add `SYS_PROCESS_SPAWN`.
   Only PID 1 can call it, because it is the trusted Supervisor. The request
   names an ELF image in the caller's memory and one Channel endpoint handle to
   move into the child. The kernel loads the image into a new, bounded address
   space and assigns the next kernel Process ID. Fork semantics are not added.
2. **Independent address space.** The child gets its own PML4. The kernel's
   supervisor-only entries are copied. The user half contains only the child's
   own ELF pages, a bounded stack, and a guard. Nothing from the init image,
   TLS, mmap window, or Surface mappings is mapped. All user-pointer validation
   for a syscall uses the calling process's mappings. The kernel switches CR3
   when the cooperative scheduler hands off to a thread in another process.
3. **Independent handle table.** The user IPC manager keeps one handle table
   per process. Handles are process-local integers. A handle value from one
   process is meaningless in another. The only way to move an endpoint is the
   kernel transfer path: spawn or a Channel message. Its rights may be
   attenuated, never strengthened (ADR-0010).
4. **Restricted syscall surface for app processes.** Only console write, time,
   yield/sleep, random, Channel create/send/receive/wait, handle close, and
   process exit are allowed. Every other syscall returns failure for a non-init
   process. Device capabilities (block, display, input, network, audio, Model
   Store) are never delivered to the child. A child exit terminates only that
   child.
5. **Identity is kernel-stamped PID plus a supervisor launch record.** The
   kernel adds no AppId or policy concept. It stamps the sender's Process ID on
   every message, and the sender cannot change it. The Supervisor records
   `ProcessId -> (AppId, AppSessionId)` when it spawns the process. A service
   hosted by the Supervisor resolves its caller by that record. Identity fields
   in the payload are untrusted data and are never used for authorization.

## Bounds and non-goals

- One isolated child process slot. Its image is at most 1 MiB, its stack
  64 KiB, and it has one thread. More slots, mmap/TLS for children, and
  process-exit waits are later work.
- The scheduler remains cooperative with ring-3 interrupts disabled
  (ADR-0029). A child that never makes a syscall can stall the system. This is
  recorded as a known limit, not hidden.
- Ring-3 exceptions are not yet contained. There is still no TSS-backed
  privilege-transition stack (ADR-0029), so a CPU exception in either user
  process stops the machine instead of terminating only that process. The
  kernel never maps init memory into the child, so a stray child access
  faults rather than reading or writing init state. Converting child faults
  into a child-only exit is the next kernel step.
- This ADR does not claim that production Files, Browser, or Search services
  have moved out of init. It provides the authenticated caller primitive those
  services require.

## Verification

- Host unit tests must cover:
  - per-process handle isolation;
  - sender stamping under forged payload IDs;
  - transfer-only endpoint delivery;
  - init-only spawn;
  - scheduler ownership checks.
- A QEMU acceptance must prove all of the following:
  - a real second ELF runs in its own address space;
  - the child's request reaches the Supervisor-hosted service with the
    kernel-stamped child PID;
  - a forged payload identity is ignored;
  - init-only addresses and privileged syscalls are rejected for the child;
  - init continues after the child exits.
