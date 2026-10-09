# ADR 0070: ordinary owner Files operations and recoverable trash

Status: implemented for review; M19 remains PARTIAL pending guest integration evidence.

The ordinary signed-in Files panel previously submitted only Search queries.
Create/rename/permanent deletion existed in acceptance helpers. The current
VFS now journals regular-file creation and same-directory rename using
reserved metadata blocks and SHA-256 checked undo state. Preparation is
durable before allocation/directory mutation; failure or writable remount
restores parent entries, inode bytes, allocation bits and free counts. The
root fix is recovered in `user/libnagi/src/storage.rs`. Temporary Files capacity/sector
guards were removed after crowded-directory and failure tests passed.
The bounded recovery contract and remaining VFS mutation limits are described
in `docs/workstreams/m19-vfs-create-recovery-proposal.md`.

Files operations run in the OS-owned signed-in desktop, against the mounted
owner User Data VFS. Selection carries name, inode and generation. Rename and
trash reject stale selections; restoration never overwrites an existing name.
The signed `org.nagi.files` Search client, kernel-stamped caller validation,
live grants, private Workspace and visibility checks are retained.

Trash renames the regular file to `.nagi-trash-<inode hex>-<generation hex>`
in its existing directory. No inode, data block or content is removed. The
original UTF-8 name is stored in two bounded, checksummed journal files under
`/home/owner`, outside the searchable directory. Both journal copies are
written and flushed before rename. If preparing either copy fails, no
physical rename is attempted. A crash after prepare or restoration is
resolved by comparing the physical entry's inode/generation. A corrupt
latest copy can fall back without losing the most recent trash name because
both copies were prepared before rename. An orphan reserved entry or two
corrupt journals with physical trash fails closed; it is never guessed or
permanently deleted. The current UI operates on direct regular children of
`/home/owner/files`; nested Search remains supported at its existing bounds.

Ordinary Files list/create/rename/trash/restore use VFS directly, even when
Search is unavailable or over capacity. After every attempted mutation the
desktop reconciles Search and clears stale displayed results. A failed sync
disables queries until a complete reconciliation succeeds. Physical trash
retains the existing producer identity in the metadata store but is excluded
from the active Files Workspace; restoring the same inode/generation keeps
the same ObjectId. No file contents are read by the indexer.

The dedicated host suite compiles the actual leaf and runtime sources against
the real Nagi VFS with an explicit in-memory block device. It exercises
orchestration and storage encoding, not a host filesystem substitute for
guest acceptance. Shared CLI/CI/image-generation changes and normal Albert
history publication remain integration-owner proposals.

The pre-existing VFS creation inode-alias defect is reproduced in historical
failure evidence and repaired by the root transaction. The formerly ignored
regression is included in the dedicated Files suite. Imported evidence reports
24 Files and 93 VFS unit plus 2 boot renderer tests; fresh checks are recorded
separately and guest production acceptance is still open.
