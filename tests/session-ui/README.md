# Nagi Bar / SessionServices checks

Run `tests/session-ui/check.sh` with the pinned repository toolchain and
rustfmt, Clippy and rust-src installed. This standalone workspace leaves the
root manifest and shared CI registry unchanged.

The host suite exercises the exact production controller/input/adapter source
with scripted transport responses and actual `libnagi::security::Session`
values. It also runs PR43's worker protocol tests. Scripts are test-only
orchestration; no host model worker, inference library or canned production
response is provided. The real adapter fails closed on the host.

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
