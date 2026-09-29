# Nagi OS (NAGI — Native Agentic General Interface)

Nagi OS 0.1 Developer Preview is an independent, local-first operating system
targeting the QEMU x86-64 reference machine.

## Developer workflow

On Windows PowerShell:

```powershell
.\nagi.ps1 fetch
.\nagi.ps1 doctor
.\nagi.ps1 build
.\nagi.ps1 test
```

On a POSIX development host:

```sh
./nagi fetch
./nagi doctor
./nagi build
./nagi test
```

`fetch` materializes and validates pinned third-party sources. It may need a
network connection and substantial free disk space. Run it before workspace
Cargo commands when those sources are absent.

If PowerShell execution policy blocks local scripts, invoke the launcher with
the repository-scoped bypass used by the acceptance test:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\nagi.ps1 doctor
```

`doctor` checks development-host dependencies. The current command list is
available with `./nagi --help` or `.\nagi.ps1 --help`. For the first guest
image and QEMU run, see the [Developer Preview guide](docs/developer-preview/README.md).

Read `AGENTS.md`, the primary specification, and
`docs/implementation_status.md` before changing the project.

## Project status and limits

Nagi OS is an independent, local-first Developer Preview. Milestone states,
acceptance evidence, and known blockers change as work proceeds; use
[`docs/implementation_status.md`](docs/implementation_status.md) as the
current status source. A command or library existing in the repository does
not by itself mean its milestone acceptance has passed.

The reference environment is QEMU x86-64 with UEFI/OVMF, q35, four vCPUs,
8 GiB of RAM, VirtIO block/network/GPU/sound/RNG, and Nagi's software
rendering path.

## Licensing

The license for Nagi OS itself has not been finalized. Do not infer a license
for Nagi OS from any third-party component. Current third-party source pins,
license metadata, and items requiring review are recorded in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
Third-party components remain subject to their respective licenses.

## More documentation

- [Developer Preview setup, QEMU acceptance, and diagnostics](docs/developer-preview/README.md)
- [Rust and C SDK surface with the sample packaging flow](sdk/README.md)
- [Contribution and verification workflow](CONTRIBUTING.md)
- [Milestone roadmap](ROADMAP.md)
- [Security reporting policy](SECURITY.md)

## Security status

Nagi OS is experimental software and does not make production security
guarantees. Please do not put credentials, private keys, or other sensitive
details in public issues. See [`SECURITY.md`](SECURITY.md) for the current
reporting boundary.
