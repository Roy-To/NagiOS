# Acceptance and bounded migration plan

A PASS: baseline/ownership/remote/PR audit, all kernel modules classified,
anchored inventory with hashes and explicit unknowns. B PASS: proposal scope,
semantic negative fixtures plus unchanged production mechanism/ABI tests.
C PASS: independent real ARM64 ELF executed under QEMU, ordered guest serial,
SVC exception return and monotonic counter, exit 0; EL2 boot rejects with exit 1.
None implies M30, Nagi ARM64, x86 guest regression, Pi or phone support.
D/E: BLOCKED by M30 and owner checkpoint; no implementation attempted.

## Implemented verification

- `experiments/nagi-multiarch-contracts/verify.sh`: format, 99 actual-source and
  proposal tests, standalone warning-denied Clippy, 16 BootInfo tests, 5 ABI
  tests. Rights amplification/stale handles, IPC transfer/wait and kernel-origin
  sender IDs, VMO protection/overlap, ELF W^X/TLS/entry, scheduler/process limits,
  x86 CPU-table layout and ACPI tests retain their original assertions.
- New proposal cases validate 1/2/4 CPU topology, capacity/duplicates/missing BSP,
  version/firmware/headless distinctions, mapping overflow/alignment/W^X and
  architecture machine routing. Actual v5 and production ELF reject proposed
  v6/headless/ARM64, as before. Fixtures are not a DMA, firmware or MMU backend.
- `experiments/nagi-aarch64-spike/verify.sh`: actual cross build, format, target
  Clippy, runner failure tests, EL1 guest and actual EL2 negative boot. Serial is
  guest MMIO; semihosting is used only to report experimental exit, not to print
  markers or perform Nagi services. The CPU counter check is not timer IRQ/GIC
  acceptance. Translation stays disabled; no MMU/user-space acceptance claimed.
- `audit.py --output ... --check`: exact source hashes/evidence regenerate check.
  `./nagi dev verify` checks the unchanged registry; provisional state is separately
  JSON-Schema validated. `dev status/resume` intentionally reject this unregistered
  branch; no registry mutation to hide it. x86 product/QEMU suites: NOT_RUN.

## Proposed production changes — every row GATED

Before each step: record exact authorized base/paths, review diff with owner,
run current x86 baseline acceptance and retain image/log digests. Commit one
bounded subsystem change only after focused tests, target build and implicated
real x86 QEMU acceptance pass. Rollback for every row: revert that subsystem's
isolated commit on the assigned owner branch, rebuild the pinned baseline and
rerun the same acceptance; never reset/rewrite another branch/worktree. If boot
protocol changes, restore the matching signed loader/kernel image pair, not one
binary alone. No merge is authorized by this plan.

| Order / exact proposed edits | Required regressions before adoption |
|---|---|
| 1. `kernel/src/main.rs`: delegate serial and arch entry helpers while retaining `_start`/RCX and immediate GDT/TSS/IDT ordering; later separate x86-specific assembly | M1 boot, BootInfo rejects, pre-main NMI/#DF/#MC and panic serial; x86 binary entry inspection |
| 2. `kernel/src/memory.rs`: separate validated page-region allocator inputs from x86 page-table helpers; `user_process.rs`/`user_child.rs`: private address-space backend | M2 allocator/overflow/identity mapping, M5 W^X/guards/foreign mappings, TLB/reuse tests and real fault/EL0 isolation |
| 3. `interrupts.rs` + `cpu_tables.rs`: private trap tables/IRQ controller, clock/timekeeper facade | M2 exceptions/early failure stacks, M3 ticks/preemption; double-fault/NMI/machine-check coverage on every x86 CPU remains |
| 4. `syscall.rs`, `user_process.rs`, `user/libnagi/src/lib.rs`: arch-specific entry/frame/TLS/trampoline, shared validated dispatch | x86 register/clobber/sysret behavior, kernel-origin caller, pointer bounds, IPC/handle negatives, child isolation, native TLS/thread/PAL and M17/M18 guest regressions |
| 5. `loader/src/main.rs`, `loader/src/elf.rs`, `kernel/src/user_elf.rs`, BootInfo owner library: explicit version/architecture routing and firmware parsing | v5 136-byte ABI unchanged; wrong machine/version/size/entry/W^X/overflow reject; signed slot/trial/readiness/rollback/update including M27 and m30-update |
| 6. `smp.rs` + `acpi.rs`: normalized dense CPU IDs plus startup adapter; x86 4-CPU reference unchanged | duplicate/missing/oversize topology rejects; M3 four CPU online/work/preemption, per-CPU fault coverage, SMP race/timekeeper and stack/TLS tests |
| 7. `virtio.rs`, `net.rs`, `input.rs`, `audio.rs`, `random.rs`: queue/device semantics separate from PCI port/MMIO, discovery and DMA/cache operations | block read/write/flush capability and failure semantics; real network/input/audio/RNG; queue bounds/wrap/timeouts, memory barriers/cache/address-width and physical driver tests |
| 8. `display.rs` + user mapping consumers: optional boot display and typed display attachment without weakening current reference | unchanged framebuffer/checksum and browser pixels, bounded dynamic dimensions/stride, surface rights, pointer/touch distinction; new physical display/power criteria separately |

Root Cargo/lock, CI, IDL, syscall/BootInfo definitions, targets/linker and accepted
ADRs require explicit shared-file owners. Phase E starts only after accepted
contracts and D prerequisites: one ARM CPU → exceptions/timer/MMU → EL0/SVC and
capability negatives → PSCI/GIC/SMP → VirtIO and services. Each is independently
accepted; Raspberry Pi 5 and mobile device acceptance remains NOT IN SCOPE.

## Errors, analysis and repairs

Initial host tests passed with unused imported-API warnings. Warning-denied
Clippy then found two style defects in our mapping validator and existing style
lints in IPC tests/loader ELF/scheduler. Fixed our validator with `is_multiple_of`;
scoped only named baseline style lint allowances on imported modules in the
experiment (documented in README). No production source/test assertion changed.

Initial spike Clippy from repository root selected x86 because Cargo reads config
from the current directory. It rejected x0/x1 registers. Running the local verifier
from the spike directory uses its explicit aarch64 target; target Clippy passes.
Entry was strengthened to select SP_EL1 before establishing the stack, not after
a Rust prologue. Rebuild/guest validation passed after the change.

Host lacked Rust/QEMU initially. Rust toolchains and Debian QEMU packages were
installed/extracted only under `/workspace/port-tools`. Snapshot Debian package
index was denied by network policy; the approved `deb.debian.org` source was
used for local downloads. Resolved missing QEMU libraries locally, including
libvdeplug2t64's package-name transition and ndctl/daxctl dependencies. No system
installation, host-security change or global cache deletion. Initial logs remain
for diagnosis; verified logs identify the final candidate and exact source commit
in state/evidence after commit verification.
