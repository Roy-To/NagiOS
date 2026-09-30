# Nagi OS M29 — Developer Preview Polish Workstream

## Current state: PARTIAL

This checkpoint improves onboarding and SDK documentation and audits the
preview-facing surfaces named in the M29 specification. It does not claim a
finished consumer onboarding flow or complete product UX.

## Audit

| Surface | Current evidence | M29 result / remaining work |
| --- | --- | --- |
| First boot and onboarding | `./nagi image` and `./nagi run` create and boot the QEMU reference image; the implementation status records QEMU acceptance by milestone. | The Japanese guide documents the host and first QEMU workflow. No first-run setup wizard, physical installer, or onboarding screenshots are present. |
| Desktop, defaults, and Settings | `user/nagi-init/src/desktop.rs` draws four fixed M10 acceptance panels for Calculator, Notes, Files, and Terminal. | The guide labels this as an acceptance surface. A real app launcher, default-app selection, and integrated Settings experience are not evidenced. |
| Errors and missing providers | CLI commands expose host diagnostics and per-acceptance serial logs; M20–M26 workstreams record typed provider boundaries and unavailable paths. | The guide separates host failures from guest providers and says when a capability is not available. There is no shared end-user error center or provider-management UI. |
| Recovery and update | M16 verifies a sample package install/list/info/launch/atomic-update/remove fixture. M27 QEMU acceptance covers GPT-backed A/B trial rollback/promotion and a read-only Recovery path, including recovery with a pending journal. | Documentation distinguishes the M16 fixture from M27 guest recovery. Authenticated slot manifests, GPT-integrated update installation, account-login readiness, and a user-facing recovery flow remain. |
| Language and accessibility | `docs/architecture/language-architecture.md` defines `en-US` and `ja-JP` as equal first-class languages. The M10 desktop has fixed English labels and a Japanese sample string; Albert's view model exposes localization keys. | The guide does not imply complete translation or accessibility. Full locale controls, complete first-party translations, an accessibility tree, and assistive-technology acceptance remain unverified. |
| Diagnostics and debug output | `./nagi doctor` is host diagnostics; it recognizes `python3` as well as the `python` and Windows launcher names. Milestone commands save guest serial logs. The M17 acceptance requires bounded startup trace markers, and `m17_trace_excerpt` elides middle lines past its configured cap. | The guide documents log paths and marker-based evidence. The Python 3 alias regression is covered by an integration test, and the current macOS host reports 12/12 checks passing. M17 traces were retained because they support and are consumed by startup acceptance; no indiscriminate trace deletion was made. A `nagi diagnose bundle` command is absent. |
| Developer and SDK docs | Root README, Japanese Developer Preview guide, SDK README, contribution guide, and roadmap now cross-link the verified command surface and known limitations. The SDK README describes the IDL-backed Rust/C APIs and the Hello Nagi sample package flow. | Documentation is present. The SDK remains an early surface; no general app lifecycle, IPC, or capability API is claimed. |
| Package metadata and notices | `nagi.toml` records version `0.1.0-dev` and the QEMU reference machine. `THIRD_PARTY_NOTICES.md` now covers all 18 `sources.lock` components, their exact version/revision/toolchain pins, declared license expressions, and unresolved binary redistribution reviews. A `nagi-cli` regression checks that new pins are added to the notice inventory. `cargo metadata --locked` reports license expressions for all 673 external Cargo packages (41 distinct expressions), including dev and target-specific packages. | The guides link the notices and avoid assigning a project license. Cargo metadata is not a license-text or redistribution review or an image bill of materials. Nagi's license is still undecided; Rust std, Mesa component notices, transitive native sources, and asset provenance need human review before binary redistribution. |
| Screenshots and performance | `docs/assets/screenshots/nagi-m10-qemu-desktop.png` is the M10 QEMU desktop after keyboard/mouse acceptance. `docs/assets/screenshots/nagi-m18-qemu-browser.png` is Albert after three verified HTTPS pages reached the guest surface. Three repeated persistent-data M10 QEMU boots on macOS aarch64 reached READY in 2,466–2,568 ms (median 2,486 ms), measured from QEMU spawn to host receipt of the marker. | These are fixed M10/M18 acceptance surfaces, not finished product UIs. The timing includes QEMU/UEFI startup and serial delivery; it is not a clean-install or cross-host performance benchmark. |
| Clean build | CI checks build from fresh checkouts. This local documentation checkpoint did not run `./nagi clean`, which removes `target/` and `out/` including preserved acceptance logs and persistent disks. | Existing evidence remains preserved. A clean release build and artifact reproducibility are part of the M30 release gate. |

## Captured M10 acceptance surface

This is a real guest display captured after M10 QEMU input acceptance. It
documents the current fixed four-panel surface and does not imply a complete
desktop shell or complete Japanese localization.

![Nagi M10 fixed QEMU acceptance desktop](../assets/screenshots/nagi-m10-qemu-desktop.png)

## Captured M18 Albert browser acceptance

The M18 acceptance runner now captures QEMU's display over QMP after the
guest reports all three HTTPS pages rendered. The saved image is from the
last accepted page (`example.net`) and shows Albert's tab, navigation controls,
address bar, and rendered Example Domain content. It is an acceptance snapshot,
not a claim that browser UX or localization is complete.

![Nagi M18 Albert browser after three-page HTTPS acceptance](../assets/screenshots/nagi-m18-qemu-browser.png)

## Verification

- The POSIX root launcher now prefers Cargo beside the discovered rustup
  executable and prepends that directory so Cargo also resolves its matching
  rustc shim. This avoids an x86_64 Homebrew Cargo/rustc pair shadowing the
  repository-pinned ARM64 nightly on this macOS host. The M0 acceptance uses
  fake PATH entries to verify both shim selection and the rustc lookup.
- `./nagi --help` printed the supported command list and `./nagi doctor`
  reported 12/12 checks with the original PATH. The complete
  `sh tests/acceptance/m0_launcher.sh` acceptance also passed, including its
  image build.
- A local Markdown-link audit checked 47 relative links across the root
  README, contribution guide, roadmap, Developer Preview guide, SDK README,
  and this workstream; all resolve.
- `cargo test --locked --offline -p nagi-cli --lib third_party_notices::third_party_notices_cover_every_pinned_component_and_declared_license`
  — passed; every source-lock component, version/revision/toolchain pin, and
  declared license expression is represented in `THIRD_PARTY_NOTICES.md`.
- `python3 tools/audit_cargo_license_metadata.py` — passed for all 673 external
  locked packages; zero packages lacked a declared license expression. The
  graph includes dev and target-specific dependencies and is not an image bill
  of materials. This is metadata coverage, not license-text or redistribution
  approval.
- `doctor_recognizes_python3_without_a_python_alias` — passed after confirming
  it failed against the prior candidate list; `./nagi doctor` then passed on
  macOS with 12 checks, including `/opt/homebrew/bin/python3`.
- `cargo test --locked --offline -p nagi-cli --all-targets` — passed (135 unit
  tests, 21 integration tests); `cargo clippy --locked --offline -p nagi-cli
  --all-targets -- -D warnings` — passed.
- `rustfmt --check` for the changed Rust files and `git diff --check` — passed.
- `./nagi desktop` — passed on 2026-09-30 after the scanout and bitmap-font
  changes; the guest reported READY, four app-focus markers, Japanese input,
  and `Nagi M10 acceptance PASS`. QEMU QMP saved the guest display as a
  1280×800 PNG. Its SHA-256 is
  `061c02741343026c2b6974ae846fbbf26bde48b8e00d0160abe918da2744932e`.
- Three real `./nagi desktop` runs passed with the same guest
  acceptance markers. Host times from QEMU child spawn to receipt of
  `Nagi M10 desktop READY` were 2,486 ms, 2,466 ms, and 2,568 ms (median
  2,486 ms). Each run's image, OVMF vars, serial logs, persistent-data snapshot,
  and measurement are preserved under
  `out/evidence/m29-desktop-timing-sample-{1,2,3}/`.
- The kernel scanout geometry test is configured in Ubuntu-host CI because the
  local macOS host is AArch64 and cannot compile the kernel's x86 inline
  assembly as a native host test. The real x86-64 guest acceptance above
  exercises the 1280×800 full-scanout path.

## POSIX launcher toolchain selection — 2026-10-01

The standard launcher initially selected `/usr/local/bin/cargo` from PATH,
which attempted to link the CLI for x86_64 on an ARM64-only Command Line Tools
installation. Selecting only the rustup Cargo proxy still allowed Cargo to
find the Homebrew rustc by name. The launcher now selects Cargo next to rustup
and places that shim directory first in PATH, keeping Cargo and rustc on the
same pinned host toolchain. The mocked M0 regression failed before the fix and
passed after it; the full M0 launcher/image acceptance, `./nagi doctor`
(12/12), and `./nagi --help` passed after the change.

## ARM64 host workspace checks — 2026-10-01

The root `build`, `test`, and `lint` commands previously selected every
workspace package except the kernel. On an ARM64 host that also compiled
`libnagi`'s x86_64 syscall-register stubs and target-only service packages as
host code. The CLI now keeps the full host workspace selection on x86_64 and
selects an explicit set of host-compatible packages on other architectures.
The regression checks the unchanged x86_64 arguments and the ARM64 package
set, including its warning-denied Clippy boundary.

On this ARM64 macOS host, `./nagi test`, `./nagi build`, and `./nagi lint` all
passed. The CLI suite passed with 135 unit and 21 integration tests, and the
changed CLI package passed `cargo fmt --manifest-path
tools/nagi-cli/Cargo.toml -- --check`. `./nagi fmt` now matches the CI source
selection and passes without checking or reformatting the vendored Servo tree.

## QEMU timeout diagnostics — 2026-10-01

Headless and GUI/VNC QEMU marker waits now attempt one bounded `query-status`
and CPU-register request after an acceptance timeout, then append returned
data, query errors, or a skipped request to the serial log. QMP response lines
are capped at 64 KiB and the combined query budget is three seconds. Four CLI tests cover the
queries, serial-log append, and line bound. The focused suite passed 138 unit
tests and 21 integration tests; `./nagi test`, `./nagi lint`, `./nagi build`,
and `./nagi fmt` passed. A fresh M28 one-repetition QEMU run also passed M19,
M22, and M27 after a prior M27 Recovery timeout. That timeout remains
unexplained; the successful rerun did not exercise the diagnostic-on-timeout
path.

## Remaining M29 work

1. Capture additional genuine QEMU screenshots as more preview UI surfaces
   are implemented; the current tracked images cover M10 desktop and M18
   Albert browser acceptance.
2. Complete a first-run flow, end-user provider/error presentation, Settings,
   accessibility, and localization work in their owning milestones.
3. Measure clean-install boot and broader clean-checkout/release reproducibility;
   the current three-run sample covers persistent-data QEMU boots on one host.
4. Resolve project and third-party redistribution notices before a binary
   Developer Preview is distributed.

These open items keep M29 `PARTIAL` and are not release-ready claims.

## M18 QEMU browser screenshot — 2026-10-01

`./nagi m18` passed after the M18 target build and real QEMU boot. The guest
verified TLS, presented browser chrome, and rendered pages for `example.com`,
`example.org`, and `example.net`, then reported
`Nagi M18 browser scenario complete pages=3`. QMP captured the accepted
1280×800 display as `docs/assets/screenshots/nagi-m18-qemu-browser.png`
(SHA-256
`ede8a7967a1634c393aa53252749af22d8d98aa91e4d4711402de0e860c7e097`). The
original is preserved at
`out/evidence/m29-browser-1790798334374076000/nagi-m18-browser.png`; the
serial log is `out/logs/m18-albert.log`. This QEMU build emitted a host
audio-backend diagnostic (`virtio-sound.in` unavailable), but browser
acceptance exited 0. Host audio playback is not covered by this run. This
evidence adds a browser acceptance surface; M29 remains `PARTIAL`.
