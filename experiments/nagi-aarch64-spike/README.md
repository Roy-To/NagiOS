# Independent ARM64 QEMU feasibility spike

This is an independent freestanding Rust binary, **not nagi-kernel**, a Nagi
ARM64 boot protocol, production service or Raspberry Pi implementation. No
Linux guest or host-side service supplies its results. It is deliberately
outside the production Cargo workspace; no root/ABI/loader/kernel changes.

Current-main revalidation is PENDING. The previously recorded Rust/QEMU
versions and all old receipts belong to the historical 379d19e baseline. The
package has no external dependencies. No new build or QEMU run has been
performed for fa0e0415d381da76a190f049b07b950fcfdaca98.

The standalone runner is observational only: it reports the binary and QEMU
hashes before/after execution and always records
source_attestation=NOT_CLAIMED_BY_RUNNER. For source-bound evidence, use the
generator after reviewing a clean fixed source commit and tree:

    ./verify.sh SOURCE_COMMIT_SHA SOURCE_TREE_SHA fa0e0415d381da76a190f049b07b950fcfdaca98 $NAGI_STORAGE_ROOT/private-evidence/NEW-UNIQUE-DIR

Run through the SSD guard so NAGI_STORAGE_ROOT names the approved storage
root. The output must be new and outside the source repository. The generator
builds twice with separate empty Cargo homes and target directories, using the
pinned toolchain and --locked --offline; it runs only one of its fresh ELFs.
The second build hash is a same-host comparison, not proof of path-independent
reproducibility. It records the toolchain, build commands/logs, ELF hashes,
QEMU executable hash/path/version, serial/stderr hashes, run outcomes, source
commit/tree/clean state, and a non-circular manifest that excludes its own
attestation.json. verify_attestation.py validates those bindings without
building or running QEMU.

These JSON records and hashes can be fabricated by a party able to write the
receipt directory. Treat them as reviewable evidence, not a cryptographic
execution proof; an independently repeated build/run and artifact comparison
are still required before a full C PASS.

For a locally extracted QEMU, set `QEMU_AARCH64` and the applicable
`LD_LIBRARY_PATH`. The verifier performs EL1 boot and an expected-failure EL2
boot (`--virtualization on`). The negative run's own result is correctly FAIL;
its *negative acceptance* passes only with exit 1, the EL mismatch marker,
no timeout and no COMPLETE marker. No tolerances are weakened.

ELF is linked at QEMU virt RAM address 0x40080000, with RX/R/RW segments,
a zeroed BSS and aligned 16-KiB stack. Entry masks DAIF, selects SP_EL1 before
creating its stack and parks non-primary CPU affinity (the supported invocation
has exactly one CPU). Rust checks actual CurrentEL == EL1 and SCTLR.M == 0.
It installs a 2048-byte aligned 16-slot vector table, executes SVC #0x4e, checks
ESR EC/ISS in the handler, preserves registers and returns with ERET. It then
checks CNTFRQ is nonzero and CNTPCT advances. PL011 MMIO at 0x09000000 writes
bounded ASCII markers with a bounded TX FIFO poll. This fixed QEMU platform
address/reset behavior is not hardware discovery or a Pi UART driver.

Exact guest invocation is retained as an argument array in `result.json`:

```sh
qemu-system-aarch64 -machine virt,gic-version=2,virtualization=off \
  -cpu cortex-a53 -accel tcg -smp 1 -m 128M -display none -monitor none \
  -serial stdio -no-reboot -nic none \
  -semihosting-config enable=on,target=native \
  -kernel target/aarch64-unknown-none/release/nagi-aarch64-feasibility-spike
```

Expected ordered serial lines:

```text
SPIKE ARM64 EL1 PL011 PASS
SPIKE SVC VECTOR ERET PASS
SPIKE GENERIC COUNTER PASS
SPIKE COMPLETE PASS
```

Guest SYS_EXIT_EXTENDED uses semihosting only for exit status. Serial comes from
actual MMIO, not semihosting print or host files. If exit is unsupported, DAIF
remains masked and a DSB/WFI idle loop is bounded by the host runner's 15-second
timeout, which is a test FAIL even if markers appeared. Unexpected traps/panic
fail visibly. The runner requires aarch64 ELF, all markers exactly once/in order,
no FAIL marker, no timeout and emulator exit 0; it saves full serial/stderr,
command, QEMU version and SHA-256 digests.

NOT_RUN/unsupported: MMU mapping/W^X enforcement, EL0, Nagi syscall ABI,
interrupt-driven timer/GIC, PSCI/SMP, VirtIO, DMA/cache coherence, real Nagi
services, graphics, suspend/resume and physical hardware. ELF PHDR flags do not
prove isolation while MMU is disabled. Secondary parking and idle fallback
exist in code but are not multi-core or wakeup acceptance. The only verified
runtime is this independent single-core experiment.

The prior run used cached pinned toolchain components and a locally extracted
QEMU; its exact cache paths and tool versions are historical and stay in the
unchanged evidence tree. The spike Cargo.lock contains only the local package
and there are no dependency sections, so an empty CARGO_HOME with
--locked --offline can resolve the crate graph without downloaded crates.
The pinned rustup toolchain, target component, linker, host C compiler and QEMU
must already be installed/available; this workflow does not install them.
Separate target/Cargo-home roots can change absolute-path inputs or debug data,
so matching ELF hashes are recorded as an observed same-host comparison, not
a guaranteed path-independent reproducibility property.
