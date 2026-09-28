# M18-A Albert Remote Web Status

**Status:** `PARTIAL`  
**Branch:** `codex/m18a-albert-remote-web`  
**Required base:** `94e9a027618182b10c0ac2315e94673543f22423`  
**Latest pushed code checkpoint:** `ebe2395e19e4c40f26e68e9827901b33ad3a0460`
**Latest CI:** run `36381892343` — Ubuntu host and Windows launcher passed; target dependencies, Mesa, kernel, user init, and UEFI loader built; M17 QEMU acceptance passed; M18-A QEMU acceptance failed on `Nagi M18A remote navigation FAIL fixture identity` before remote Surface acceptance.
**Current local checkpoint:** Added guest URL/page-title diagnostics and early QEMU termination on guest `Nagi ... FAIL` markers; focused host checks pass, pending commit/push and target rerun.
**Scope:** remote browser networking and its user-space runtime path; no M18-B browser chrome or UI.

## Acceptance status

| Requirement | Status | Evidence / remaining work |
|---|---|---|
| Preserve the M17 PASS baseline | PASS | On the M18-A branch, CI run `36381892343` rebuilt the target and passed the real M17 first-web-pixel QEMU acceptance before M18-A. |
| Servo remote navigation to controlled fixture | PARTIAL | The M18-A guest booted and Servo reported a completed load, but the loaded title did not match either allowed fixture title. Added URL and title serial diagnostics to identify the failed navigation on the next run. |
| DNS and TCP/socket transport | PARTIAL | Servo's target `getaddrinfo` resolves through the capability-scoped POSIX-to-smoltcp DNS boundary. DNS A-answer/error mapping and DNS egress are tested; the controlled page uses the QEMU gateway IP, so browser-originated DNS is not demonstrated end-to-end. TCP nonblocking calls remain bounded and fail closed. |
| HTTP and redirects | PARTIAL | Deterministic fixture tests cover HTTP redirect, POST echo, and attachment download. Guest load completed, but the current QEMU log does not prove which response or redirect reached Servo. |
| HTTPS with chain and hostname validation | PARTIAL | Servo keeps `ignore_certificate_errors = false` and uses pinned WebPKI roots plus an additive guest-installed fixture CA. Host tests accept the fixture DNS/IP SAN and reject untrusted chains and wrong hostnames. Guest HTTPS completion is not yet established by QEMU. |
| Timeout, reset, DNS, and TLS failure behavior | PARTIAL | POSIX maps DNS failure, timeout, reset, and would-block distinctly; tests cover the mapping. Host TLS tests cover untrusted and hostname failures. Browser-visible DNS/timeout/reset outcomes remain unverified. |
| Download and upload transport | PARTIAL | HTTPS fixture host tests verify attachment bytes and POST echo. The guest page runs sequential HTTPS fetch upload/download, but the failed navigation produced no guest transfer marker. |
| Remote page rendered into Nagi Surface | PARTIAL | The M18-A QEMU gate requires the HTTPS transfer title, fixture identity, nonzero Servo frame checksum, and remote-specific Surface marker. M17 QEMU passed; M18-A produced no remote Surface marker. |

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

Added a controlled TLS fixture with a test CA whose signing key is not
committed, a dedicated server leaf key containing `m18a.test` and `10.0.2.2`
SANs, and an HTTP-to-HTTPS redirect. A separate self-signed server endpoint
has a matching IP SAN but no trusted issuer. The guest installs only the
fixture root through its own POSIX/VFS path; Servo keeps normal chain/hostname
validation enabled and retains the pinned WebPKI roots. The page performs a
sequential HTTPS POST and attachment GET, then requires Servo to reject the
self-signed endpoint before it marks the run successful. Host tests reject
untrusted chains and wrong hostnames. No development-host certificate store
is used.

## Checkpoint 4 — guest realtime seed for TLS validity

Added UEFI RTC sampling before `ExitBootServices`, validated conversion to Unix
nanoseconds, and BootInfo v3 transport to the kernel's existing realtime
syscall. Missing, invalid, daylight-adjusted, pre-epoch, or unrepresentable
times remain unavailable and fail closed. QEMU uses `-rtc base=utc`; no host
clock or UEFI runtime service is consulted after boot. The architectural
change is recorded in `docs/decisions/0037-m18a-uefi-realtime-seed-for-tls.md`.

Verification for this checkpoint:

- `cargo test --locked --target x86_64-apple-darwin -p nagi-net -p nagi-posix -p nagi-albert -p nagi-abi -p nagi-bootinfo` — PASS (37 unit/integration tests).
- `PYTHONDONTWRITEBYTECODE=1 python3 tests/fixtures/m18a/test_server.py` — PASS (11 HTTP/TLS tests).
- Scoped `cargo clippy ... -- -D warnings` for the five changed Rust packages — PASS.
- Scoped Rust formatting and `sh -n tests/acceptance/m18a_albert_remote_web.sh` — PASS.
- Kernel release build for `targets/x86_64-unknown-nagi.json` — PASS.
- UEFI loader release build for `x86_64-unknown-uefi` — PASS.
- `git diff --check` — PASS.
- GitHub Actions `36379279390` on `330f322`: Ubuntu host and Windows launcher — PASS. `nagi-target` failed in `Build Nagi user init` at `user/nagi-albert/src/lib.rs:9`: `pub(super)` is invalid at crate root (`too many leading super keywords`). The UEFI loader and both QEMU acceptance steps were skipped.
- Corrected the guest module declaration to `pub(crate)` so the sibling remote-web module can use it without exporting the module outside the crate. Focused host tests (37) and scoped Clippy pass after this change; target/QEMU confirmation is pending.
- An isolated local custom-target `cargo check -p nagi-albert` did not reach the crate: registry `libc` 0.2.174 failed compiling its target bindings. Use the repository's full target build path in CI for acceptance.

## Checkpoint 5 — first M18-A target/QEMU run and diagnostics

CI run `36381892343` on pushed HEAD `ebe2395e19e4c40f26e68e9827901b33ad3a0460`
passed both host jobs, built the M18-A target and UEFI loader, and passed M17's
real QEMU first-web-pixel acceptance. The M18-A QEMU run then emitted
`Nagi M18A remote navigation FAIL fixture identity` and did not produce its
remote frame/Surface markers. The acceptance runner waited the full 120-second
timeout after a guest failure marker, which obscured iteration and the actual
page state.

The current local checkpoint logs remote WebView URLs and completed page
titles, and stops QEMU as soon as a guest `Nagi ... FAIL` marker appears. A
focused `nagi-cli` host test covers failure-marker detection; all 93
`nagi-cli` library tests, scoped clippy with warnings denied, changed-file
formatting, and `git diff --check` pass locally. The guest title/URL diagnostic
and M18-A remote path require the next target CI run.

Next, commit and push this diagnostic checkpoint, rerun CI from the fixed M17
base, then use the emitted URL/title to repair the first failed navigation.
Keep certificate validation enabled. M18-A stays `PARTIAL` until remote
navigation, HTTPS transfers, untrusted-chain rejection, and Surface markers
pass, with browser-originated DNS and browser-visible timeout/reset behavior
still needing deterministic evidence.

Shared-file changes are limited to the CI acceptance workflow, `Cargo.lock`,
the global implementation-status summary, the kernel/loader BootInfo realtime
contract, and QEMU's deterministic UTC RTC option. These changes are required
to build and verify the M18-A guest path and are recorded here for M18-B
integration.
