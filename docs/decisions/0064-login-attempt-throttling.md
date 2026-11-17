# ADR 0064: Throttling failed sign-in attempts

Status: accepted
Date: 2026-10-06
Builds on: ADR 0063 (desktop owner login)

## Context

ADR 0063 left the lock screen without a rate limit beyond the cost of the
PBKDF2 key derivation. A local attacker could keep guessing, and a restart
reset nothing.

## Decision

1. **Policy.** `libnagi::login::LoginThrottle` is a host-tested policy.
   - The first three consecutive failures are free.
   - After the third, the next attempt waits 5 s.
   - Each further failure doubles the wait, up to 60 s.
   - A successful sign-in resets the count.
   - Times are a caller-supplied monotonic clock in milliseconds; in the
     guest, the kernel's 10 ms timer ticks.
2. **No check during a wait.** An attempt made during the wait is refused
   *before* the key derivation runs. The password is cleared, and the
   screen shows the localized "Please wait" / 「待機中」 notice. A wait
   costs an attacker time and cannot be used to make the guest run PBKDF2.
3. **Persistence.** The consecutive-failure count is written to User Data
   `login-throttle` (`NLT1` and a little-endian u32) after every attempt.
   - At boot, a count past the free attempts starts the wait from that
     boot, so a restart does not skip it.
   - A missing file means zero failures.
   - An unreadable or corrupt file fails closed to the free-attempt limit,
     which imposes one wait.
4. **End of the wait.** While a wait is running, the desktop's idle loop
   polls the clock. When the wait ends, it prints `Nagi login retry
   allowed` and clears the notice.
   - The notice appears only after an attempt during the wait, not on the
     first frame.
   - Otherwise a refused attempt would leave the screen identical to the
     first frame, which the desktop's "a handled event changes the frame"
     check rejects.

## Acceptance

`./nagi login` passed on the arm64 macOS host (evidence
`out/evidence/login-1791281978301774000`). It runs three boots on one
disk:

1. **create** — choose the language and create the owner.
2. **throttle** — three wrong passwords, each `unlock REJECTED`, then
   `throttle engaged failures=3`.
3. **restart** — the boot prints `throttle restored failures=3`. The
   correct password typed immediately is refused unchecked
   (`throttled remaining_ms=…`). After `retry allowed`, the same password
   unlocks.

`./nagi consent`, `m27`, `m29` and `m30-update` still pass. While
re-running them, the M27/m30-update readiness-order check was relaxed to
accept a readiness line whose trailing `PASS` had not been written yet.
QEMU stops on the line's prefix, and the kernel prints that prefix only
after a successful write.

## Bounds

- **Attacker model.** The limit is per owner account and local to the
  guest. Someone who can rewrite User Data offline can reset it; disk
  encryption is out of scope for 0.1.
- **Clock.** The clock is the kernel's coarse 10 ms tick, which keeps
  advancing while ring 3 runs. The desktop loop polls it while a wait is
  running. An earlier draft of this ADR said a sleep could not be used in
  the single-threaded desktop init. That was wrong: `sleep_ns` works there,
  checked with 300 consecutive 10 ms sleeps after the first frame. The halt
  observed during development was init exiting on the desktop's
  frame-change check, not the sleep.
