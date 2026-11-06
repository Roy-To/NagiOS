# Directory rename result and delete flush regression

The parent is aa0a17cb. Keep its initial Trash journal repair and evidence immutable.

Same-parent rename is documented for directory entries. Directory data is already renamed by its transaction, but result issuance used regular-file handle_for after commit. Validate the source inode as a regular file or directory, construct/capture the result identity before mutation, and return it after commit without a new fallible inode lookup. Unsupported inode types fail before mutation. Same-name directory rename returns the same identity. Existing regular-file validation and file I/O rejection for directories remain unchanged. No cross-parent move, inode format, Undo protocol or ownership-policy change is introduced.

Regressions preserve nested children and directory inode/generation and verify directory identity cannot be used for file read/write. Failure tests enumerate successful-operation write/flush boundaries for root/nested directories in both FaultDisk cache modes, explicitly recover, assert old state, retry and verify child and neighbor contents.

Delete tests must distinguish first transaction flush failure from runtime flush after commit. Before mutation, preserve file metadata and Search identity after reconciliation; after commit, preserve query denial until reconciliation, then require file absence and tombstone. Require fault consumption in both cases. No assertion may silently assume every EIO means the operation applied.

Host source suites are the available validation route. Formal-manifest missing dependencies, Linux, target/guest, process-shared ownership and real-device ordering remain separate gates.

Reserved old/new dot and dotdot names must be rejected before lookup and same-name handling. They are mandatory directory records, not renameable entries. Root/nested regression asserts zero write/flush calls and full disk-byte equality for both entrypoints, followed by integrity. Independent static review found this inherited path and confirmed the added guard closes it.
