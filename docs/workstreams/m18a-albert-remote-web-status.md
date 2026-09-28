# M18-A Albert Remote Web Status

**Status:** `PARTIAL`  
**Branch:** `codex/m18a-albert-remote-web`  
**Required base:** `94e9a027618182b10c0ac2315e94673543f22423`  
**Scope:** remote browser networking and its user-space runtime path; no M18-B browser chrome or UI.

## Acceptance status

| Requirement | Status | Evidence / remaining work |
|---|---|---|
| Preserve the M17 PASS baseline | PARTIAL | Worktree starts at the exact fixed SHA. Kernel and UEFI loader release builds pass with BootInfo v3; rerun real M17 QEMU acceptance on the M18-A branch is pending. |
| Servo remote navigation to controlled fixture | PARTIAL | Added opt-in network initialization and a typed URL request runner. The current QEMU fixture acceptance uses HTTP then HTTPS; final target/QEMU run is pending. |
| DNS and TCP/socket transport | PARTIAL | Servo's target `getaddrinfo` resolves through the capability-scoped POSIX-to-smoltcp DNS boundary. DNS A-answer/error mapping and DNS egress are tested; the controlled page currently uses the QEMU gateway IP, so browser-originated DNS is not yet demonstrated end-to-end. TCP nonblocking calls remain bounded and fail closed. |
| HTTP and redirects | PARTIAL | Deterministic fixture tests cover HTTP redirect, POST echo, and attachment download. Guest Servo follows an HTTP-to-HTTPS redirect; QEMU evidence is pending. |
| HTTPS with chain and hostname validation | PARTIAL | Servo keeps `ignore_certificate_errors = false` and uses pinned WebPKI roots plus an additive guest-installed fixture CA. Host tests accept the fixture DNS/IP SAN and reject an untrusted chain and wrong hostname; guest TLS handshake/QEMU evidence is pending. |
| Timeout, reset, DNS, and TLS failure behavior | PARTIAL | POSIX maps DNS failure, timeout, reset, and would-block distinctly; tests cover the mapping. Host TLS tests cover untrusted and hostname failures. Deterministic browser-visible guest failure states remain unverified. |
| Download and upload transport | PARTIAL | HTTPS fixture host tests verify attachment bytes and POST echo. The guest page runs sequential HTTPS fetch upload/download and gates its title on both results; QEMU evidence is pending. |
| Remote page rendered into Nagi Surface | PARTIAL | The M18-A QEMU gate requires the HTTPS transfer title, fixture identity, nonzero Servo frame checksum, and remote-specific Surface marker; current-source target execution is pending. |

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

## Checkpoint 3 — verified HTTPS and transfer substrate

Added a controlled TLS fixture with a private test root kept separate from the
checked-in server key, a server leaf containing `m18a.test` and `10.0.2.2`
SANs, and an HTTP-to-HTTPS redirect. The guest installs only this fixture root
through its own POSIX/VFS path; Servo keeps normal chain/hostname validation
enabled and retains the pinned WebPKI roots. The page performs a sequential
HTTPS POST and attachment GET and updates its title only after both responses
match the expected bytes. Host tests reject untrusted chains and wrong
hostnames. No development-host certificate store is used.

## Checkpoint 4 — guest realtime seed for TLS validity

Added UEFI RTC sampling before `ExitBootServices`, validated conversion to Unix
nanoseconds, and BootInfo v3 transport to the kernel's existing realtime
syscall. Missing, invalid, daylight-adjusted, pre-epoch, or unrepresentable
times remain unavailable and fail closed. QEMU uses `-rtc base=utc`; no host
clock or UEFI runtime service is consulted after boot. The architectural
change is recorded in `docs/decisions/0037-m18a-uefi-realtime-seed-for-tls.md`.

Verification for this checkpoint:

- `cargo test --locked --target x86_64-apple-darwin -p nagi-net -p nagi-posix -p nagi-albert -p nagi-abi -p nagi-bootinfo` — PASS (37 unit/integration tests).
- `PYTHONDONTWRITEBYTECODE=1 python3 tests/fixtures/m18a/test_server.py` — PASS (10 HTTP/TLS tests).
- Scoped `cargo clippy ... -- -D warnings` for the five changed Rust packages — PASS.
- Scoped Rust formatting and `sh -n tests/acceptance/m18a_albert_remote_web.sh` — PASS.
- Kernel release build for `targets/x86_64-unknown-nagi.json` — PASS.
- UEFI loader release build for `x86_64-unknown-uefi` — PASS.
- `git diff --check` — PASS.
- Servo target init build and current-source QEMU run — pending public Ubuntu CI; the earlier run tests only the HTTP checkpoint and is not evidence for the TLS/RTC changes.

## Exact next action

Push this verified code checkpoint and use its Ubuntu CI target job to build
M18-A and rerun both real-QEMU M17 and HTTPS remote-page acceptance. Repair any
target/QEMU failure without weakening certificate checks. Keep M18-A `PARTIAL`
until the current-source QEMU markers pass and Servo-originated DNS plus
browser-visible network failure behavior have deterministic evidence.

Shared-file changes are limited to the CI acceptance workflow, `Cargo.lock`,
the global implementation-status summary, the kernel/loader BootInfo realtime
contract, and QEMU's deterministic UTC RTC option. These changes are required
to build and verify the M18-A guest path and are recorded here for M18-B
integration.
