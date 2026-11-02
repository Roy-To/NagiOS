# Nagi OS

**NAGI — Native Agentic General Interface**

[English](README.md) · [日本語](README.ja.md)

Nagi is an independent, local-first operating system being built around explicit
capabilities, a native desktop, and AI that operates within the user's authority.
It has its own kernel and user-space stack; Linux, Windows, and macOS are
development hosts. The 0.1 Developer Preview targets QEMU x86-64.

**This is an experimental Developer Preview, not a finished general-purpose OS.**
The maintainer's original Nagi OS source within the documented scope is licensed
under **MPL-2.0**. Existing package licenses and third-party/model terms remain
separate. Some glyph/media provenance and complete-image redistribution checks
remain unresolved. See [Licensing](#licensing) and [LICENSE_SCOPE.md](LICENSE_SCOPE.md).

## Why Nagi?

**Safe by default. Powerful by choice. Restrict software, not the owner.**

Nagi explores an interface where people can express intent while retaining
control of their machine. Ordinary GUI, keyboard, and mouse use is a design
requirement even when AI is disabled. Core operation is intended to work without
a cloud account, cloud LLM, API key, telemetry, or remote storage.

AI is a user-space, untrusted provider. Its proposed actions must pass
deterministic validation, policy, permissions, and an executor before changing
state. Confidence never grants authority. Local inference, semantic objects,
history, and undo are parts of this design; their end-to-end integration is
still in progress.

English and Japanese are equal first-class user-language commitments. English
is canonical for APIs and diagnostics. Complete localization and accessibility
acceptance remain open.

## Preview in QEMU

These are existing guest captures from acceptance runs, not product mockups.
They show the implementation at each capture checkpoint, rather than every
feature described in the roadmap.

![Albert browser running inside Nagi on QEMU](docs/assets/screenshots/nagi-m18-qemu-browser.png)

*Albert / Servo HTTPS rendering acceptance, captured on 2026-10-01.
This image predates the full interactive M18 CI closure on 2026-10-07.*

| Native desktop acceptance surface | Japanese Settings acceptance |
| --- | --- |
| ![Nagi M10 desktop in QEMU](docs/assets/screenshots/nagi-m10-qemu-desktop.png) | ![Nagi Japanese Settings in QEMU](docs/assets/screenshots/nagi-m29-settings-ja-jp.png) |
| Fixed Calculator, Notes, Files, and Terminal panels; not a completed app suite. | Language selection / restart evidence; not proof of complete translation. |

A demo video has not been published as part of this preparation.
The [recording plan](docs/oss/demo-recording-plan.md) ties proposed shots to
existing guest acceptance evidence and defines what must be reverified.

## Architecture and technical direction

```text
UEFI / OVMF → Nagi Loader → Nagi Kernel → nagi-init / Supervisor
                                            │
                       user-space services and Nagi SDK / PAL
                                            │
                              native apps and Albert (Servo)
```

This is the architectural direction. Some preview services still run inside
init; the diagram does not imply that all services have production isolation.

- **Capability-based hybrid kernel:** execution, memory, IPC, and authority stay
  in the kernel; filesystem, networking, desktop, audio, packages, and AI policy
  belong in user space.
- **Native stack:** Rust, ELF64, the Nagi ABI, versioned service interfaces,
  Rust/C SDKs, and a Nagi PAL / relibc compatibility boundary. POSIX is a
  compatibility layer rather than a Linux kernel contract.
- **Real browser port:** Albert embeds Servo on Nagi's Mesa Softpipe software
  rendering path. Guest work is not delegated to a host browser.
- **Local AI direction:** typed provider capabilities separate deterministic,
  decision, and generative paths. The 0.1 generative runtime is llama.cpp / GGUF;
  IBM Granite 4.2 3B is the selected Standard default.
- **Objects and recoverability:** stable Object IDs and Workspaces separate user
  objects from paths/processes; transactions, history, undo, and A/B recovery
  have bounded acceptance implementations, with broader integration pending.

Read the [architecture overview](docs/architecture/README.md) and
[implementation specification](docs/Nagi_OS_0.1_Codex_Implementation_Spec.md)
for the contracts and intended scope.

## Development status

Snapshot: **2026-10-11**, documentation based on main
`379d19eadb6f8792c27b9bc3f147e1ea48eac4a2`.
[Implementation status](docs/implementation_status.md) is the authoritative
record; individual acceptance results do not complete a whole milestone.

| Area | Evidence available | Remaining limits |
| --- | --- | --- |
| Boot, kernel, storage, display/input, networking, ABI, SDK | M0–M17 are recorded PASS, including real QEMU acceptance. | Bounded preview implementations and documented regression/environment limits apply. |
| Albert browser (M18) | PASS: [target CI run 37601664873](https://github.com/Roy-To/NagiOS/actions/runs/37601664873) rendered three HTTPS sites and exercised permission, clipboard, kana IME, upload, and user-activated download. | Production authentication, notification presentation, StorageManager persistence; kanji conversion deferred. Browser services remain in-process. |
| Search / semantic layer (M19) | PARTIAL: persistent Object IDs, browser-history/Search restart flows, and bounded signed-in Files indexing have guest evidence. | Production app-facing Search with live grants, Files mutation/trash UI, and normal history producer lifecycle remain open. |
| Local generative AI (M20) | PARTIAL: recorded local QEMU Granite inference produced a schema-constrained Japanese response. | Hosted inference CI failed before inference on a storage write; normal model installation and lazy-service integration remain open. |
| Planner, undo, browser context, embedding, voice, model routing (M21–M26) | PARTIAL: validators, bounded guest fixtures, and individual provider experiments exist. | No completed general agent workflow, page-summary UI, production semantic-search pipeline, or microphone-to-action voice experience. Qwen inference and Gemma use are not accepted. |
| Recovery, stress, polish, release (M27–M30) | PARTIAL: bounded A/B/recovery, restart, language, and release-bundle checks have evidence. | Full integrated acceptance, security/localization/accessibility review, and human redistribution review remain open. |

Physical hardware, mobile/multi-device execution, cloud synchronization, and a
finished installer are future directions, not accepted 0.1 functionality.
Nagi makes no production security guarantees; see [SECURITY.md](SECURITY.md).

## Build and explore

Use a supported development host with the pinned Rust toolchains, QEMU,
OVMF, and build dependencies. [The Developer Preview guide](docs/developer-preview/README.md)
covers setup and diagnostics; consult the current status record where older
guide descriptions differ.

POSIX host:

```sh
./nagi fetch
./nagi doctor
./nagi build
./nagi test
./nagi image
./nagi run
```

Windows PowerShell:

```powershell
.\nagi.ps1 fetch
.\nagi.ps1 doctor
.\nagi.ps1 build
.\nagi.ps1 test
.\nagi.ps1 image
.\nagi.ps1 run
```

`fetch` materializes hash/revision-pinned third-party sources and fonts. It may
need substantial disk space and network access. It does not establish that all
models are installed. `build` and `test` cover host-compatible packages;
`image` builds the default guest image, and `run` boots it. Feature acceptance
images are separate: `./nagi --help` lists commands such as `m18` and `login`.

Reference VM: QEMU x86-64, UEFI/OVMF, q35, 4 vCPU, 8 GiB RAM,
VirtIO Block/Network/GPU/Sound/RNG, software rendering.
Physical PC support is not a 0.1 completion requirement.

If PowerShell blocks scripts, the repository's acceptance workflow uses:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\nagi.ps1 doctor
```

## Contributing and project preparation

Read [AGENTS.md](AGENTS.md), the specification, and current status before changing
implementation. [CONTRIBUTING.md](CONTRIBUTING.md) explains verification;
the [SDK guide](sdk/README.md) shows sample packaging. Useful contributions include
reproducible guest failures, narrowly scoped ports, documentation corrections,
and evidence-backed reviews.

This documentation prepares Nagi for OSS publication and a Codex for Open Source
application. The [application preparation notes](docs/oss/codex-application-preparation.md)
describe the project, proposed use of support, and outstanding maintainer checks.
They are preparation material, not an application submission or acceptance claim.

## Licensing

The maintainer's original Nagi OS source identified in
[LICENSE_SCOPE.md](LICENSE_SCOPE.md) is licensed under **[MPL-2.0](LICENSE)**.
This is a scoped source grant, not a claim that every repository file, image,
model or dependency is covered by MPL or cleared for redistribution.

The existing `MIT OR Apache-2.0` declarations in `tools/nagi-cli`, `tools/legal`,
`crates/nagi-embedding-provider` and `crates/nagi-tts-provider` are preserved.
Third-party software and model artifacts retain their own terms. Granite and
Qwen artifacts are recorded as Apache-2.0; Gemma has separate Google terms.
No model weights are redistributed by this licensing change.

Known holds include `user/nagi-init/src/font.rs` (embedded glyph provenance),
media and artifact-specific notices. They are not silently included in the new
grant. The [scope review](docs/oss/mpl-2.0-scope-review.md) describes the reviewed
evidence and its limits. Complete-image/source-bundle redistribution review and
Nagi 0.1 release acceptance remain separate work.

[Third-party notices](THIRD_PARTY_NOTICES.md) and the
[historical Apache-2.0 audit](docs/oss/apache-2.0-readiness.md) retain their
dependency findings. Their old project-license proposal is superseded by
LICENSE_SCOPE.md, not their unresolved provenance and notice requirements.

More: [Roadmap](ROADMAP.md) · [Release notes](RELEASE_NOTES.md) ·
[Dependency/SBOM tooling](docs/legal/README.md)
