# Nagi OS (NAGI — Native Agentic General Interface)

Nagi OS 0.1 Developer Preview is an independent, local-first operating system
targeting the QEMU x86-64 reference machine.

## M0 developer workflow

On Windows PowerShell:

```powershell
.\nagi.ps1 fetch
.\nagi.ps1 doctor
.\nagi.ps1 build
.\nagi.ps1 test
```

On a POSIX development host:

```text
./nagi fetch
./nagi doctor
./nagi build
./nagi test
```

`fetch` materializes the pinned Servo and libc sources, applies the tracked
Nagi patches, and records their fingerprints. Run it before root-workspace
Cargo commands when the generated third-party sources are absent.

If PowerShell execution policy blocks local scripts, invoke the launcher with
the repository-scoped bypass used by the acceptance test:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\nagi.ps1 doctor
```

`doctor` reports host dependencies. Commands for guest image creation and
boot remain unavailable until their owning milestones implement them.

Read `AGENTS.md`, the primary specification, and
`docs/implementation_status.md` before changing the project.

## Project status

Nagi OS is an independent, local-first Developer Preview. The current
implementation milestone is M17, Servo Bootstrap. M17 is currently
`BLOCKED`, and M18 has not started. This repository does not claim a real
guest first-web-pixel acceptance result yet.

The reference environment is QEMU x86-64 with UEFI/OVMF, q35, four vCPUs,
8 GiB of RAM, VirtIO block/network/GPU/sound/RNG, and Nagi's software
rendering path.

## Licensing

The license for Nagi OS itself has not been finalized. Do not infer a license
for Nagi OS from any third-party component. Current third-party source pins,
license metadata, and items requiring review are recorded in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
Third-party components remain subject to their respective licenses.

## Security status

Nagi OS is experimental software and does not make production security
guarantees. Please do not put credentials, private keys, or other sensitive
details in public issues. See [`SECURITY.md`](SECURITY.md) for the current
reporting boundary.
