# M13 Rust std / POSIX Compatibility Implementation Plan

> **Execution contract:** Work through this plan task by task. For each task,
> use TDD: write or update the smallest focused test first, implement the
> change, run the focused test, then run the milestone acceptance test before
> recording PASS.

## Task 1: Expand the bounded user image budget

Files:

- `kernel/src/user_elf.rs`
- `kernel/src/user_process.rs`
- affected kernel tests

Change the fixed user image page limit from 16 to 256 pages and update boundary
tests to reject page 257. Preserve the existing stack/TLS mappings and all
capability checks. Run focused kernel tests and both M8/M12 regressions.

Commit: `feat: expand bounded M13 user image budget`

## Task 2: Add the Nagi PAL crate

Files:

- `Cargo.toml`
- `Cargo.lock`
- `user/nagi-pal/Cargo.toml`
- `user/nagi-pal/src/lib.rs`
- `user/nagi-pal/src/{alloc,fd,sync,time,io}.rs`

Add a `no_std + alloc` PAL with a bounded allocator, generation-checked fd
table, Nagi VFS/network adapters, synchronization/TLS primitives, clock/sleep
helpers, and native spawn result types. Use small host unit tests for pure
descriptor and allocator invariants. Exercise the actual guest adapters from
the M13 acceptance image; host tests must not stand in for guest behavior.

Commit: `feat: add Nagi user-space PAL foundation`

## Task 3: Add the POSIX C ABI facade

Files:

- `Cargo.toml`
- `user/nagi-posix/Cargo.toml`
- `user/nagi-posix/src/lib.rs`
- `user/nagi-posix/src/{errno,fs,net,process}.rs`

Expose bounded C ABI wrappers over the PAL for the M13 Tier-A surface. Keep
errno deterministic, return errors for unsupported fork/host escape paths, and
test the ABI contract on the host without pretending it proves guest I/O.

Commit: `feat: add user-space POSIX ABI foundation`

## Task 4: Build the guest vertical slice

Files:

- `tests/apps/m13_posix.c`
- `user/nagi-init/build.rs`
- `user/nagi-init/src/m13.rs`
- `user/nagi-init/src/main.rs`
- `user/nagi-init/Cargo.toml`

Compile the C test as a freestanding target object with the available LLVM
toolchain, link it into the Nagi user image, and execute it through the POSIX
ABI. Run the Rust PAL contract against real M7 VFS and M12 networking. Use the
small no-std-capable `itoa` library in the same guest path and emit the four
M13 markers only on success.

Commit: `feat: add M13 guest Rust and C acceptance path`

## Task 5: Add CLI/acceptance and close M13

Files:

- `crates/nagi-cli/src/main.rs`
- `crates/nagi-cli/tests/*`
- `tests/acceptance/m13_rust_posix.ps1`
- `tests/acceptance/m13_rust_posix.sh`
- `docs/implementation_status.md`

Add `nagi posix`, QEMU fixture setup, PowerShell and Git Bash acceptance tests,
then run workspace tests, cross-builds, focused regressions, and both M13
acceptance scripts sequentially. Update implementation status to `PASS` only
if every marker and required build/test succeeds; otherwise record the exact
blocker as `PARTIAL` or `BLOCKED`.

Commit: `docs: record M13 acceptance and advance to M14`
