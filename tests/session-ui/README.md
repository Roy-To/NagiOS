# Nagi Bar / SessionServices checks

Run `tests/session-ui/check.sh` with the pinned repository toolchain and
rustfmt, Clippy and rust-src installed. This standalone workspace leaves the
root manifest and shared CI registry unchanged.

The host suite exercises the exact production controller/input/adapter source
with scripted transport responses and actual `libnagi::security::Session`
values. It also runs PR43's worker protocol tests. Scripts are test-only
orchestration; no host model worker, inference library or canned production
response is provided. The real adapter fails closed on the host.

The dedicated `desktop-host` workspace additionally includes the real Desktop,
LoginScreen, credential verification, VFS and font renderer. It replaces only
the host syscall boundary: block transport is in-memory, readiness is scripted,
and time/console are deterministic. Entropy panics to prohibit accidental account
creation. The shim refuses to compile for Nagi and supplies no inference.
The regressions exercise existing-account sign-in, F4 lock, identical initial
unlock frames and relogin in both locales, failed readiness, and the retained
legacy frame/login acceptance. These are host orchestration/render regressions,
not guest or native-worker acceptance.
Populated-state cases seed the controller through a test-only scripted transport
and then use actual F4/Lock handlers and the real adapter to clear queued,
in-flight and displayed Bar state. They compare the complete lock frame and
reopen the Bar after relogin. Controller regressions also cover failed cancel
calls and fresh tokens for the same account. The complete `run` loop is NOT_RUN.
The legacy login unit configuration omits Files runtime because its inherited
host tests do not initialize their acceptance-only fields. This does not alter
any production feature dependency or Files source; full Files/guest acceptance
is not claimed by the login unit check.

The Nagi-target Clippy step compiles the actual worker, model service and Bar
adapter against the production API. The `desktop-check` step additionally
compiles the exact Desktop/login/Files runtime sources under ordinary model
and authenticated-session features. It does not enable signed Files IPC or
consent acceptance features, does not link native C++ archives, and never
boots a guest. Existing POSIX/Files dead-code warnings remain visible.

Actual ordinary-session model output and lock/relogin acceptance are NOT_RUN.
They require the production image, pinned model bytes, explicit user terms
acceptance and QEMU evidence. See the dedicated technical handoff in
`docs/workstreams/desktop-session-ai-integration.md`.
