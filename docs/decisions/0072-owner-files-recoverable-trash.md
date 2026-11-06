# ADR 0072: ordinary owner Files operations and recoverable trash

Status: local reintegration candidate based on ead5ee8b; M19 remains PARTIAL.
The combined Files UI and guest lifecycle have not been validated on target.

The ordinary signed-in Files panel previously submitted only Search queries.
Create/rename/permanent deletion existed in acceptance helpers. The current
VFS now journals regular-file creation and same-directory rename using
reserved metadata blocks and SHA-256 checked undo state. Preparation is
durable before allocation/directory mutation. Failed operations attempt inline
rollback; ordinary mount and VFS access never repair a pending journal and
return RecoveryRequired without writes. Only the explicit Recovery console
repairs it. The
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

The Files runtime tests use an explicit in-memory block device with the real
Nagi VFS; they do not substitute the host filesystem for guest behavior. Those
tests are carried into this candidate but are not counted as passing until the
combined workspace test builds. The standalone Recovery harness covers the VFS
recovery contract, not the Files panel or QEMU guest lifecycle.

The original PR reported historical Files and VFS test totals. Those results
apply to its older head, not to this ead5-based combination. The VFS regression
remains in the candidate tests. M19 stays PARTIAL until the combined host and
normal guest Files lifecycle evidence is available.
