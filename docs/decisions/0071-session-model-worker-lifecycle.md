# ADR 0071: ordinary session model worker ownership

Status: shared implementation; Desktop consumer and ordinary inference pending.

The ordinary model service already uses raw native Llama contexts. They remain
inside one bounded Nagi guest worker. The Desktop owns SessionServices and passes
its live authenticated Session; request text cannot establish identity. The
worker receives only the read-only Model Store capability, guest admission quota,
pinned terms reference and one bounded owned request. It receives no Files,
action, network, microphone or playback authority. This uses existing native
bootstrap threads and does not activate a 0.2/0.3 runtime or claim process isolation.

new configures metadata only. on_signed_in must follow authenticated Desktop
readiness and launches a 4 MiB guest stack. Each request has an independent cancel
token and its captured Session generation. Lock/signout or Session replacement
invalidates old work immediately; late output is discarded even if native work
finishes later. The worker unloads its old service before rebinding. Repeated
calls with the same Session preserve monotonic request IDs. One slot remains
occupied through completion until the UI consumes its result. Input is at most
24 KiB; output is at most 64 KiB and 256 tokens, with a 15 minute service deadline.

poll_reply cooperates with the guest scheduler and must run even without input.
The worker yields while idle and uses the actual model service's read/cancel
callbacks during work. Drop signals cancellation; it never unmaps a running
stack or waits synchronously on model I/O. A single retired worker retains its
stack until completion and join; replacement is rejected until safe reclamation.
One bridge lease serializes reclamation and prevents multiple native contexts.
The UI yields while idle or locked, including before retrying replacement after
Drop; an exiting worker cannot progress in a yield-free retry loop. Failed
unload permanently quarantines the worker across Session generations. The
worker records this at exit so a replacement is rejected for the process even
after its owned thread and stack are reclaimed. This contains uncertain native
cleanup; it does not recover resources consumed by a failing backend.

Terms confirmation is explicit, ephemeral and bound to the live Session and the
exact pinned reference. The worker applies it to its own service. Durable Store
consent and the UI confirmation/persistence flow remain separate. Guest quota
is supplied by the OS caller; machine RAM is never reported as free model memory.

Host tests are orchestration-only. Production host builds launch no inference
worker. The target harness compiles the actual bridge and actual model-service
leaves together; this does not prove guest execution. Native final linking roots
the real worker entry so absence of a Desktop consumer cannot discard it and
produce misleading link evidence. Ordinary inference still requires actual
Desktop input, verified pinned model bytes and guest response evidence.
