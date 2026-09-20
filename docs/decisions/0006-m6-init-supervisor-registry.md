# M6 Init, Supervisor, and Service Registry Decision

Status: accepted for Nagi OS 0.1 M6

M6 keeps service discovery and lifecycle policy in user space. The kernel is
not extended with service, manifest, file, or process-management syscalls;
existing native console and process-exit syscalls are used only for the
bootstrap diagnostic path. This preserves the kernel boundary and allows the
registry implementation to move to Channel IPC when multi-process services
are introduced.

The bootstrap implementation is deliberately bounded: eight registry slots,
four dependencies per manifest, fixed-size service names, and a finite
restart budget. Service identities are `(name bytes, API version)` rather than
PIDs or executable names. Resolution returns a generation-checked handle, and
calls are rejected for stale handles or unavailable health states.

The supervisor computes dependency order deterministically, exposes explicit
Starting/Ready/Healthy/Failed/Restarting/CrashLoop states, and never retries
past the manifest's restart budget. The M6 acceptance service is a real
user-space `echo@1` endpoint invoked by `nagi-init`; its response is produced
by copying the bounded request through the registered handler, not by a canned
acceptance result. The kernel emits the terminal M6 acceptance marker only
after the user process reaches the successful existing process-exit syscall,
so the marker remains contingent on that verified call.
