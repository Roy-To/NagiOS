# Nagi OS 0.1 Developer Preview — Release Notes

This is a release-preparation draft for the QEMU x86-64 Developer Preview. It
is not a release announcement or a claim that Nagi OS 0.1 is release-ready.

## Read the acceptance state

The accepted scope and known limitations belong to the implementation status
and milestone workstreams for the exact source revision being reviewed. Do not
infer that a feature is complete from a command, package, or library existing
in the source tree. Integrity checks in `release-manifest.json` and
`SHA256SUMS` establish file consistency only; they do not establish guest
acceptance.

## Reference target

The intended reference machine is QEMU x86-64 with UEFI/OVMF, q35, four vCPUs,
8 GiB RAM, and the configured 64 GiB disk. The expected image name is
`Nagi-OS-0.1-devpreview.qcow2`. The image, guest boot, and M30 acceptance must
be independently verified before a binary release is announced.

## Distribution status

The Nagi OS project license has not been selected. The third-party notice
inventory identifies upstream license, notice, patched-source, and asset
provenance reviews that remain open. Do not redistribute a binary or bundled
third-party source until the applicable review and release acceptance are
complete.
