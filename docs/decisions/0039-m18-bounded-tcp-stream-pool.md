# ADR 0039: M18 bounded TCP stream pool

Status: accepted for M18
Date: 2026-09-29
Milestone: M18 — Albert Browser

## Context

The first real HTTPS page rendered in QEMU, but the next page could not open a
connection while Servo retained the first page's keep-alive TCP stream. The
existing M17 bootstrap `SocketApi` owns one smoltcp TCP socket and returns
`Unsupported` for any additional live stream. Reusing that socket for another
peer would corrupt descriptor-to-connection ownership, while aborting the old
stream would break valid browser keep-alive and concurrent requests.

M18 therefore needs more than one TCP connection without adding a kernel
network syscall, a host socket dependency, or an unbounded socket table.

## Decision

- Keep the single-stream M17 capacity unchanged.
- Under the existing M18 browser-network feature, provide four fixed TCP
  socket slots and fixed 2 KiB RX/TX buffers per slot.
- Return an opaque user-space `TcpConnectionId` from connect and require it for
  every send, receive, readiness, option, shutdown, close, and local-name call.
  POSIX file descriptors retain the ID that was assigned to their connection.
- Keep one smoltcp interface/socket set for the pool and serialize its access
  through the existing capability-scoped user-space network service and
  cooperative M18 lock.
- Reuse a slot only after smoltcp removes its socket. Exhaustion continues to
  fail closed with `Unsupported`.

## Consequences

- M18 can keep up to four guest TCP streams alive while Servo loads different
  HTTPS origins; the configured buffer memory is bounded at 16 KiB.
- M17 keeps its one-stream resource bound and legacy wrapper behavior.
- The new ID stays inside `nagi-net` and the POSIX user-space adapter. The
  kernel still exchanges only capability-checked Ethernet frames.

## Verification

Focused network/POSIX tests and the M18 QEMU acceptance must verify descriptor
IDs remain associated with their own smoltcp sockets, free slots can be used
while another stream is retained, and M17 still builds with one slot.
