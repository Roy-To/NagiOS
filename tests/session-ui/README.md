# Nagi Bar host regressions

Run `sh tests/session-ui/check.sh` on the x86_64 Linux host used by CI, with
the repository's pinned Rust toolchain.
This standalone workspace keeps the host regression suite independent from the
root workspace and does not build a Nagi target or native model library.

The controller suite compiles the production `session_ui`, `bar_adapter`, and
`bar_panel` sources against real `Session` values and an explicit scripted
transport. It covers readiness, session binding, lock, cancellation, delayed and
late replies, user switching, terms acknowledgement, bounded UTF-8 input and
output, localization, and control-character-safe rendering. Scripted replies are
for orchestration only; the host adapter creates no model worker or inference.

The `desktop-host` workspace compiles the real Desktop, LoginScreen, consent
dialog/settings, Supervisor, SessionServices and font renderer. Its only platform
substitutions are deterministic syscall/readiness/time/console and in-memory
block-device seams. It runs the production `SessionServices` unit tests over the
real `Shared` state, as well as Desktop lock/relogin rendering, consent-modal
lock-precedence, and close/reopen while a scripted reply drains through the real
Shared queue. The scripted backend does not spawn the Nagi worker. No guest
result or inference acceptance is claimed.
