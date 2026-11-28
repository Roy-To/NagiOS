# Nagi OS 0.1 Developer Preview — scope B (draft)

> Public documentation draft. This is not a release acceptance record or a
> claim that a downloadable image is ready. The release artifact name and
> qcow2 filename remain undecided.

## Scope

The confirmed 0.1 public scope is **B**: the Nagi desktop and the first-party
apps Notes, Files, Settings, and Calendar. Nagi is a Nagi-owned userland running
on the Linux kernel; it is not based on a general-purpose Linux distribution.
Choosing this scope does not mean every app behavior or release acceptance has
passed. See the [roadmap](../../ROADMAP.md) and
[DEC-123](../product-decisions/DEC-123-kernel-unification-and-linux-0.1.md).

## Booting the current development image

The repository snapshot used for this draft does not contain a fixed release
qcow2, `SHA256SUMS`, release source tag, or release build manifest. The steps
below boot the prepared **development** image; they do not install or boot a
released scope B package.

On a host with the dependencies reported by `./nagi doctor`, prepare a Task24
run directory containing its `preflight.json` and the verified boot inputs,
then run these commands from the repository root:

```sh
./nagi doctor
python3 tools/linux-boot/make_dev_bundle.py path/to/prepared-task24-run
./nagi run
```

The bundle is written to `out/linux-dev/` by default. `./nagi run` launches the
QEMU development image and reuses the bundle's writable Data image. The host
QEMU requirement and candidate OVMF paths are listed in
[`nagi.toml`](../../nagi.toml). The project reference machine is x86-64, q35,
UEFI, four vCPUs, and 8 GiB RAM; the exact release boot command and firmware
packaging must be fixed with the release manifest. Host QEMU/OVMF requirements
do not mean those tools are included in the guest image.

The release image, its download location, checksum, source revision, build
manifest, and final boot procedure will be documented when those artifacts are
fixed. No artifact filename is chosen here. For the current development flow,
see the [developer guide](../developer-preview/README.md).

## Known limitations

- Automatic idle lock is not implemented in 0.1.
- The 0.1 target is the QEMU x86-64 reference machine. Physical-PC support and
  compatibility with other virtual machines are not claimed.
- The app names define the selected scope, not a promise that every planned
  workflow or integration is complete. Check the release-specific acceptance
  record before relying on a behavior.
- Albert/Servo browsing, local AI, voice, semantic search, and A/B boot and
  recovery are outside this 0.1 scope.
- Nagi is experimental software and does not provide production security
  guarantees. Read the [security policy](../../SECURITY.md).
- The final qcow2 and its image-specific dependency, kernel, and firmware
  notices are not available in this draft. See
  [`THIRD_PARTY_NOTICES.md`](../../THIRD_PARTY_NOTICES.md) and
  [`LICENSE_SCOPE.md`](../../LICENSE_SCOPE.md); neither source visibility nor
  this draft establishes image redistribution clearance.

## Reporting a problem

For a reproducible product or tooling issue, use the
[GitHub issue form](https://github.com/Roy-To/NagiOS/issues/new/choose). Include
the source revision, host/QEMU setup, the smallest reproduction, and sanitized
logs. Remove credentials, tokens, personal information, user-name-containing
local paths, and chat identifiers before posting.

For a suspected security vulnerability, do **not** open a public issue. Use
GitHub's private vulnerability reporting from the repository's **Security**
tab, under **Advisories → Report a vulnerability**, as described in
[`SECURITY.md`](../../SECURITY.md).

## Licensing

The Nagi source license applies only to the source scope in
[`LICENSE_SCOPE.md`](../../LICENSE_SCOPE.md). Third-party components and assets
retain their own terms. The current notice file is a source-preparation record,
not a complete bill of materials or permission to redistribute a compiled
image.
