# ADR 0057: VFS inode generations for file identity

Status: accepted
Date: 2026-10-05
Milestones: M7 (storage), M19 (Semantic Layer / Search)

## Context

Every VFS `FileHandle` carried generation 1, and Search keyed a file's stable
`ObjectId` on its inode number alone. Deleting a file frees its inode slot,
and the next file created takes the lowest free slot, so a new, unrelated file
could inherit the deleted file's `ObjectId`, and a handle kept for the deleted
file would silently read and write the new one. M19's workstream listed
"identity across delete/recreate and inode reuse" as a remaining blocker.

## Decision

- Use the ext2 `i_generation` field (inode offset 100). Creating a file or
  directory in a slot assigns a generation greater than any previously issued
  for that slot: `max(previous, 1) + 1`. Clearing an inode keeps its
  generation so the next occupant always gets a new one.
- Inodes written before this change store 0, which reads as generation 1 —
  the value their existing handles carry — so old volumes need no migration.
- `FileHandle` stores a 32-bit generation and validates only against the
  inode's current generation; a handle to a deleted file is rejected as
  `InvalidHandle` even after its slot is reused. `open`, `open_path`,
  `rename`, and `replace` issue handles with the inode's generation.
  `FileMetadata` exposes it.
- M19's Files indexing records `nagi.files.vfs_generation` beside the inode
  number and matches identity on the pair (records without it read as
  generation 1, so existing records keep their `ObjectId`). When a record
  names the same inode with a different generation, the file it described was
  deleted, so the record is removed from the index; the new file gets a fresh
  `ObjectId`.

## Consequences

- Stale handles and stale Search records can no longer alias a reused inode.
- `./nagi m19` deletes and recreates a scratch file, requires its slot to be
  reused with a higher generation, a new `ObjectId`, and the old record gone.
- Search still learns about files when its producer runs; continuous
  synchronization with a Files service remains future work.
