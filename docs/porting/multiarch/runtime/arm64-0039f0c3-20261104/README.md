# Source-bound verifier repair acceptance

Source `0039f0c3ad6df56fee57fcdeca7ea02bd1f9db13`, tree
`0b2597ae1b707c0a2fa8d42e16e42d46566d56c5`, exact main
`0362427747e769b00558053e4c7f4c756d303802`. The only experiment change
after 511ec126 pins the preflight Rust tools and forwards source commit/tree
arguments to the receipt verifier. The complete verify.sh exited 0 without
external tool-selection overrides. Formatting, warning-denied target Clippy,
30 model-free runner/receipt fixtures, two fresh locked/offline AArch64 builds,
real QEMU EL1 and expected-negative EL2, and explicit-SHA receipt verification
all succeeded for this source. The two 79,464-byte ELF hashes match
`47a0f88035087945ac27f763d7ff43ba2a13afc4da6677ebbef626fa35c5cc3b`.

The existing complete unsigned receipt was reverified by full SHA during the
records refresh. No new build/QEMU run was necessary for the docs-only records
children. Public summary and logs are copies/derivatives of that source run,
not execution of each later records head. The complete receipt, source archive,
ELFs and raw logs stay at the approved SSD location in [summary.json](summary.json).
Summary/receipt hashes are reviewable bindings, not a trusted execution signature.
Phase C remains PENDING independent rebuild/review. Physical hardware remains
NOT_RUN; local QEMU TCG is distinct from a physical ARM boot.

The earlier entrypoint failures and FE52 snapshot were preserved unchanged:
[earlier run](../arm64-ae9a5654-20261104/README.md) and
[checkpoint manifest](../../checkpoints/fe52-through-511ec126/manifest.json).
Current schema/3 records validate source/main identity, summary and serial hashes,
two AArch64 build results, EL1 exit 0, real EL2 rejection exit 1 without timeout,
and pending independent review. This is summary-integrity validation only.

To reproduce, check out the exact source above, use the SSD guard, then run:

```sh
sh experiments/nagi-aarch64-spike/verify.sh \
  0039f0c3ad6df56fee57fcdeca7ea02bd1f9db13 \
  0b2597ae1b707c0a2fa8d42e16e42d46566d56c5 \
  0362427747e769b00558053e4c7f4c756d303802 \
  "$NAGI_STORAGE_ROOT/private-evidence/NEW-UNIQUE-ARM64-RUN"
```

Production D/E, shared registration and ABI/BootInfo changes retain the M30 and
bounded Integration Owner/os-core gates. No media writes or merge occurred.
