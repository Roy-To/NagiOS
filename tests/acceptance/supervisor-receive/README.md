# Supervisor received application binding

Run `sh tests/acceptance/supervisor-receive/check.sh` with the installed repository
toolchain and existing offline cache. It compiles the exact production
`supervisor.rs` plus its host injection tests. The target build uses the real
libnagi channel receive/handle-close functions; host tests inject only those
operations into a private helper. Synthetic launch records/envelopes do not
prove a kernel launch, signature verification, real Channel or guest execution.

`try_receive_from(&Launched)` performs one nonblocking kernel receive on the
Supervisor-owned endpoint. Before and after receiving it checks the registry's
full launch record; it also requires the actual stamped sender to match the
expected launched process. The receipt constructor is private. Payload fields
cannot select the actor. A receipt borrows its launch; `live_launch()` checks
revocation again. Protocol/version/opcode/request/payload still need independent
consumer validation. The entry accepts inline messages only; unexpected received
handles are all released within the ABI bound even when the count is malformed,
and cleanup failure is reported. Empty queues are not retried or blocked on.

This is app-sender binding only. It installs no Bar endpoint/manifest, produces
no logical Bar caller from init or foreground Albert, and does not authenticate
the Desktop's user Session or intent. A summary consumer must still bind/recheck
that Session, active request and signed Bar actor, then enforce Browser sharing,
Context Authority, terms and budgets. The ordinary Desktop still supplies None
for the live Browser owner. Albert's event/Servo extraction adapter remains
unconnected; none of those gaps is satisfied by these host tests.

The full target leaf attempt may stop before compilation on pinned offline
libc0.2.191; do not fetch or substitute packages, probe denied registry paths or
report a target pass from host success. No guest, model, network, consent, public
push or merge is part of this check.
