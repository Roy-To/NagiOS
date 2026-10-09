# M19 VFS creation and rename recovery

Status: recovered implementation with fresh host tests and compatibility checks passing; guest production remains PARTIAL.
Scope: `user/libnagi/src/storage.rs` and its in-file tests, plus the Files leaves.
The caller follow-up additionally changes Recovery and the optional main.rs
read-only diagnostic. No shared Cargo/CLI/CI/Registry/kernel/Browser/provider
source changes are made.

## Original defects and correction

Before this fix, a second-sector directory write failure during create could
leave an entry published while `create_in_directory` cleared its allocation
bits. After remount, creating another name reused inode 7 and its contents
became readable through the failed name. Historical reproduction logs are
preserved privately and are not included in this publication candidate.
Interrupted repacking of a directory containing fourteen 32-byte names also
made the directory unreadable.

Create now selects the free inode/data block without writes, records its
exact prior inode bytes and parent directory before allocating, then commits
all mutations under a bounded undo transaction. Rename uses the same parent
directory backup. The Files-only twelve-child and first-sector guards have
been removed. Normal Files can rename/trash/restart/restore crowded directories
through the actual VFS journal. The prior ignored regression is now a normal
passing test, `interrupted_create_never_aliases_the_next_inode`.

## Transaction and recovery

The existing Nagi format already reserves blocks 0 and 14. Block 14 stores
one 1024-byte parent backup. Sector 0 stores a SHA-256 checked header with
operation kind, parent inode/block, new inode/data allocation, prior 128-byte
inode and the backup digest. No file-data allocation or shared ABI change is
needed. The header and backup are validated before recovery writes, including
bounds, reserved exclusions, parent correspondence and collisions with another
inode's direct/indirect data or directory entries.

The transaction flushes any visible clean header before reusing the backup,
flushes the backup, publishes and flushes the header, performs the mutation,
flushes metadata, then clears and flushes the header. The initial barrier
prevents a cached zero header from being paired with an older durable header
and a newer backup after a preceding clear-flush failure.

On failure, recovery restores the parent and prior inode, clears only this
creation's allocation bits, recomputes both superblock and group free counts,
flushes the repaired state, then clears and flushes the header. Each step is
idempotent. Persistent failures retain the journal and block ordinary access
until recovery succeeds. Writable mount recovers; every subsequent public
operation also checks recovery before touching ordinary state. A final flush
failure can have an uncertain complete old/new outcome, but must not alias
files or leave an unreadable directory after successful recovery.

All mounted Vfs instances within the same userspace process share a core
atomic access gate, including desktop and Search snapshot adapters. Nested
operations use one scope; unwind resets the instance flag and releases the
gate. The caller must retain exclusive block mutation authority across
processes. This gate is not a kernel-wide lock and adds no permission grant.

## Compatibility and read-only contract

An all-zero legacy sector 0 is accepted. Unknown nonzero headers and corrupt
checksums are rejected without writes or formatting; a corrupt superblock
with such a header is also preserved. New formatting clears/flushed the
header before publishing geometry. Existing geometry and reserved allocations
are validated before journal recovery.

`check_existing` remains read-only. A valid pending journal returns the new
`StorageError::RecoveryRequired`; it never repairs or flushes. Recovery and the
optional `m27-ro-vfs-check` caller explicitly diagnose this state without repair.
The physical-console `recover` operation performs the existing no-format
writable recovery, then requires a full integrity check before exposing a mount.
Recovery is limited to regular-file create and directory rename. Mkdir, remove,
replace, and file resizing retain their prior durability limits. The protocol
assumes atomic 512-byte sector writes and honored flush barriers; malformed
sector data fails closed on checksum rather than being guessed.

## Imported historical evidence

- `cargo test -p libnagi --locked --offline`: 93 unit + 2 boot renderer PASS.
- `cargo clippy -p libnagi --all-targets --locked --offline -- -D warnings`: PASS.
- Dedicated Files suite: 24 PASS, no ignores, including 192 short/crowded
  trash/restore sector-failure/remount scenarios and normal crowded lifecycle.
- Ordinary desktop and signed production-route Nagi target compilation: PASS.

In-file fault tests enumerate every successful create/rename write and flush
boundary using one-shot and persistent failure with uncached and separately
volatile/durable disks. Power cuts discard unflushed writes; persistent cuts
prevent inline rollback from masking interruption. Recovery is interrupted
at each write/flush, then retried on a clean remount. Tests assert intact
neighbor bytes, unique inodes and read-only integrity. Other cases cover
corrupt header/backup, invalid geometry with pending/unknown headers, a
checksummed allocation collision, cached header-clear followed by another
transaction, two separately mounted instances, and unwind gate release.

Imported logs and their claimed old commit identities are historical and are
not included in this publication candidate. Fresh checks are recorded separately.
The historical guest attempt stopped in kernel M7 before init. No normal guest
Files production PASS is claimed; shared GUI/Browser hooks and CI remain separate.

## Fresh recovery validation

24 Files tests, 93 libnagi unit plus 2 renderer tests, 40 Search/IPC/localization
regressions, focused formatting and warnings-denied Clippy passed. The actual
exact-base VFS wrote a synthetic 8 MiB disk accepted by the recovered read-only
checker/mount/read_at with zero writes and flushes and identical bytes. Clean
recovered create/rename output remained readable by the exact-base VFS. No real
User Data was formatted. Pending undo requires the new recovery path; old-reader
compatibility is claimed only for clean completed state.

Native target compilation and normal guest UI/Search/restart acceptance remain
blocked or unverified. Existing imported target/VM logs are historical evidence.
