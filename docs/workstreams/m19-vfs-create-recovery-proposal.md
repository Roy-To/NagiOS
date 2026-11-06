# M19 VFS creation and rename recovery

Status: local Files/Recovery reintegration candidate based on ead5ee8b; M19 remains PARTIAL.
Scope: the VFS recovery core, explicit Recovery caller, Files UI/Trash leaves,
and bounded acceptance-only storage diagnostics. The candidate also wires the
standalone Recovery harness into host CI. The combined Files UI suite and guest
acceptance are pending.

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
idempotent. Persistent failures retain the journal and block ordinary access.
Ordinary mount and public operations only inspect the journal; they return
RecoveryRequired and never perform repair writes. The explicit Recovery console
uses `Vfs::recover_existing` without formatting, then checks integrity before
exposing a mount. A final flush failure can have an uncertain complete old/new
outcome; callers must surface Indeterminate and must not blindly retry it.

All mounted Vfs instances within the same userspace process share a core
atomic access gate, including desktop and Search snapshot adapters. Nested
operations use one scope; unwind resets the instance flag and releases the
gate. The caller must retain exclusive block mutation authority across
processes. This gate is not a kernel-wide lock and adds no permission grant.
Inter-process volume exclusion remains an external contract and is not proven
by this candidate.

## Compatibility and read-only contract

An all-zero legacy sector 0 is accepted. Unknown nonzero headers and corrupt
checksums are rejected without writes or formatting; a corrupt superblock
with such a header is also preserved. New formatting clears/flushed the
header before publishing geometry. Existing geometry and reserved allocations
are validated before explicit journal recovery. Compatibility with unknown
pre-existing volumes whose sector 0 is nonzero remains a HOLD.

`check_existing` remains read-only. A valid pending journal returns the new
`StorageError::RecoveryRequired`; it never repairs or flushes. Recovery and the
optional `m27-ro-vfs-check` caller explicitly diagnose this state without repair.
The physical-console `recover` operation performs the existing no-format
writable recovery, then requires a full integrity check before exposing a mount.
Recovery is limited to regular-file create and directory rename. Mkdir, remove,
replace, and file resizing retain their prior durability limits. The protocol
assumes atomic 512-byte sector writes and honored flush barriers; malformed
sector data fails closed on checksum rather than being guessed. Real-media
power-loss behavior and POSIX EIO post-failure state remain unverified.

## Historical evidence from the original PR head

The following counts and claims describe the older d817c0a PR tree, not this
ead5-based combination.

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

## Current candidate verification

Host results for this combined candidate are recorded separately in
`docs/implementation_status.md`. Prior Debian host results apply to ead5ee8b
before the Files UI/Trash paths were layered on top. No target build, VM, normal
guest Files lifecycle, or real User Data repair has been run for this combined
tree. The holds listed above remain active.
