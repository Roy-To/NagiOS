# M5 First User Process Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Boot a real Nagi user ELF in a separate ring-3 address space, route its bounded console syscall through Nagi's SYSCALL/SYSRET ABI, and make `nagi-init` print `Hello from user space` in QEMU.

**Architecture:** The loader will place a pinned `INIT.ELF` image in `LOADER_DATA` pages and publish its physical address in BootInfo. The kernel will validate and load that ELF into a bounded M5 bootstrap address space at `0x0000_4000_0000_0000`, using a copied kernel PML4 entry plus kernel-owned page-table and image/stack/TLS pages. A small GDT and MSR-backed SYSCALL entry will provide the native ABI; only bounded diagnostic console-write and process-exit calls exist in M5, with user pointers checked and copied before serial output.

**Tech Stack:** Rust nightly-2025-08-01, `#![no_std]` kernel/UEFI/user binaries, ELF64, x86-64 SYSCALL/SYSRET, custom Nagi target JSON, UEFI FAT12 image builder, QEMU/OVMF serial acceptance tests.

## Global Constraints

- Nagi is an independent OS and host filesystem, sockets, rendering, or inference must not implement guest behavior.
- Production runtime must not gain a Linux dependency or POSIX/Linux kernel semantics.
- Kernel keeps execution, memory, IPC, and authority; no file, socket, window, audio, package, or AI high-level syscall is added.
- The user process is untrusted; syscall numbers and user ranges are validated deterministically, and no capability/security check is bypassed.
- ELF64 and static linking are used for the custom Nagi target; published syscall numbers are never reused.
- The official acceptance target remains QEMU x86-64/q35/UEFI/4 vCPU/8 GiB with the existing VirtIO devices.
- M5's fixed bootstrap limits are explicit: one process, at most eight 4 KiB image pages, one 4 KiB stack page, and one 4 KiB TLS page. These are not presented as the final M6 process model.
- Existing M1-M4 acceptance markers and tests remain enabled and must continue to pass.

## File Map

- Create `docs/decisions/0005-m5-user-bootstrap.md`: records the fixed address-space, static-page, interrupt, TLS, and syscall scope decisions.
- Modify `crates/nagi-bootinfo/src/lib.rs`: append versioned `InitImageInfo` metadata and validation.
- Modify `loader/src/main.rs`: read `\\EFI\\NAGI\\INIT.ELF`, allocate persistent loader pages, and publish the image metadata.
- Create `kernel/src/user_elf.rs`: validate user ET_EXEC ELF64 load segments and entry permissions.
- Create `kernel/src/user_process.rs`: build the bounded user page tables, copy validated segments, map stack/TLS, and enter ring 3.
- Modify `kernel/src/memory.rs`: expose the minimum safe page-table read/write primitives needed by the bootstrap address space.
- Create `kernel/src/syscall.rs`: define syscall numbers, validate/copy console input, install GDT/MSRs, and implement SYSCALL/SYSRET assembly.
- Modify `kernel/src/main.rs` and `kernel/src/interrupts.rs`: run M5 after M4 and keep M5 bootstrap entry with interrupts disabled until M6 installs a user TSS.
- Create `user/libnagi/Cargo.toml` and `user/libnagi/src/lib.rs`: bounded no-std Rust syscall wrappers.
- Create `user/nagi-init/Cargo.toml`, `user/nagi-init/src/main.rs`, and `user/nagi-init/linker.ld`: static user ELF entry, message, panic handler, and `libnagi` calls.
- Create `targets/x86_64-unknown-nagi-user.json`: reproducible user target without the kernel linker script.
- Modify `.cargo/config.toml`: attach the user target's linker script without affecting the kernel target.
- Modify workspace `Cargo.toml` and `Cargo.lock`: add the two user crates.
- Modify `tools/nagi-cli/src/image.rs`: include `INIT.ELF` in the deterministic FAT12 image and report its cluster layout.
- Modify `tools/nagi-cli/src/commands.rs`: build/copy `nagi-init`, preserve existing artifact names for old acceptance scripts, and wait for the M5 marker.
- Create `tests/acceptance/m5_first_user_process.ps1` and `.sh`: require all prior markers, the exact user message, the syscall marker, and final M5 marker.
- Modify `docs/implementation_status.md`: record M5 as `PARTIAL` during implementation and `PASS` only after both acceptance scripts pass.

---

### Task 1: Lock the M5 bootstrap architecture and BootInfo contract

**Files:**
- Create: `docs/decisions/0005-m5-user-bootstrap.md`
- Modify: `crates/nagi-bootinfo/src/lib.rs`
- Modify: `kernel/src/acpi.rs` test fixtures and `kernel/src/memory.rs` test fixtures
- Test: `crates/nagi-bootinfo/src/lib.rs`

**Interfaces:**
- Produces `#[repr(C)] pub struct InitImageInfo { pub address: u64, pub size: u64 }`.
- Extends `BootInfo` with `pub init_image: InitImageInfo` and uses `BOOT_INFO_VERSION = 2` because the C-compatible layout changes.
- `BootInfo::validate_for_user_bootstrap` rejects a zero or empty init image with `BootInfoError::MissingInitImage`; the general pre-M5 `BootInfo::validate` remains compatible with existing M1-M4 boot fixtures until the real loader image is integrated in Task 2.

- [ ] **Step 1: Write the failing tests**

Add these assertions to the bootinfo test module, with `valid_boot_info` initially leaving `init_image` at zero:

```rust
#[test]
fn valid_boot_info_requires_a_persistent_init_image() {
    let mut info = valid_boot_info();
    info.init_image = InitImageInfo { address: 0x30_0000, size: 4096 };
    assert_eq!(info.validate_for_user_bootstrap(), Ok(()));
}

#[test]
fn empty_init_image_is_rejected() {
    let mut info = valid_boot_info();
    info.init_image = InitImageInfo { address: 0x30_0000, size: 0 };
    assert_eq!(info.validate_for_user_bootstrap(), Err(BootInfoError::MissingInitImage));
}
```

Update the existing valid test fixture's `init_image` to the nonzero value only after the first failing run has demonstrated the missing field behavior.

- [ ] **Step 2: Run the focused test to verify it fails**

Run:

```text
cargo test -p nagi-bootinfo valid_boot_info_requires_a_persistent_init_image
cargo test -p nagi-bootinfo empty_init_image_is_rejected
```

Expected: compilation/test failure because `InitImageInfo`, the BootInfo field, and the validation error do not yet exist.

- [ ] **Step 3: Implement the minimum contract**

Append `InitImageInfo` to the `BootInfo` layout, initialize it to zero in `BootInfo::new`, bump only the BootInfo layout version to `2`, add `MissingInitImage`, and implement `validate_for_user_bootstrap` so it checks `address != 0 && size != 0` after the common validation. Keep `validate` compatible with pre-M5 M1-M4 fixtures and add an explicit absent descriptor in the loader until Task 2 replaces it. Do not make the kernel trust the image without a later ELF validation step.

- [ ] **Step 4: Run focused and regression tests**

Run:

```text
cargo test -p nagi-bootinfo
cargo test -p nagi-kernel --lib memory::tests acpi::tests
```

Expected: all focused tests pass and no fixture reports `MissingInitImage`.

- [ ] **Step 5: Commit**

```text
git add docs/decisions/0005-m5-user-bootstrap.md crates/nagi-bootinfo/src/lib.rs kernel/src/acpi.rs kernel/src/memory.rs
git commit -m "feat: define M5 init image boot contract"
```

### Task 2: Load and package the real user ELF

**Files:**
- Modify: `loader/src/main.rs`
- Create: `targets/x86_64-unknown-nagi-user.json`
- Modify: `.cargo/config.toml`
- Create: `user/libnagi/Cargo.toml`
- Create: `user/libnagi/src/lib.rs`
- Create: `user/nagi-init/Cargo.toml`
- Create: `user/nagi-init/src/main.rs`
- Create: `user/nagi-init/linker.ld`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Test: `loader/src/main.rs` helper tests and `user/libnagi/src/lib.rs`

**Interfaces:**
- Loader publishes `InitImageInfo { address, size }` whose pages are allocated with UEFI `MemoryType::LOADER_DATA` before `ExitBootServices`.
- `libnagi::console_write(bytes: &[u8]) -> usize` performs syscall `1`; `libnagi::exit(code: u64) -> !` performs syscall `2`.
- `nagi-init` is a no-std static ELF whose `_start` calls `console_write(b"Hello from user space\r\n")`, then `exit(0)`.

- [ ] **Step 1: Write failing packaging and wrapper tests**

Add a loader helper test for the page count boundary:

```rust
#[test]
fn init_image_pages_round_up_and_reject_zero() {
    assert_eq!(init_image_page_count(1), Some(1));
    assert_eq!(init_image_page_count(4096), Some(1));
    assert_eq!(init_image_page_count(4097), Some(2));
    assert_eq!(init_image_page_count(0), None);
}
```

Add a host-testable libnagi contract test for the published numbers/constants without invoking the host's `syscall` instruction:

```rust
#[test]
fn published_bootstrap_syscalls_are_stable() {
    assert_eq!(SYS_CONSOLE_WRITE, 1);
    assert_eq!(SYS_PROCESS_EXIT, 2);
    assert_eq!(MAX_CONSOLE_WRITE, 256);
}
```

- [ ] **Step 2: Run tests to verify the new interfaces fail**

Run:

```text
cargo test -p nagi-loader init_image_pages_round_up_and_reject_zero
cargo test -p libnagi published_bootstrap_syscalls_are_stable
```

Expected: the helper/package and user crate are absent.

- [ ] **Step 3: Implement loader persistence and user crates**

Refactor the loader's regular-file read into a bounded `INIT.ELF` read of at most 512 KiB. Allocate `ceil(size / 4096)` `LOADER_DATA` pages with `AllocateType::AnyPages`, copy the bytes into them, and return `InitImageInfo`. Construct BootInfo only after this succeeds. Keep the allocation alive by never freeing it; the kernel allocator already consumes only UEFI conventional memory.

Use a user target with `os: "nagi"`, `panic-strategy: "abort"`, static relocation, small code model, and no kernel linker flag. Link `_start` at `0x0000_4000_0000_0000`, place text/rodata/data/bss in separate page-aligned PT_LOAD segments, and keep the image below eight pages. `libnagi` contains only the two inline-assembly wrappers and constants; `nagi-init` supplies the panic handler and calls those wrappers from `_start`.

- [ ] **Step 4: Build the actual user ELF and inspect it**

Run:

```text
cargo fmt --all
cargo build -p nagi-init --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --release
llvm-readelf --file-header --program-headers target/x86_64-unknown-nagi-user/release/nagi-init
```

Expected: an ELF64 x86-64 executable with entry in the fixed user range, static load segments, no host executable invocation, and a file small enough for the eight-page bootstrap limit.

- [ ] **Step 5: Run tests and commit**

Run:

```text
cargo test -p nagi-loader
cargo test -p libnagi
cargo build --workspace --exclude nagi-kernel --exclude nagi-loader --locked
```

Expected: focused tests and host workspace build pass. Then commit:

```text
git add Cargo.toml Cargo.lock .cargo/config.toml targets user loader/src/main.rs
git commit -m "feat: package nagi-init as a real user ELF"
```

### Task 3: Add deterministic user ELF validation and bootstrap address space

**Files:**
- Create: `kernel/src/user_elf.rs`
- Create: `kernel/src/user_process.rs`
- Modify: `kernel/src/lib.rs`
- Modify: `kernel/src/memory.rs`
- Test: `kernel/src/user_elf.rs` and `kernel/src/user_process.rs`

**Interfaces:**
- `user_elf::parse(bytes: &[u8]) -> Result<UserLoadPlan, UserElfError>` accepts ET_EXEC x86-64 ELF64 only.
- `UserLoadSegment` contains file offset, virtual address, file size, memory size, and PF flags; segments must be page-aligned, non-overlapping, within `USER_IMAGE_BASE..USER_IMAGE_LIMIT`, and the entry must be executable and loaded.
- `user_process::prepare(boot_info: &BootInfo) -> Result<UserContext, UserProcessError>` validates the image, maps at most eight image pages plus stack/TLS, and returns entry, user stack top, user TLS base, and new CR3.

- [ ] **Step 1: Write failing ELF parser tests**

Create a minimal synthetic ELF fixture with `e_type = ET_EXEC`, one PF_X segment at `0x0000_4000_0000_0000`, and a valid entry. Test rejection of identity-only addresses, writable/executable overlap, segment outside the bounded user range, and an entry outside executable bytes. The first test must call the not-yet-existing `user_elf::parse` and fail to compile.

- [ ] **Step 2: Run the focused parser test and confirm failure**

Run:

```text
cargo test -p nagi-kernel --lib user_elf::tests
```

Expected: failure because the user parser module/API is absent.

- [ ] **Step 3: Implement strict user ELF parsing**

Reuse the existing bounded ELF field-reading style, but require `ET_EXEC`, x86-64, little-endian ELF64, `PT_LOAD`, valid file/memory sizes, page alignment, canonical lower-half addresses in the fixed range, non-overlapping virtual ranges, and an executable entry. Reject W+X segments and reject more than 16 load segments before any memory is touched.

- [ ] **Step 4: Write failing address-space tests**

Add a test using a synthetic `UserLoadPlan` and local page-table storage that proves the code page is user/read/execute, the writable stack is user/write, the TLS page is user/write, and the kernel PML4 entries are retained. Add a test that an image requiring nine pages returns `UserProcessError::ImageTooLarge`.

- [ ] **Step 5: Implement the bounded page-table builder**

Expose only the page-table operations needed by M5: read a raw entry, replace an empty slot, clear a table, and obtain the current CR3. Allocate the M5 page-table hierarchy and backing pages from kernel-owned aligned static storage, copy the active kernel PML4 entries, reject an occupied user PML4 slot, and install a user PML4/PDPT/PD/PT chain at index 128. Copy each validated PT_LOAD byte range from the loader-provided identity-mapped image into the static user image pages, zero BSS, set `USER` on user entries, set `WRITABLE` only for writable segments/stack/TLS, and map the stack/TLS pages. Set FS base to the user TLS virtual address in the entry path.

- [ ] **Step 6: Run kernel focused tests and commit**

Run:

```text
cargo test -p nagi-kernel --lib user_elf::tests user_process::tests memory::tests
cargo clippy -p nagi-kernel --lib --locked -- -D warnings
```

Expected: all parser/page-table tests pass with no warnings. Commit:

```text
git add kernel/src/lib.rs kernel/src/memory.rs kernel/src/user_elf.rs kernel/src/user_process.rs
git commit -m "feat: build bounded M5 user address space"
```

### Task 4: Implement the native syscall entry and ring-3 handoff

**Files:**
- Create: `kernel/src/syscall.rs`
- Modify: `kernel/src/main.rs`
- Modify: `kernel/src/interrupts.rs`
- Modify: `kernel/src/user_process.rs`
- Test: `kernel/src/syscall.rs` host tests and the QEMU acceptance script

**Interfaces:**
- Syscall ABI constants are `SYS_CONSOLE_WRITE = 1` and `SYS_PROCESS_EXIT = 2`; no Linux syscall numbers or file/network APIs are introduced.
- `syscall::initialize() -> Result<(), SyscallError>` installs a five-entry GDT, `IA32_EFER.SCE`, `IA32_STAR`, `IA32_LSTAR`, and `IA32_FMASK`.
- `user_process::enter(context: UserContext) -> !` loads CR3, sets FS base, and executes an `iretq` frame with user CS/SS and interrupts cleared for the M5 single-thread bootstrap.

- [ ] **Step 1: Write failing syscall contract tests**

Add tests for exact numbers, bounded length, and user-pointer range:

```rust
#[test]
fn console_write_policy_accepts_only_bounded_user_ranges() {
    assert!(is_valid_user_read(USER_IMAGE_BASE, 1));
    assert!(is_valid_user_read(USER_IMAGE_BASE, MAX_CONSOLE_WRITE));
    assert!(!is_valid_user_read(USER_IMAGE_BASE, MAX_CONSOLE_WRITE + 1));
    assert!(!is_valid_user_read(0xffff_8000_0000_0000, 1));
}
```

- [ ] **Step 2: Run the focused test to verify failure**

Run:

```text
cargo test -p nagi-kernel --lib syscall::tests
```

Expected: failure because the syscall module and policy function do not yet exist.

- [ ] **Step 3: Implement syscall policy and dispatch**

Implement deterministic validation before dereferencing user memory. For syscall 1, reject a length above 256 or a range outside the mapped user image, copy bytes with volatile reads into a 256-byte kernel buffer, call the existing serial primitive, and return the copied length. For syscall 2, emit `Nagi M5 syscall PASS` and `Nagi M5 acceptance PASS`, then halt the BSP; it must never return into untrusted code. Unknown numbers return `u64::MAX` without side effects.

- [ ] **Step 4: Implement the assembly entry and GDT/MSR setup**

Use SYSCALL to enter a fixed aligned kernel stack, preserve RCX/R11/user RSP, pass the documented RAX/RDI/RSI/RDX/R10/R8/R9 registers to Rust, then restore the saved user RIP/RFLAGS/RSP and execute SYSRETQ for syscall 1. Use descriptors for kernel CS 0x08, kernel SS 0x10, user SS 0x18, and user CS 0x20; configure STAR so SYSCALL enters 0x08/0x10 and SYSRET returns to 0x20/0x18. Do not use `swapgs` or an unconstrained user-controlled stack pointer in the kernel path. Keep IF clear in the initial iret frame until M6 adds a TSS-backed interrupt stack.

- [ ] **Step 5: Wire M5 after M4 and run focused builds**

After the existing M4 acceptance markers, print `Nagi M5 user process START`, validate `BootInfo.init_image`, prepare the context, initialize syscall support, and enter the user process. Any preparation/initialization error prints a specific `Nagi M5 ... FAIL` marker and halts. Run:

```text
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --exclude nagi-kernel --exclude nagi-loader --locked -- -D warnings
cargo clippy -p nagi-kernel --lib --locked -- -D warnings
cargo build -p nagi-kernel --target targets/x86_64-unknown-nagi.json -Zbuild-std=core,compiler_builtins --release
```

Expected: all host tests/lints and the custom kernel build pass. Commit:

```text
git add kernel/src/main.rs kernel/src/interrupts.rs kernel/src/syscall.rs kernel/src/user_process.rs
git commit -m "feat: enter nagi-init through native syscall ABI"
```

### Task 5: Integrate INIT.ELF into the disk image and acceptance harness

**Files:**
- Modify: `tools/nagi-cli/src/image.rs`
- Modify: `tools/nagi-cli/src/commands.rs`
- Create: `tests/acceptance/m5_first_user_process.ps1`
- Create: `tests/acceptance/m5_first_user_process.sh`
- Modify: `docs/implementation_status.md`

**Interfaces:**
- `build_fat12_image(bootloader, kernel, init) -> Result<Vec<u8>, String>` writes `EFI/NAGI/INIT.ELF` in addition to existing files.
- `ImageLayout` reports init start cluster/count and retains existing bootloader/kernel fields.
- CLI `image` builds the user target before the kernel, reads `target/x86_64-unknown-nagi-user/release/nagi-init`, and passes it to the image writer.

- [ ] **Step 1: Write failing image tests**

Change the existing image test call sites to pass `b"init"` only after first adding this failing assertion to the new API test:

```rust
#[test]
fn includes_init_elf_in_the_nagi_directory() {
    let image = build_fat12_image(b"loader", b"kernel", b"init").expect("image");
    assert_eq!(&image[DATA_OFFSET + 128..DATA_OFFSET + 132], b"INIT");
}
```

- [ ] **Step 2: Run the image test and confirm failure**

Run:

```text
cargo test -p nagi-cli image::tests::includes_init_elf_in_the_nagi_directory
```

Expected: compile failure because the image API has only two guest files.

- [ ] **Step 3: Implement deterministic FAT12 integration**

Add the init file to the NAGI directory, allocate its chain after the kernel chain, include its size in fit checks, and preserve deterministic cluster ordering: bootloader, kernel, init. Update the CLI build sequence and error messages. Keep `out/artifacts/nagi-0.1-m1.img` and `out/logs/m1-qemu-boot.log` paths so M1-M4 scripts remain usable; the guest marker, not the filename, identifies the milestone.

- [ ] **Step 4: Add and run the M5 acceptance scripts**

The scripts must run the existing repository build/image/run flow or invoke the same checked commands, then require these serial lines in order: `Nagi M2 acceptance PASS`, `Nagi M3 acceptance PASS`, `Nagi M4 acceptance PASS`, `Nagi M5 user process START`, exact `Hello from user space`, `Nagi M5 syscall PASS`, and `Nagi M5 acceptance PASS`. They must fail if any marker is absent; they must not inject text into the serial log or use host output as a guest result.

Run:

```text
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tests\acceptance\m5_first_user_process.ps1
& 'C:\Program Files\Git\bin\bash.exe' ./tests/acceptance/m5_first_user_process.sh
```

Expected: both scripts report `PASS` after observing the QEMU guest's actual serial output.

- [ ] **Step 5: Run the complete M5 verification set**

Run:

```text
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --exclude nagi-kernel --exclude nagi-loader --locked -- -D warnings
cargo clippy -p nagi-kernel --lib --locked -- -D warnings
cargo build --workspace --exclude nagi-kernel --exclude nagi-loader --locked
cargo build -p nagi-kernel --target targets/x86_64-unknown-nagi.json -Zbuild-std=core,compiler_builtins --release
cargo build -p nagi-loader --target x86_64-unknown-uefi --release
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tests\acceptance\m5_first_user_process.ps1
& 'C:\Program Files\Git\bin\bash.exe' ./tests/acceptance/m5_first_user_process.sh
```

Review `out/logs/m1-qemu-boot.log`, record exact commands and results in `docs/implementation_status.md`, and set M5 to `PASS` only when both QEMU scripts pass. Otherwise leave M5 as `PARTIAL` or `BLOCKED` with the concrete log/error and attempted fixes.

- [ ] **Step 6: Commit the accepted milestone**

```text
git add tools/nagi-cli/src/image.rs tools/nagi-cli/src/commands.rs tests/acceptance docs/implementation_status.md
git commit -m "docs: record M5 first user process acceptance"
```

## Self-Review Checklist

- Spec coverage: M5 user/kernel separation, syscall entry, user ELF loader, user address space, stack/TLS basics, `libnagi`, and `nagi-init` are covered by Tasks 2-4; CLI/QEMU evidence is covered by Task 5.
- Security coverage: bounded user ranges, no raw host I/O, no arbitrary syscall, no privilege elevation, no W+X segment, no user-controlled kernel stack, and no bypassed capability checks are explicit.
- Regression coverage: all M1-M4 markers are required by the M5 scripts and the complete workspace suite is rerun.
- Placeholder scan: this plan contains no unfinished placeholder instructions, fake output, disabled tests, or unspecified acceptance commands.
- Type consistency: `InitImageInfo`, `UserLoadPlan`, `UserContext`, syscall constants, three-argument image builder, and extended `ImageLayout` are named consistently across tasks.
