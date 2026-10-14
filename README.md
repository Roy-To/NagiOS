# Nagi OS

**NAGI — Native Agentic General Interface**

[English](README.md) · [日本語](README.ja.md)

> **Experimental Developer Preview.** Nagi is an independent operating system
> under active development. The 0.1 target is the QEMU x86-64 reference machine;
> this repository does not claim a finished, general-purpose OS or physical-PC
> support. Nagi's project-wide license has not been selected or granted.

## Project Overview

Nagi is a from-scratch operating system project with its own kernel and
user-space stack. Linux, Windows, and macOS are development hosts only; Nagi is
not a Linux distribution. The current implementation line is Nagi OS 0.1
Developer Preview, with QEMU x86-64 as its official target.

The project explores a local-first desktop where normal keyboard, pointer, and
GUI use remains central while AI can help with user-directed work. The AI
design keeps models in user space and treats their output as untrusted. The
design requires proposed state changes to pass deterministic validation,
policy, capability, and executor boundaries; end-to-end integration remains
under development.

This summary was checked against source commit bcb92dc (2026-10-12). The
milestone record in [implementation status](docs/implementation_status.md) is
the source of truth for acceptance results and known limits.

## Vision

Nagi is guided by:

> **Safe by default. Powerful by choice.**
>
> **Restrict software, not the owner.**

The long-term vision is an AI-native operating system that understands user
intent while keeping authority with the person who owns the machine. AI should
be useful without becoming a privileged kernel component or gaining authority
from its confidence.

Local-first and offline-first are core design goals: basic operation should not
depend on a cloud account, cloud LLM, API key, telemetry service, or remote
storage. This is a product direction; it does not mean every current preview
feature or provider has completed offline acceptance.

English and Japanese are equal first-class user-language commitments. The
planned Nagi OS 0.3 Personal Agent builds on these boundaries with user-owned,
model-independent profile and memory concepts. That agent experience is a
future architecture, not a completed 0.1 feature.

## Why Nagi OS?

- **An operating system built as an operating system:** Nagi has its own kernel,
  ABI, services, and native desktop rather than a custom shell on Linux.
- **The user keeps authority:** capabilities and explicit permissions define
  what software can do; probabilistic AI output does not grant access.
- **Local data is the default direction:** user objects, history, and planned
  AI context are designed around local ownership and recoverability.
- **Useful without an agent:** ordinary GUI, keyboard, and pointer interaction
  remains a requirement even when AI is unavailable or disabled.
- **Designed for English and Japanese:** both are first-class user languages,
  with broader localization and accessibility work still in progress.

## Key Features

| Area | Current evidence | Scope and limits |
| --- | --- | --- |
| Independent OS and kernel | M0–M18 are recorded PASS for their stated acceptance scope. The guest boots Nagi's own kernel and user-space stack on the QEMU reference machine. | Milestone PASS results do not mean the full 0.1 Developer Preview is complete. Physical hardware is not a 0.1 target. |
| Native desktop | M10 real-QEMU acceptance is PASS. The tracked capture shows Calculator, Notes, Files, and Terminal panels. | This is a bounded desktop acceptance surface, not a finished application suite or installer. |
| Albert browser | M18 target CI acceptance is PASS: [run 37601664873](https://github.com/RT-NISH/NagiOS/actions/runs/37601664873) rendered three HTTPS sites and exercised site permission, clipboard, kana IME, upload, and user-activated download. | Production authentication, notification presentation, and Servo StorageManager persistence remain open; kanji conversion is deferred. |
| Capability-based security | Kernel handles, IPC, and capability checks have accepted implementation foundations. | Nagi makes no production security guarantee. Full app-facing and AI-action authorization integration is still in progress. |
| Local-first and local AI | A real QEMU run loaded Granite 4.2 3B in Nagi and produced a schema-constrained Japanese response. | M20 remains PARTIAL. Model installation and normal lazy-service integration remain open; this is not a finished AI chat or agent workflow. |
| Wayback and Activity Ledger | M15 guest acceptance verified persistent history, versions, restore, and undo. Later M22 work has bounded History and Activity Ledger evidence. | General production Activity Ledger and AI-action integration remain partial. |
| English and Japanese | The M10 desktop has a Japanese text path; M29 QEMU acceptance verified Japanese Settings selection and restoration after restart. | Cross-process language propagation, complete translation, and accessibility acceptance remain open. |
| Personal Agent | Nagi 0.3 architecture work describes profile, model-independent memory, learning, and capability-aware agent execution. | This is a forward roadmap, not a 0.1 implementation claim. |

## Architecture

~~~text
UEFI / OVMF
    ↓
Nagi Loader
    ↓
Nagi Kernel — execution, memory, IPC, and authority
    ↓
nagi-init / Supervisor
    ↓
User-space services · Nagi ABI · SDK / PAL / relibc
    ↓
Native applications · Albert browser (Servo)
~~~

The kernel is Nagi's own, not Linux-based. The architecture places filesystem,
networking, desktop, audio, packages, and AI policy in user space. Albert uses
Servo as its browser engine. Some preview services still run inside init, so
the diagram describes the target boundary and does not imply full production
service isolation.

For the design contracts, see the
[0.1 implementation specification](docs/Nagi_OS_0.1_Codex_Implementation_Spec.md)
and [architecture overview](docs/architecture/README.md).

## Current Development Status

Status at source commit bcb92dc (2026-10-12):

| Milestones | Status | Summary |
| --- | --- | --- |
| M0–M18 | PASS | The accepted baseline includes the kernel and desktop foundations and the full M18 Albert guest scenario. See the linked acceptance records for exact scope. |
| M19 | PARTIAL — current milestone | Bounded guest Search, Browser History persistence, and Files Search flows have evidence. Production app-facing Search grants, Files trash/restore integration, and normal Browser History producer updates remain open. |
| M20 | PARTIAL | Local Granite guest inference passed. The [hosted inference run 37702418216](https://github.com/RT-NISH/NagiOS/actions/runs/37702418216) stopped before inference on a User Data write timeout; model installation and normal service integration remain open. |
| M21–M26 | PARTIAL | Validators, provider boundaries, and bounded guest experiments exist. General production agent actions, semantic-search integration, page summary, and a microphone-to-action voice experience are not accepted. Qwen inference and Gemma use are not accepted. |
| M27–M30 | PARTIAL | Bounded recovery, integration, Japanese Settings, and release-bundle checks have evidence. Full integrated acceptance, authenticated updates, and redistribution review remain open. M30 is not PASS. |

M30's recorded release manifest still has
<code>m30_acceptance=NOT_EVALUATED</code>.
Completion of individual QEMU scenarios or artifact checksums does not make the
0.1 release ready. For run IDs, source revisions, and precise limitations,
consult [docs/implementation_status.md](docs/implementation_status.md) and the
owning workstream documents.

## Screenshots

These are existing captures from QEMU guest acceptance work, not mockups or
video stills. Each image shows only the checkpoint named in its caption.

| M10 — Native desktop | M18 — Albert / Servo |
| --- | --- |
| ![M10 QEMU desktop with Calculator, Notes, Files, and Terminal panels](docs/assets/screenshots/nagi-m10-qemu-desktop.png) | ![M18 Albert browser showing an HTTPS page inside Nagi](docs/assets/screenshots/nagi-m18-qemu-browser.png) |
| Fixed desktop acceptance panels; not a completed app suite. | Captured 2026-10-01, before M18's full interactive CI closure on 2026-10-07. The still shows a rendered page, not all browser interactions. |

### M29 — Japanese Settings

![M29 Japanese Settings selection shown in the QEMU guest](docs/assets/screenshots/nagi-m29-settings-ja-jp.png)

This image accompanies the QEMU language-selection and same-disk restart
acceptance. It does not demonstrate complete translation or accessibility.

No demo video has been recorded or published by this preparation. The
[QEMU demo recording plan](docs/oss/DEMO_RECORDING_PLAN.md) describes a
recording based on reverified guest behavior.

## Roadmap

- **Nagi OS 0.1 — Developer Preview:** establish a bootable independent OS and
  its first native desktop, browser, storage, security, and local-AI foundations.
  The 0.1 line remains in progress; M19–M30 are PARTIAL.
- **Nagi OS 0.2 — Platform Foundation:** stabilize identity, permissions, IPC,
  settings, model runtime, updates, application contracts, Search, and
  Activity/Wayback. The 0.2 specification requires 0.1 M30 PASS and an explicit
  activation checkpoint; host foundations are not target-runtime acceptance.
- **Nagi OS 0.3 — Agentic System:** a forward architecture for a Personal Agent
  with user-owned profile and memory, model portability, capability-aware
  actions, and Wayback-backed undo. These goals are not 0.1 features.
- **Nagi OS 1.0:** the long-term target for formal public OSS release. No
  release date is promised, and the project license is not yet selected.

See [ROADMAP.md](ROADMAP.md), the
[0.2 implementation specification](docs/Nagi_OS_0.2_Codex_Implementation_Spec.md),
and the [0.3 Architecture Roadmap](docs/Nagi_OS_0.3_Architecture_Roadmap.md).

## Getting Started

Nagi 0.1 targets QEMU x86-64 with UEFI/OVMF, q35, four virtual CPUs, and 8 GiB
of RAM. Use the pinned Rust toolchain and the host dependencies listed by the
repository. Physical PC support is not a 0.1 completion requirement.

From a local checkout, the POSIX host workflow is:

~~~sh
./nagi doctor
./nagi fetch
./nagi build
./nagi test
./nagi image
./nagi run
~~~

fetch may require network access and substantial disk space to materialize
pinned third-party sources. It does not install every optional model. The
build and test commands cover host-compatible packages; they do not replace
QEMU milestone acceptance. For Windows PowerShell, host setup, troubleshooting,
and current command details, follow the
[Developer Preview guide](docs/developer-preview/README.md) and run
<code>./nagi --help</code>.

The public repository can be inspected, but its visibility does not grant
permission to use or redistribute the source. See
[Licensing Status](#licensing-status) before working with or sharing it.

## Contributing

Start with [CONTRIBUTING.md](CONTRIBUTING.md),
[AGENTS.md](AGENTS.md), the current
[implementation status](docs/implementation_status.md), and the workstream
that owns the change. Keep acceptance evidence tied to its source revision and
separate verified behavior from plans and partial implementations.

Useful contributions include reproducible QEMU failures, focused fixes,
documentation improvements, and reviews that identify concrete evidence or
remaining limits. Check the licensing status below and the repository's
contribution instructions before submitting code.

## Licensing Status

Nagi's official project license has **not been selected or granted**. Public
repository visibility is not an open-source license and does not imply
permission to use, copy, modify, or redistribute Nagi-owned code or
documentation. No candidate license should be treated as a grant.

Third-party source, fonts, model weights, voice data, and images have their own
terms. Review [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and
[Legal documentation](docs/legal/README.md) before relying on or redistributing
those materials. The formal public OSS release is planned for Nagi OS 1.0,
after license and release reviews are complete.
