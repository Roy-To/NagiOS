# ADR 0056: Multi-block regular files in the Nagi VFS

Status: accepted
Date: 2026-10-05
Milestones: M7 (storage foundation); unblocks M18 downloads and larger
user documents

## Context

The ext2-like User Data VFS stored every regular file in a single 1 KiB
block (`MAX_FILE_SIZE = BLOCK_SIZE`), using only `i_block[0]`. The POSIX layer
could only replace a whole file at offset 0 and computed file sizes by reading
the file into a 1 KiB stack buffer. Browser downloads, uploaded documents,
and ordinary sequential writes (`std::fs::write`, which writes in chunks)
could not be stored. Several subsystems (Recovery, Desktop settings, M19
fixtures, M22 history) also used `MAX_FILE_SIZE` as the size of on-stack
record buffers.

## Decision

- Regular files use the standard ext2 pointer layout: `i_block[0..12]` direct
  blocks and `i_block[12]` as a single-indirect block (256 pointers), for a
  maximum of 268 KiB per file. Double and triple indirect blocks remain
  unused. Directories keep exactly one block.
- The on-disk format stays compatible: existing single-block files have zero
  extra pointers and `i_blocks = 2`, which is exactly what the new code writes
  for them. No migration is needed.
- New `Vfs::read_at` and `Vfs::write_at` provide ranged I/O; `write`,
  `truncate`, `remove`, `replace`, and path removal grow or release every data
  block (and the indirect block) and keep free counts exact. Bytes past the
  end of a file in allocated blocks are kept zero, so growth never exposes
  stale data. A failed growth releases every block it allocated and leaves
  the inode unchanged.
- The read-only integrity check (`check_existing`) validates every direct and
  indirect pointer: in range, allocated in the bitmap, owned by exactly one
  inode, no pointers past the file's block count, and `i_blocks` consistent.
- `MAX_SMALL_FILE_SIZE` (1 KiB) names the old bound. Whole-file `mmap`
  mappings and subsystems that keep a record in one stack buffer use it, so
  their behavior and stack use are unchanged.
- `nagi-posix` reads and writes at the descriptor offset through the ranged
  API and takes file sizes from metadata.

## Consequences

- Files up to 268 KiB can be created, appended, read, and truncated through
  POSIX. The 8 MiB volume and 64-inode limits are unchanged.
- Writes remain in place without journaling, as before; crash consistency is
  no stronger than the existing single-block behavior.
- Subsystems that need records larger than 1 KiB must stop using whole-file
  stack buffers before raising their own bounds.
