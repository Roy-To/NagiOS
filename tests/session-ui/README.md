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

The Files consent regression compiles `wait_for_answer` verbatim from
`m19_files_client.rs` into the host harness (the build script validates the
source path and unique function boundaries). It uses finite input, display and
yield syscall scripts, real Desktop/modal handlers and the real Shared reply
queue. It covers no input, continuous modal input, reply collection, pending
cancel, lock/session loss, absent adapter and failed redraw. A locally created
unregistered request deliberately fails closed with `NotLive`; the tests neither
launch a signed child nor manufacture a granted Files permission. The scripted
reply is ready before the loop starts. Yield counts verify scheduler handoff
calls, not execution of the Nagi worker or native inference.

The Files search wait tests additionally compile the complete production client
and service sources with explicit Channel and process-exit syscall scripts.
They cover request/relay delays, normal result validation, clean reap, Lock in
receive/reap, Escape/edit/resubmit invalidation, timeout, failed presentation,
late replies, wrong sender/changed relay, deferred exit collection, and model
reply progress with idle/continuous input. The real Supervisor registry uses a
clearly marked test fixture granting a synthetic launch; the embedded host
package is empty and never launched. These tests exercise orchestration and
live grant checks, not signature verification, kernel scheduling or guest exit.
Blocking receive/wait seams panic if the foreground path accidentally calls them.
