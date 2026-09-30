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
| Screenshots and performance | No Nagi screenshots are tracked; the only tracked raster images found are third-party documentation assets. | No screenshots or boot-time benchmark were added. Capture authentic QEMU UI and measure boot time when a stable preview surface is available. |
| Clean build | CI checks build from fresh checkouts. This local documentation checkpoint did not run `./nagi clean`, which removes `target/` and `out/` including preserved acceptance logs and persistent disks. | Existing evidence remains preserved. A clean release build and artifact reproducibility are part of the M30 release gate. |

## Verification

- With the rustup shims in `~/.cargo/bin` before the Homebrew toolchain,
  `./nagi --help` passed on the current macOS host and printed the actual
  supported command list.
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
- `cargo test --locked --offline -p nagi-cli --all-targets` — passed (134 unit
  tests, 19 integration tests); `cargo clippy --locked --offline -p nagi-cli
  --all-targets -- -D warnings` — passed.
- `rustfmt --check` for the changed Rust files and `git diff --check` — passed.
- No QEMU acceptance was run specifically for the M29 documentation and host
  doctor changes. Existing M16/M17/M18 and M27 acceptance evidence is recorded
  in the current implementation status and relevant workstreams.

## Remaining M29 work

1. Add genuine QEMU screenshots when the preview UI is stable and the capture
   can be tied to a specific guest build.
2. Complete a first-run flow, end-user provider/error presentation, Settings,
   accessibility, and localization work in their owning milestones.
3. Obtain boot-time and broader clean-checkout/release reproducibility evidence.
4. Resolve project and third-party redistribution notices before a binary
   Developer Preview is distributed.

These open items keep M29 `PARTIAL` and are not release-ready claims.
