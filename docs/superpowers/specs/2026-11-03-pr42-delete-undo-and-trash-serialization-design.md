# PR #42 delete recovery and Trash serialization design

## Problem and intended outcome

Vfs::remove_path_with_kind clears a parent directory entry, allocation bits, inode data, and free counters through separate writes without an undo record. A later block or flush error can return EIO while deletion is partly applied. Files Trash also stores a mirrored intent through multiple VFS calls; the process-local gate serializes each call, not the whole journal update and rename.

Make delete operations recoverable after any modeled write/flush failure and make the Files journal protocol serial within one process, while keeping the frozen 9f00f603 candidate unchanged.

## Scope

- Add a checksummed UNDO_DELETE variant to the existing sector-zero undo marker and block-14 snapshot.
- Apply it to path-based unlink/rmdir and the root-name Vfs::remove API, preserving validation and result semantics.
- Expose a documented, reentrant, process-local VFS exclusive-operation closure and use it around full Files trash, restore, and journal-backed listing calls.
- Add deterministic FaultDisk source tests for cached and write-through failure paths, including a regular file whose allocation uses the single-indirect table.
- Document that the gate is process-local.

## Design

Before mutation, the transaction captures the complete parent directory block and raw 128-byte target inode, validates their identity and block pointers, writes the undo backup, and durably publishes the marker. Delete may then clear the directory entry and release inode/block allocation. Recovery restores the saved directory and inode, sets the inode bit and each nonzero direct/indirect data-block bit, recomputes free counts from the bitmaps, flushes, and clears the marker last. These operations are idempotent, so a second failure leaves the durable marker for explicit Recovery.

UNDO_DELETE validation must accept the original and partially deleted/partially restored bitmap and inode states. It must reject malformed inode pointers, duplicate ownership, a changed/reused inode identity, and conflicting current directory contents rather than overwrite unrelated data. The saved directory must contain exactly one valid entry for the target inode. Restoring a directory inode must preserve its unchanged data block and the saved generation. Ordinary VFS operations remain blocked while Recovery is pending.

The new exclusive-operation closure holds the existing process-global gate across the full multi-call Files protocol. Nested VFS calls on the same Vfs are already reentrant by access ID. It does not provide cross-process ownership and does not make raw device access safe.

## Non-goals and limits

- `Vfs::replace` still rewrites the directory block and frees the destination through separate writes; rollback for a partially applied replacement remains separate and unresolved.
- Multi-block `write_at` and regular-file overwrite operations can partially apply data before `StorageError::Block`; rollback or truthful partial-write semantics are outside this patch.
- This change does not claim cross-process locking, sector-write atomicity on real hardware, or power-cut behavior beyond the FaultDisk model.
- Do not run the previously rejected host-test command through another target directory or environment.

## Regression plan

- For cached and write-through FaultDisk, inject each write and flush failure during path unlink, root-name remove, and empty-directory removal. Include a multi-block file with an indirect pointer.
- Cut deletion at each modeled durable event, retry recovery after write/flush failures from several progressed states, and assert the original inode/generation and bytes. Keep a version-1 rename-journal fixture.
- Seed one atomic cached writeback-order state where the inode-table sector is durable while its inode bitmap remains allocated; require explicit Recovery to restore the exact saved inode. This is a synthetic state, not evidence about physical media.
- Exercise the integrity-accepted empty-file state (`size == 0`, `blocks == 0`, with its allocated direct block) through both path unlink and root-name remove.
- Where immediate rollback succeeds, verify the original name, inode generation, contents, block ownership, free counters, neighbor file, and integrity report are unchanged.
- Where rollback cannot finish, verify normal mount/format/check are read-only RecoveryRequired, then verify explicit Recovery restores the original file and is idempotent under failures during recovery.
- For the write-through final header-clear-flush failure, recognize that the deletion may already be durable and assert the complete new state rather than claiming it was rolled back.
- For Trash concurrency, use two VFS handles to one test device and a deterministic gate. Verify the second call cannot enter the full Files protocol while the first holds it, and verify both resulting journal entries. A concurrent journal-listing assertion remains additional coverage.
- Do not execute tests until an execution route independent of the earlier auto-review rejection is available.

## Implementation and verification state

The successor worktree is based on frozen commit `9f00f6033db9465d192db43d7ef05efe8e486f0f` (tree `1ceb3575170cdb2c2a9d5e32a23ebda92f3bb75c`). The frozen worktree remains unchanged. This successor is saved in a local-only commit with source and test changes in `user/libnagi/src/storage.rs` and `user/nagi-init/src/m19_files.rs`; this design note and the implementation-status entry describe them. It has not been pushed or merged.

The source now writes version-2 delete undo records while retaining version-1 create/rename record parsing. It routes path unlink, empty-directory removal, and root-name removal through delete transactions. Source-level FaultDisk cases cover each modeled write/flush failure for path unlink, root-name remove, and rmdir, plus failures during repeated explicit recovery. The delete validator follows the existing integrity-check exception for empty files with `size == 0` and `blocks == 0`, while still validating/restoring their allocated direct block. A new regression exercises both unlink entrypoints in cached and write-through modes. The M19 Files APIs hold the same process-local VFS gate over each full multi-call protocol. The two-handle test pauses the first Trash call inside the gate, waits for the second to reach the wrapper, requires no completion during a bounded contention window, then checks both journal intents after release. This does not provide cross-process ownership.

`rustfmt 1.8.0 --edition 2021` on the two modified Rust files completed with RC 0. No Cargo build, host test, Clippy, Nagi target, VM, or QEMU run was performed. In particular, the newly added test source has not been compiled or executed; the previously rejected host-test operation was not retried through another route. `git diff --check` also completed with RC 0.

Multi-block `write_at` partial-error semantics remain outside this patch. POSIX `EIO` state confirmation, concurrent ownership from other processes, real-media sector atomicity and power-loss behavior, unknown sector-0 compatibility, and target/guest integration remain unverified or held. The FaultDisk suite models selected write/flush and power-cut states; it is not an execution proof for physical media.

The Trash concurrency source test checks two calls through separate VFS handles in one process. It does not prove cross-process exclusion or force a live cross-process TOCTOU interleaving, and it does not yet include a concurrent journal-listing assertion.