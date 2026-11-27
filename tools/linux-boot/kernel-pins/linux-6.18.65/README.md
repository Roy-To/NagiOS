# Linux 6.18.65 x86_64 development pin

This directory fixes the Linux input for the L11-4 x86_64 development profile.
DEC-077 sets LTS as the product-kernel standard while leaving the exact version
provisional. On 2026-11-25, kernel.org listed 6.18.65 as the current longterm
release; the 6.18 series had a projected end of support in December 2028. This
pin records that dated selection and does not make a Linux-image acceptance or
release claim.

`x86_64.config` is the complete Linux 6.18.65 Kconfig result. It starts from
that release's `arch/x86/configs/x86_64_defconfig` and applies
`nagi-x86_64.config.fragment`. The profile includes the current x86_64
q35/OVMF boot and display path, built-in VirtIO block and ext4, dm-verity for
L11 System integrity, and Landlock plus namespaces for the L11 app-isolation
profile. The manifest records source and config hashes, the empty downstream
patch set, and the retrieval and regeneration inputs.

The 155 MB upstream source archive is not copied into this repository. To
retrieve it and check the bytes:

```sh
curl -fLO https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.18.65.tar.xz
curl -fLO https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.18.65.tar.sign
shasum -a 256 linux-6.18.65.tar.xz
xz -cd linux-6.18.65.tar.xz | gpg --verify linux-6.18.65.tar.sign -
```

The expected archive SHA-256 and the upstream signature key fingerprint are in
`manifest.json`. Obtain the release key through the kernel.org Web Key
Directory and confirm the full fingerprint before treating the detached
signature as verified. The signature was not locally verified when this
manifest was generated because GnuPG is unavailable in that host environment.

To regenerate the config with GNU Make 4.0 or newer:

```sh
tar -xf linux-6.18.65.tar.xz
cd linux-6.18.65
make ARCH=x86 x86_64_defconfig
scripts/kconfig/merge_config.sh -m .config /path/to/nagi-x86_64.config.fragment
make ARCH=x86 olddefconfig
make ARCH=x86 kernelrelease
cmp .config /path/to/x86_64.config
```

The final command before `cmp` must report `6.18.65`. This is a source and
configuration pin only; the kernel was not built and no QEMU result is claimed.
