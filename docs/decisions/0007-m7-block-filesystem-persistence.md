# M7 Block, Filesystem, and Persistence Decision

Status: accepted for Nagi OS 0.1 M7

M7 keeps filesystem policy in user space. The kernel supplies only PCI/legacy
VirtIO Block transport and a capability-checked fixed-sector read/write ABI;
VFS, ext2, directory operations, file handles, and the bounded file-backed
mapping API live in `libnagi`.

The official acceptance uses a separate raw VirtIO data disk. The guest
formats, creates, writes, and maps a file on the first QEMU boot, then mounts
and reads the same file on a second QEMU boot without host-side filesystem
access. This keeps persistence real while preserving the existing FAT12 ESP
boot path.
