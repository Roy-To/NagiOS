# ADR-0013: M30 Reference Disk Layout and User Data Capability

**Status:** Accepted for the M30 reference-image implementation

**Date:** 2026-09-30

## Context

The implementation specification requires a self-contained 64 GiB GPT image
with an EFI System Partition (ESP), System A, System B, User Data, Recovery,
and Model Store. The current kernel gives user processes a capability to the
whole largest writable block device. Passing a GPT disk to the existing VFS
would let its initial format writes overwrite the primary GPT entry array.
The loader currently reads payloads only from the UEFI filesystem that
contains the loader.

## Decision

The reference image uses 512-byte logical sectors and 1 MiB-aligned
partitions, in this order:

| Partition | Size | Filesystem | Type GUID | Unique partition GUID |
| --- | ---: | --- | --- | --- |
| ESP | 512 MiB | FAT32 | EFI System Partition GUID `c12a7328-f81f-11d2-ba4b-00a0c93ec93b` | `4e414702-0001-4e41-4749-000000000000` |
| System A | 4 GiB | FAT32 | `4e414701-0001-4e41-4749-000000000001` | `4e414702-0001-4e41-4749-000000000001` |
| System B | 4 GiB | FAT32 | `4e414701-0001-4e41-4749-000000000002` | `4e414702-0001-4e41-4749-000000000002` |
| User Data | 16 GiB | Nagi's ext2-like VFS | `4e414701-0001-4e41-4749-000000000003` | `4e414702-0001-4e41-4749-000000000003` |
| Recovery | 4 GiB | FAT32 | `4e414701-0001-4e41-4749-000000000004` | `4e414702-0001-4e41-4749-000000000004` |
| Model Store | 32 GiB | FAT32 | `4e414701-0001-4e41-4749-000000000005` | `4e414702-0001-4e41-4749-000000000005` |

The remaining 3.5 GiB stays unallocated for future growth. Fixed, documented
unique partition GUIDs identify the reference-image partitions; the disk GUID
is also deterministic so the generated image and checksums are reproducible.
Only the ESP has the EFI System Partition type. Reserved Nagi partitions use
Nagi-owned type GUIDs and human-readable GPT names.

The ESP contains the fallback loader at `EFI/BOOT/BOOTX64.EFI`. System A,
System B, and Recovery each contain their own `KERNEL.ELF` and `INIT.ELF` at
the root of their FAT32 volume. The loader identifies those volumes by their
GPT unique partition GUIDs through UEFI `PartitionInfo` and `SimpleFileSystem`
protocols. Discovery uses the pinned `uefi 0.37` `locate_handle_buffer` API;
if a requested unique GUID is absent or appears on more than one attached
volume, boot fails closed. The existing directory-based FAT acceptance images
remain a separate compatibility path while the GPT release path is introduced.

The kernel validates both GPT headers and entry-array checksums, checks the
User Data extent against usable LBAs and all other non-empty entries, and then
creates the user block capability for only the first 16,384 sectors (8 MiB) of
that extent, matching the current fixed VFS geometry. Block syscalls take User
Data-relative sector numbers and reject out-of-range requests before adding
the validated starting LBA. Invalid or missing GPT metadata yields no user
block capability; there is no raw-disk fallback. VFS formatting and
normal storage syscalls therefore cannot address the MBR, either GPT header,
the entry arrays, ESP, system slots, Recovery, or Model Store.

The existing custom VFS uses a fixed 8 MiB on-disk geometry. The 16 GiB User
Data partition reserves room for a later growable filesystem, while the initial
block capability itself is limited to the first 8 MiB. The current Model Store
FAT32 layout has a just-under-4-GiB maximum for a single file; until that limit
is replaced, release preflight must reject model artifacts at or above 4 GiB.

## Consequences

- M30 builds the deterministic partition table without a host partitioning
  utility. The reference qcow2 has booted System A under QEMU, and the bounded
  User Data VFS has passed persistence checks across restart.
- The kernel, not user space, derives and bounds the data capability.
- `./nagi m30` runs its QEMU format/write/restart acceptance against a
  byte-identical qcow2 copy in the run evidence directory. The built reference
  image remains a clean release input instead of inheriting acceptance data.
- M-stage acceptance disks must be converted to GPT images with a User Data
  partition before the kernel can remove its whole-disk fallback.
- Model Store and the system partitions have release-image placement, but
  package management, authenticated updates, Model Store installation, and
  System B/Recovery acceptance from this GPT image remain separate work.
- The release tool's clean-tree preflight, assembly, and integrity checks pass.
  Two QEMU boots of a byte-identical disposable copy passed GPT System A and
  M7 persistence acceptance; the untouched assembled package retained its
  SHA-256 and passed `release.py verify` and `qemu-img check` afterward. Direct
  read-only boot is not supported by the current M7 initialization path,
  because the kernel excludes read-only devices when selecting writable
  storage. M27 GPT acceptance now covers System B, Recovery, rollback, and
  readiness-based promotion using the same partition layout. M30 remains
  `PARTIAL` until authenticated update acceptance, the remaining M18–M29
  gates, and license/notice review pass.
