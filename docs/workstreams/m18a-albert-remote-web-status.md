# M18-A Albert Remote Web Status

**Status:** `PARTIAL`  
**Branch:** `codex/m18a-albert-remote-web`  
**Required base:** `94e9a027618182b10c0ac2315e94673543f22423`  
**Scope:** remote browser networking and its user-space runtime path; no M18-B browser chrome or UI.

## Acceptance status

| Requirement | Status | Evidence / remaining work |
|---|---|---|
| Preserve the M17 PASS baseline | PARTIAL | Worktree starts at the exact fixed SHA. Re-run M17 acceptance after M18-A changes. |
| Servo remote navigation to controlled fixture | NOT STARTED | Add a dedicated remote-page runner and deterministic fixture, then verify in QEMU. |
| DNS and TCP/socket transport | PARTIAL | Existing `nagi-net` smoltcp DNS/TCP path is capability-scoped. This checkpoint adds POSIX `O_NONBLOCK` retention and bounded TCP try-send/try-receive; concurrent socket support and deterministic end-to-end DNS remain. |
| HTTP and redirects | NOT STARTED | Replace the current fixed M12 HTTP helper as needed for Servo, and add controlled redirect coverage. |
| HTTPS with chain and hostname validation | NOT STARTED | Servo's pinned rustls/WebPKI path and ADR 0030 root policy are present; controlled guest validation coverage is still required. No trust bypass is allowed. |
| Timeout, reset, DNS, and TLS failure behavior | PARTIAL | TCP pre-connect operations fail closed and existing timeout/reset errors remain. Add deterministic failure tests and precise error propagation. |
| Download and upload transport | NOT STARTED | Verify browser request body and response streaming through the guest socket boundary. |
| Remote page rendered into Nagi Surface | NOT STARTED | M17 verifies local Servo rendering only; M18-A must prove remote content reaches the same Surface. |

## Checkpoint 1

Added socket status-flag persistence in `nagi-posix`, bounded nonblocking TCP
send/receive entry points in `nagi-net`, and a fail-closed regression assertion
for operations before connect. The Nagi-only `ERRNO` section annotation now
applies only to the Nagi target, allowing the existing x86_64 macOS host test
configuration to link without changing the guest linker placement.

Verification:

- `cargo test --locked --target x86_64-apple-darwin -p nagi-net -p nagi-posix` — PASS (19 unit tests, 1 integration test, doc tests).
- `git diff --check` — PASS.
- M17 target build/QEMU acceptance — pending.
- HTTPS/TLS and controlled remote-page acceptance — pending.

## Next actions

1. Add an opt-in M18-A guest entry that initializes the passed network
   capability before Servo and navigates to a controlled fixture URL, while
   leaving the M17 entry path unchanged.
2. Add a fixture-backed `nagi m18a` QEMU acceptance and require both remote
   load completion and a nonzero Servo frame presented to Nagi Surface.
3. Exercise DNS, redirect, HTTP errors, timeout/reset, HTTPS chain and hostname
   validation, and browser upload/download transport with deterministic tests.
4. Re-run M17 acceptance, update this status from evidence, and push each
   verified checkpoint.
