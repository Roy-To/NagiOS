# M7 Block, Filesystem, and Persistent Storage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a real low-level VirtIO Block path, a bounded user-space VFS/ext2 slice, and a two-boot QEMU acceptance proving that a guest-created file survives reboot.

**Architecture:** The kernel owns PCI discovery, legacy VirtIO Block transport, a fixed-sector capability-checked ABI, and user writable-range validation. `libnagi` owns the BlockDevice adapter, VFS, ext2 metadata, file handles, directory operations, and bounded file-backed mapping API. The CLI creates a separate raw data disk only when absent and runs two real QEMU boots against the same disk; it never reads or writes guest filesystem contents.

**Tech Stack:** Rust nightly-2025-08-01, `no_std` kernel/user library, x86-64 legacy VirtIO PCI, fixed 512-byte sectors and 1 KiB ext2 blocks, existing QEMU q35/UEFI/4 vCPU/8 GiB reference machine, PowerShell and Git Bash acceptance scripts.

## Global Constraints

- Nagi remains an independent OS; Linux/Windows/macOS are development hosts only.
- High-level filesystem behavior remains in user space; no path, inode, directory, or file syscall is added to the kernel.
- The kernel block ABI accepts only a per-bootstrap capability, one bounded sector, and a validated user writable mapping.
- The existing FAT12 image remains the UEFI boot medium; the persistent user-data disk is a separate VirtIO Block device.
- No host filesystem access may simulate guest VFS/ext2 behavior.
- No fake response, hard-coded read result, disabled test, or weakened capability check may be used.
- Existing M0-M6 build, test, capability, syscall, FPU, and QEMU acceptance contracts must continue to pass.
- The storage vertical slice is bounded to one 1 KiB ext2 block per file, a 64-inode filesystem, root-directory operations, and a fixed-size mapping buffer.

---

### Task 1: Add the low-level legacy VirtIO Block driver

**Files:**
- Create: `kernel/src/virtio.rs`
- Modify: `kernel/src/lib.rs`
- Modify: `kernel/src/main.rs`
- Test: `kernel/src/virtio.rs`

**Interfaces:**
- `pub fn initialize() -> Result<(), BlockError>` scans PCI, selects the largest legacy VirtIO Block device, negotiates a no-feature queue, and stores the device state.
- `pub fn user_capability() -> u64` returns zero until initialization and an opaque device-derived capability afterward.
- `pub fn capability_matches(capability: u64) -> bool` checks the stored capability without exposing device internals.
- `pub fn capacity_sectors() -> Option<u64>` returns the discovered bounded capacity.
- `pub fn read_sector(sector: u64, destination: &mut [u8; 512]) -> Result<(), BlockError>` submits one synchronous descriptor chain.
- `pub fn write_sector(sector: u64, source: &[u8; 512]) -> Result<(), BlockError>` submits one synchronous descriptor chain.

- [x] **Step 1: Write failing driver contract tests**

Add host tests for PCI configuration-address encoding, VirtIO request-header fields, descriptor flags (`NEXT` and device-write), nonzero capability derivation from a selected BDF/capacity, and rejection of a sector at or above capacity. These tests use pure values and do not invoke port I/O.

- [x] **Step 2: Implement PCI discovery and legacy queue setup**

Implement port-I/O helpers for `0xCF8/0xCFC`, scan bus 0 devices/functions for vendor `0x1AF4` and device IDs `0x1001`/`0x1042`, require an I/O BAR and bus mastering, choose the largest capacity, reset the device, acknowledge/driver status, write zero guest features, require queue 0 of at least three descriptors, and set the legacy queue PFN. Keep the queue, request header, 512-byte data buffer, and status byte in page-aligned static storage below the kernel's low identity-mapped address.

- [x] **Step 3: Implement bounded synchronous requests**

Build a three-descriptor chain for the 16-byte VirtIO block header, one 512-byte data buffer, and one status byte. Set the data descriptor device-write flag only for reads, publish the avail index with a sequential fence, notify queue 0, poll the used index with a finite spin budget, validate used descriptor id and status, then copy read data out of the kernel buffer. Serialize requests with an atomic lock and return explicit errors on timeout, bad status, queue failure, or out-of-range sector.

- [x] **Step 4: Run focused kernel tests and build**

Run `cargo test -p nagi-kernel virtio --lib --locked`, `cargo clippy -p nagi-kernel --lib --locked -- -D warnings`, and `cargo build -p nagi-kernel --target targets/x86_64-unknown-nagi.json '-Zbuild-std=core,compiler_builtins' --release`. Expected: pure driver tests pass and the kernel cross-build contains no Linux dependency.

- [x] **Step 5: Commit**

```text
git add kernel/src/lib.rs kernel/src/main.rs kernel/src/virtio.rs
git commit -m "feat: add legacy virtio block transport"
```

### Task 2: Publish a capability-checked sector ABI

**Files:**
- Modify: `kernel/src/syscall.rs`
- Modify: `kernel/src/user_process.rs`
- Modify: `user/libnagi/src/lib.rs`
- Test: `kernel/src/syscall.rs`, `kernel/src/user_process.rs`, `user/libnagi/src/lib.rs`

**Interfaces:**
- Add `SYS_BLOCK_READ = 3`, `SYS_BLOCK_WRITE = 4`, and `BLOCK_SECTOR_SIZE = 512` to both kernel and `libnagi` ABI constants.
- Add `libnagi::block_read(capability: u64, sector: u64, buffer: &mut [u8; 512]) -> bool` and `libnagi::block_write(capability: u64, sector: u64, buffer: &[u8; 512]) -> bool`.
- Extend `UserContext` with `block_capability: u64`; `prepare` captures the initialized device capability and `enter` passes it in user RDI.
- Add `is_user_writable_range_mapped(address: u64, length: usize) -> bool` that accepts only writable USER image/stack pages.

- [x] **Step 1: Add failing ABI and writable-range tests**

Add tests that reject wrong syscall numbers, empty/overflowing ranges, read-only image pages, and stack ranges that cross an unmapped page. Add a source-level entry test requiring the capability move into the initial RDI state. Add host tests that the published user constants are `3`, `4`, and `512`.

- [x] **Step 2: Implement capability and mapping validation**

Dispatch only the two new numbers. For each request, require `virtio::capability_matches`, `sector < capacity_sectors`, and `is_user_writable_range_mapped(address, 512)`. Copy user bytes through a kernel `[u8; 512]` bounce buffer, call the low-level driver, and copy read bytes back with volatile accesses. Return `u64::MAX` on every failed check; do not accept a physical address or path.

- [x] **Step 3: Implement user wrappers and pass the capability**

Change the target `_start` signature to accept `block_capability: u64`, preserve the existing M5/M6 behavior, pass the capability in `enter`, and add no host fallback to the wrappers. Keep the syscall register ABI bounded and use the existing SYSCALL/SYSRET FPU preservation path.

- [x] **Step 4: Run focused tests and cross-build**

Run `cargo test -p nagi-kernel --lib --locked`, `cargo test -p libnagi --locked`, and the user-target build. Expected: all prior tests pass, the new ABI tests pass, and `nagi-init` links without undefined host symbols.

- [x] **Step 5: Commit**

```text
git add kernel/src/syscall.rs kernel/src/user_process.rs user/libnagi/src/lib.rs user/nagi-init/src/main.rs
git commit -m "feat: expose capability checked block sectors"
```

### Task 3: Implement user-space VFS and bounded ext2

**Files:**
- Create: `user/libnagi/src/storage.rs`
- Modify: `user/libnagi/src/lib.rs`
- Test: `user/libnagi/src/storage.rs`

**Interfaces:**
- `pub trait BlockDevice { fn read_sector(&mut self, sector: u64, destination: &mut [u8; 512]) -> Result<(), StorageError>; fn write_sector(&mut self, sector: u64, source: &[u8; 512]) -> Result<(), StorageError>; }`
- `pub struct SyscallBlockDevice { capability: u64 }` implements `BlockDevice` through `libnagi::block_read/write`.
- `pub struct FileHandle { inode: u32, generation: u16 }` is returned by create/open and validated on every operation.
- `pub struct DirectoryEntry { pub inode: u32, pub name: [u8; 32], pub name_len: u8 }` is returned by root listing.
- `pub struct Vfs<D: BlockDevice>` provides `mount_or_format`, `create`, `open`, `list_root`, `write`, `read`, and `mmap`/`flush_mapping`.
- `pub struct FileMapping { ... }` provides `bytes()` and `handle()`; mapping length is at most 1024 bytes.

- [x] **Step 1: Write failing ext2/VFS tests**

Add a fixed in-memory test BlockDevice and tests for formatting/mounting, root `.`/`..` entries, create/open/list, file-handle generation rejection, 1 KiB write/read round trip, duplicate-name rejection, short-buffer rejection, malformed-superblock rejection, and mapping load/flush. The tests must exercise bytes stored in the BlockDevice, not a host path.

- [x] **Step 2: Implement sector/block helpers and ext2 metadata**

Use two 512-byte sectors per 1 KiB block. Format one ext2 group with 8192 blocks, 64 inodes, block bitmap 3, inode bitmap 4, inode table blocks 5-12, root directory block 13, and the first regular-file data block 14. Encode/decode little-endian superblock, group descriptor, inode, and directory records manually with bounds checks and explicit `StorageError` values.

- [x] **Step 3: Implement VFS operations and file handles**

Restrict M7 names to nonempty root-directory byte names of at most 32 bytes and files to one direct 1 KiB block. Allocate the first free inode/data block from the bitmaps, update free counts, insert a valid ext2 directory record, and return a generation-1 `FileHandle`. `open`, `read`, and `write` must reject invalid inode/type/generation and never infer a path from a host filesystem.

- [x] **Step 4: Implement bounded file-backed mapping**

`mmap` loads the file into a fixed 1024-byte user-space mapping buffer, `bytes()` exposes only the recorded file length, and `flush_mapping` writes the bounded bytes back through VFS. Keep the API explicit about the M7 bounded mapping contract; do not claim demand-paged kernel VMO behavior that is not present yet.

- [x] **Step 5: Run library tests and user-target build**

Run `cargo fmt --all -- --check`, `cargo test -p libnagi --locked`, `cargo clippy -p libnagi --all-targets --locked -- -D warnings`, and the user-target build. Expected: ext2 tests pass and the no_std user ELF still links.

- [x] **Step 6: Commit**

```text
git add user/libnagi/src/lib.rs user/libnagi/src/storage.rs
git commit -m "feat: add bounded user space ext2 vfs"
```

### Task 4: Integrate persistent data-disk orchestration

**Files:**
- Modify: `tools/nagi-cli/src/commands.rs`
- Modify: `tools/nagi-cli/src/image.rs`
- Test: `tools/nagi-cli/src/image.rs`, `tools/nagi-cli/tests/cli.rs`

**Interfaces:**
- Add `PERSISTENT_DISK_SIZE = 16 * 1024 * 1024` and preserve `out/artifacts/nagi-0.1-user-data.img` when it exists.
- Extend `run_qemu` with an explicit `acceptance_marker: &str` and a second `if=none` raw drive attached as `virtio-blk-pci,disable-modern=on`.
- First boot uses `NAGI_WRITE_MARKER = "Nagi M7 persistent write PASS"` and `out/logs/m7-first-boot.log`; second boot uses `GUEST_ACCEPTANCE_MARKER = "Nagi M7 acceptance PASS"` and `out/logs/m1-qemu-boot.log`.

- [x] **Step 1: Add failing CLI orchestration tests**

Add pure tests for the M7 final marker gate, the first-boot write marker, and the data-image path/size policy. The tests must verify that an existing image is preserved and that M5/M6 markers are not accepted as M7 completion.

- [x] **Step 2: Add a persistent raw data disk without guest interpretation**

Create the image only when absent with `File::create`/`set_len`, pass it to QEMU as a second VirtIO Block device, and leave existing files untouched. Keep the boot FAT12 drive and OVMF handling unchanged.

- [x] **Step 3: Implement two-marker QEMU waiting**

Parameterize the polling helper by marker. In `execute_run`, run the first boot until the guest write marker; if the first boot already contains the final M7 marker because data existed, retain/copy that log as the final log. Otherwise run a second boot against the same data image until the final marker. Fail if neither expected marker is present; never treat a timeout or arbitrary QEMU exit as success.

- [x] **Step 4: Run CLI tests and host build**

Run `cargo test -p nagi-cli --locked`, `cargo clippy -p nagi-cli --all-targets --locked -- -D warnings`, and `cargo build --workspace --exclude nagi-kernel --exclude nagi-loader --locked`. Expected: the final marker is M7-only and the orchestration compiles.

- [x] **Step 5: Commit**

```text
git add tools/nagi-cli/src/commands.rs tools/nagi-cli/src/image.rs tools/nagi-cli/tests/cli.rs
git commit -m "feat: orchestrate persistent qemu data disk"
```

### Task 5: Run the storage scenario in `nagi-init` and add Acceptance tests

**Files:**
- Modify: `kernel/src/main.rs`
- Modify: `kernel/src/syscall.rs`
- Modify: `user/nagi-init/src/main.rs`
- Create: `tests/acceptance/m7_block_filesystem_persistence.ps1`
- Create: `tests/acceptance/m7_block_filesystem_persistence.sh`

**Interfaces:**
- Kernel emits `Nagi M7 storage START` and `Nagi M7 VirtIO Block PASS` after driver initialization and before ring-3 entry.
- `nagi-init` emits first-boot `Nagi M7 ext2 format PASS`, `Nagi M7 file create PASS`, `Nagi M7 file write PASS`, `Nagi M7 file-backed mmap PASS`, `Nagi M7 persistent write PASS` and exits with code `2`.
- On the second boot it emits `Nagi M7 ext2 mount PASS`, `Nagi M7 directory lookup PASS`, `Nagi M7 file read PASS`, `Nagi M7 file-backed mmap PASS`, `Nagi M7 persistent read PASS`, then exits with code `0`.
- Kernel exit code `0` emits existing M5/M6 success markers followed by `Nagi M7 acceptance PASS`; exit code `2` emits only a reboot-required diagnostic, never a final acceptance marker.

- [x] **Step 1: Add failing guest marker contracts**

Add marker-order tests or script fixtures requiring the two distinct logs: first boot must contain the real write path and second boot must contain mount/read/mapping plus the terminal marker. Require the complete M0-M6 regression sequence in each final boot log.

- [x] **Step 2: Integrate storage initialization and exit phases**

Initialize the kernel driver before user entry, pass the capability through `UserContext`, add the exit-code-2 diagnostic, and keep all high-level ext2 operations in `nagi-init` through `libnagi::storage`.

- [x] **Step 3: Implement real create/write/reboot/read flow**

Use a fixed static filename and payload. On a blank/empty volume, format if needed, create the root file, write the payload, map and verify it, print only verified progress markers, and exit `2`. On the next boot, mount without formatting, list/lookup the directory entry, read and compare exact payload bytes, map and verify again, and exit `0`.

- [x] **Step 4: Run both real-QEMU Acceptance scripts**

Each script must remove only repository-owned `out` artifacts through the supported clean command before the first run, invoke `nagi run`, inspect `out/logs/m7-first-boot.log` and `out/logs/m1-qemu-boot.log`, and report PASS only after both ordered logs satisfy the contracts. Run:

```text
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tests\acceptance\m7_block_filesystem_persistence.ps1
& 'C:\Program Files\Git\bin\bash.exe' ./tests/acceptance/m7_block_filesystem_persistence.sh
```

- [x] **Step 5: Commit**

```text
git add kernel/src/main.rs kernel/src/syscall.rs user/nagi-init/src/main.rs tests/acceptance
git commit -m "test: prove persistent ext2 data across reboot"
```

### Task 6: Complete M7 verification and update the Source of Truth

**Files:**
- Modify: `docs/implementation_status.md`
- Modify: `docs/superpowers/plans/2026-09-18-m7-block-filesystem-persistence-plan.md`
- Modify: `docs/superpowers/specs/2026-09-18-m7-block-filesystem-persistence-design.md` only if evidence requires clarification

- [x] **Step 1: Run the complete verification set**

Run `cargo fmt --all -- --check`, `cargo test --workspace --locked`, both required Clippy commands, host build, user-target build, kernel-target build, UEFI loader build, `nagi doctor`, and both M7 Acceptance scripts. Expected: all pass without changing prior tests or architecture boundaries.

- [x] **Step 2: Review logs and repository state**

Confirm the first log proves format/create/write/mmap and the final log proves mount/directory lookup/read/mmap plus M5/M6/M7 terminal markers. Run `git diff --check`, inspect `git status --short`, and ensure no generated image/log is staged.

- [x] **Step 3: Update status only after Acceptance passes**

Set M7 to `PASS`, set current milestone M8 `NOT STARTED`, record the exact commands, commits, image preservation behavior, and both guest logs. If any Acceptance or full verification command fails, leave M7 `PARTIAL` or `BLOCKED` with the concrete error and do not advance.

- [x] **Step 4: Commit the accepted milestone**

```text
git add docs/implementation_status.md docs/superpowers/plans/2026-09-18-m7-block-filesystem-persistence-plan.md
git commit -m "docs: record M7 persistent storage acceptance and advance to M8"
```

## Self-Review Checklist

- Spec coverage: VirtIO core/block, VFS, ext2, file handles, directory operations, persistent data, and the bounded file-backed mapping contract each have a task.
- Kernel boundary: no path, inode, directory, or file syscall exists; the only new ABI is capability-checked sector I/O.
- Persistence proof: two guest boots share one raw VirtIO data disk and the host never interprets its contents.
- Security: capability, user writable mappings, sector bounds, finite queue polling, handle generations, and malformed metadata all fail closed.
- Regression: M0-M6 scripts and full workspace/cross-target builds remain required.
- Placeholder scan: no `TBD`, `TODO`, fake-output, disabled-test, or ambiguous implementation step remains.
