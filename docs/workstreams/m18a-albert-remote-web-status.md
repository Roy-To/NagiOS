# M18-A Albert Remote Web Status

**Status:** `PARTIAL`  
**Branch:** `codex/m18a-albert-remote-web`  
**Required base:** `94e9a027618182b10c0ac2315e94673543f22423`  
**Scope:** remote browser networking and its user-space runtime path; no M18-B browser chrome or UI.

## Acceptance status

| Requirement | Status | Evidence / remaining work |
|---|---|---|
| Preserve the M17 PASS baseline | PARTIAL | Worktree starts at the exact fixed SHA. Re-run M17 acceptance after M18-A changes; the CI job now does so before M18-A. |
| Servo remote navigation to controlled fixture | PARTIAL | Added opt-in network initialization and a Servo URL runner. Target/QEMU acceptance is pending. |
| DNS and TCP/socket transport | PARTIAL | Existing `nagi-net` smoltcp DNS/TCP path is capability-scoped. This checkpoint adds POSIX `O_NONBLOCK` retention and bounded TCP try-send/try-receive; concurrent socket support and deterministic end-to-end DNS remain. |
| HTTP and redirects | PARTIAL | Added a deterministic fixture with a 302 redirect; host tests verify the final downloaded HTML. Servo's target request through the guest stack remains pending. |
| HTTPS with chain and hostname validation | NOT STARTED | Servo's pinned rustls/WebPKI path and ADR 0030 root policy are present; controlled guest validation coverage is still required. No trust bypass is allowed. |
| Timeout, reset, DNS, and TLS failure behavior | PARTIAL | TCP pre-connect operations fail closed and existing timeout/reset errors remain. Add deterministic failure tests and precise error propagation. |
| Download and upload transport | PARTIAL | Fixture host tests cover redirect/download content and POST body echo. Browser transfer through the guest socket boundary remains pending. |
| Remote page rendered into Nagi Surface | PARTIAL | The M18-A QEMU gate requires fixture page identity, a nonzero Servo frame checksum, and the remote-specific Surface marker; target execution remains pending. |

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

## Checkpoint 2

Added the opt-in `m18a-remote-web` init feature. It reuses the M17 storage and
Servo bootstrap, initializes the process's network capability, and opens the
controlled fixture URL through Servo. The fixture redirects to a titled HTML
page; the embedder requires that page identity before it accepts a nonzero
frame presented to Nagi Surface. The M17 path keeps its original feature,
local data page, marker, image, and log names. Added `nagi m18a`, host fixture
tests for redirected downloads and POST uploads, and an Ubuntu target CI step
that runs M17 acceptance before M18-A.

Host verification:

- `cargo test --locked -p nagi-cli --tests` — PASS (92 library tests and 18 CLI integration tests).
- `cargo test --locked --target x86_64-apple-darwin -p nagi-net -p nagi-posix` — PASS (19 unit tests, 1 integration test, doc tests).
- `python3 tests/fixtures/m18a/test_server.py` — PASS (redirected download and POST body echo).
- Scoped Rust formatting, shell syntax, Python compilation, and `git diff --check` — PASS.
- Target/QEMU attempt on macOS stopped in Mesa configure: its ELF link check
  selected `ld64.lld`, which rejects `--entry=0` and
  `--unresolved-symbols=ignore-all`; Meson then reports missing `libatomic`.
  No M18-A target binary or QEMU run was produced locally.
- GitHub Actions now runs the same M18-A acceptance on Ubuntu after M17; result pending.

## Next actions

1. Run the new Ubuntu M18-A target/QEMU CI acceptance and repair its first
   target or transport failure without weakening M17 or certificate checks.
2. Exercise DNS, redirect, HTTP errors, timeout/reset, HTTPS chain and hostname
   validation, and browser upload/download transport with deterministic tests.
3. Re-run M17 acceptance, update this status from evidence, and push each
   verified checkpoint.
