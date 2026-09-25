# ADR 0024: Keep the M17 boot ESP out of writable user storage

Status: accepted for the M17 Servo bootstrap continuation on 2026-09-25

## Context

CI run `36092134517` (#182, head
`846cb5dc80adcbad01eb5dbd94d127419814639d`) passed the real Servo init link
and UEFI loader build, then failed on the final QEMU boot while reading
`INIT.ELF` at file offset `0xa00000`. The EFI filesystem returned
`VOLUME_CORRUPTED` after successfully opening, sizing, allocating, and rewinding
the file.

M17's two-boot acceptance first runs the real M7 persistence test, then boots
again for Servo. The kernel gave user init a capability to the largest VirtIO
block device. In M17, that was the 261,415-sector (about 128 MiB) FAT12 boot
ESP; the dedicated writable user-data disk is only 32,768 sectors (16 MiB).
The ext2 VFS formats an unrecognized disk by writing its superblock at 1 KiB
block 1, physical LBA 2–3. The ESP's first FAT begins at LBA 1. For the
generated INIT chain starting at cluster 12, FAT12 entry 341 lies at byte
offsets 1023–1024 in the image. The superblock write overwrites byte 1024,
corrupting that link. Cluster 341 is reached at file offset `0xa48000`, inside
the failing 1 MiB request beginning at `0xa00000`.

## Decision

- Attach the M17 boot ESP as read-only in both QEMU boots.
- During writable user-storage discovery, ignore VirtIO block devices offering
  `VIRTIO_BLK_F_RO` (feature bit 5), then select the largest remaining writable
  device. The smaller persistent data disk is therefore the user-storage
  capability target.
- Keep the existing QEMU disk mode for other milestones unchanged.

This keeps guest storage behind the existing kernel capability boundary and
prevents user-space filesystem initialization from modifying the boot image.
It does not change the FAT12 image format or the M17 pixel acceptance criteria.

## Verification

- The kernel regression selects the writable user-data disk when a larger
  read-only boot disk is also present.
- The CLI regression checks that both M17 QEMU boots use a read-only boot image
  and that other image-drive configurations retain their existing mode.
- `cargo +nightly-2025-08-01 build -p nagi-kernel --target
  targets/x86_64-unknown-nagi.json -Zbuild-std=core,compiler_builtins
  --release --locked` passed locally.
- Local QEMU 11.1/OVMF read a full-sized synthetic INIT (127,747,048 bytes)
  from the read-only M17 image and printed `Nagi Kernel started`; the image
  SHA-256 was unchanged across that boot.
- Public target CI must still verify the real two-boot persistence gate and
  produce the nonzero Servo pixel checksum. M17 remains `BLOCKED`; M18 remains
  `NOT STARTED`.
