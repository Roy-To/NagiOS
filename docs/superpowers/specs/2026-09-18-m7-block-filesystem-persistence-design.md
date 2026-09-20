# M7 Block, Filesystem, and Persistent Storage Design

## Goal

Extend the M6 bootstrap with a real VirtIO Block path and a user-space VFS/
ext2 slice that survives a QEMU shutdown and reboot. The M7 acceptance proof
is a guest-created file whose bytes are written on the first boot and read
back from the same persistent data disk on the second boot.

## Architecture

The kernel owns only the low-level PCI/legacy VirtIO Block transport and a
bounded sector ABI. It does not interpret paths, directories, inodes, or file
handles. `libnagi` owns the user-space `BlockDevice`, VFS, and ext2 logic.
This preserves the kernel rule that high-level filesystem behavior remains in
user space and avoids adding file-oriented kernel syscalls.

The block ABI uses a capability minted from the discovered VirtIO device and
passed only to the bootstrap user process in its entry register. Read/write
requests are fixed at one 512-byte sector, validate the current user process's
writable mapping, and use a kernel bounce buffer before reaching the device.
The kernel never accepts a path or an arbitrary physical address.

M7 targets the QEMU reference machine and uses an explicit legacy
`virtio-blk-pci,disable-modern=on` data device. PCI discovery selects the
largest VirtIO Block capacity, which is the separate persistent user-data
disk; the existing FAT12 ESP remains the immutable boot medium.

## User-space filesystem slice

`libnagi::storage` provides:

- a fixed-sector `BlockDevice` trait and `SyscallBlockDevice` adapter;
- a 1 KiB ext2 volume formatter/mounter with bounded metadata validation;
- root-directory create/open/list operations with bounded names;
- generation-checked `FileHandle` values;
- bounded single-block file read/write operations;
- `mmap`/`flush`/`munmap` for a bounded file-backed mapping backed by the
  existing file handle and user-owned page. This is the M7 mapping contract;
  demand-paged kernel VMO integration remains a later extension of the
  existing VMO subsystem.

The formatter uses a small valid ext2 layout (one 1 KiB block group, a 64
inode table, bitmaps, root directory, and bounded direct data blocks). It is
not a host filesystem implementation and all metadata/data I/O goes through
the guest VirtIO Block adapter.

## Reboot acceptance flow

The CLI creates the persistent raw data image only when it does not exist and
never overwrites an existing image during `nagi run`. It then orchestrates two
real QEMU boots using the same image:

1. Boot one formats the blank ext2 volume, creates a root file, writes the
   fixed acceptance payload, verifies a file-backed mapping, and exits with a
   dedicated reboot-required status.
2. Boot two mounts the existing ext2 volume, opens the same directory entry,
   reads and verifies the payload, verifies the mapping again, and exits
   successfully.

The first and second serial logs are retained separately. The terminal M7
acceptance marker is emitted only by the successful second process exit after
the real read-back. The host orchestrator starts/stops QEMU and does not read,
write, or interpret the guest filesystem contents.

## Error handling and security

Invalid PCI identity, missing legacy BAR, unsupported queue size, failed
VirtIO status, out-of-range sectors, malformed ext2 metadata, duplicate
directory names, invalid handles, and short buffers fail closed. A failed
first boot never causes a second boot to be reported as acceptance. Existing
M5/M6 console, syscall, FPU, and service-registry checks remain required in
the final boot log.

## Verification

- Host unit tests cover ext2 formatting, mount validation, directory entries,
  handles, write/read round trips, mapping flush, and malformed metadata.
- Kernel tests cover PCI address encoding, VirtIO descriptor/request layout,
  capability checks, and user writable-range checks.
- User-target, kernel, and UEFI builds remain required.
- M7 PowerShell and Git Bash acceptance scripts require both ordered boot logs
  and the final persistent read-back marker.
