# Independent ARM64 QEMU feasibility spike

This is an independent freestanding Rust binary, **not nagi-kernel**, a Nagi
ARM64 boot protocol, production service or Raspberry Pi implementation. No
Linux guest or host-side service supplies its results. It is deliberately
outside the production Cargo workspace; no root/ABI/loader/kernel changes.

Prerequisites: nightly-2025-08-01, `aarch64-unknown-none` core target,
rustfmt/Clippy, host `cc`, Python 3, and qemu-system-aarch64. Verified with
Rust 1.90.0-nightly (adcb3d3b4), Debian 13 x86-64, QEMU 10.0.13 TCG.
The package has no external dependencies. Run from the package directory so
Cargo reads its `.cargo/config.toml`:

```sh
cd experiments/nagi-aarch64-spike
./build.sh
python3 run.py
./verify.sh
```

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

No global software installation is needed. In this session Rust homes and
QEMU .deb extraction were confined to `/workspace/port-tools`; package/source
hashes and tool versions are saved under `docs/porting/multiarch/evidence/`.
