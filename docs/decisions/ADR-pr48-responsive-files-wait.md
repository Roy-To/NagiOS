# Responsive Files Search waits

Status: implemented locally; host validated; guest validation pending

The signed Files route must process UI input and bounded model orchestration
while awaiting its request, relay and exit. Existing blocking Channel receive
and process wait retain their contracts for other callers. Add an init-only
nonblocking process exit poll (syscall 40): ready consumes exactly one status,
live returns pending without registering a waiter, invalid returns error. A
registered blocking waiter excludes polling. This is a low-level process API,
not a Files syscall.

The foreground Files wait uses nonblocking Channel receive and exit polling,
yields every iteration, and checks owner session and query revision before
accepting messages or displaying results. Lock, query edit, resubmit and Escape
invalidate old work. A fixed 3000 tick budget covers request/relay/exit waiting
(excludes user consent). Failure sends abort, immediately revokes the launch
and closes its endpoint. One retained PID is polled in subsequent Desktop
iterations until exit; new Files launches are refused while it remains pending.
This bounds resource use without claiming a stalled child has exited or adding
unsafe termination. No result is accepted before verified clean reap.

Synchronous runtime search evaluation, VFS synchronization and consent user
think time are outside this waiting fix. No guest acceptance is inferred from
host syscall scripts. The private product ledger was unavailable (authenticated
404); this implements existing public responsiveness and revocation contracts.
