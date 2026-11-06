# Independent cloud rebuild and QEMU review candidate

P2 follow-up: the backend helper now requires explicit `--raw-run`, `--output`
and `--qemu-root`; it refuses any existing output path. The original backend
logs below belong to the original helper at commit `334cdc5`, and are preserved.
See [P2-followup.md](P2-followup.md) for corrected commands, regression coverage
and the retained raw-receipt/source/ELF/run binding check. A fresh supplemental
run uses a separate output directory; no earlier execution is relabeled.

Result: PASS for the freestanding ARM64 feasibility experiment at fixed source
`0039f0c3ad6df56fee57fcdeca7ea02bd1f9db13`, tree
`0b2597ae1b707c0a2fa8d42e16e42d46566d56c5`, main baseline
`0362427747e769b00558053e4c7f4c756d303802`.

The source was independently fetched from the public PR history and checked out
with a clean tracked source in a separate cloud directory. The pinned, unchanged
`verify.sh` passed formatting, warning-denied target Clippy, all 30 model-free
fixtures, two locked/offline fresh builds with separate empty Cargo homes/target
directories, actual QEMU EL1 and expected-negative EL2 runs, and full-SHA receipt
verification. Each ELF is 79,464 bytes with SHA-256
`47a0f88035087945ac27f763d7ff43ba2a13afc4da6677ebbef626fa35c5cc3b`.
Both equal the published Mac artifact for exactly this source/tree. The published
Mac summary was read from PR head `b50e8ff181efcf5093c928472fc729d5c9e231cd`;
its SHA-256 and the comparison results are in [summary.json](summary.json).

EL1 exits 0 with all four ordered markers, no FAIL, no timeout and empty stderr.
EL2 actually fails with exit 1 and `SPIKE FAIL exception-level`, without COMPLETE
or timeout. This satisfies negative acceptance (MET); it is not EL2 boot success.
Serial bytes/hashes match the fixed Mac results. PL011 output, CurrentEL/SCTLR
checks, SVC/ESR/ERET handling and generic-counter progression were reviewed in the
fixed source. Semihosting supplies exit status only.

This Linux x86-64 cloud host uses pinned nightly-2025-08-01 (rustc
1.90.0-nightly adcb3d3b4) and Debian QEMU 10.0.13. Exact tool paths/hashes,
commands, source identities, build results and serial/stderr hashes are in the
summary. Setup provides a local QEMU wrapper and extracted packages, rather than
a privileged system installation. Since the raw generator hashes that wrapper,
the actual QEMU ELF was additionally hashed and invoked directly for both modes;
[backend-check.json](backend-check.json) and its logs show matching results and
unchanged backend SHA-256. This second invocation addresses the wrapper boundary;
it does not provide hardware acceptance.

The complete unchanged unsigned receipt, source archive/snapshot, ELFs and logs
remain at `/workspace/pr57-independent-evidence-20261104/run-01/attestation.json`.
Public-candidate summary/logs are derivatives, not a replacement for inspecting
that complete receipt. Receipt hashes are reviewable bindings, not trusted
execution signatures. The included receipt-verifier stdout is the actual later
integrity recheck of that receipt. `verify.stdout.log` also contains synthetic
fixture output; the actual guest runs are the `el1-*`/`el2-*` logs and summary
commands. The original raw receipt and all older public evidence are unmodified.

## Schema treatment and remaining gates

`summary.json` uses existing `nagi-multiarch-runtime-followup/1`. Its
`acceptance.phase_c=PENDING` and `independent_rebuild=NOT_RUN` retain that schema's
self-attestation boundary, validated by the unchanged existing validator. The
separate `independent_review` section records the later independently performed
work and comparison. No existing pending-state assertion or negative test was
weakened. The workstream state's appended test row records the actual independent
PASS; prior rows remain historical results, including the older combined
independent/hardware NOT_RUN row.

Existing `current-status/3`, main evidence index, original runtime summary,
FE52 checkpoints and historical/failed records are byte-preserved. They describe
their original checkpoint, not this later execution. Canonical phase-C promotion
needs an owner-reviewed representation of independent evidence/schema adoption;
this candidate does not claim the old schema already accepts independent PASS.
A/B ownership/proposal acceptance remains pending. D/E, registry adoption, ABI/
BootInfo and production changes retain the M30 and explicit Integration Owner/
os-core checkpoint gates. Remote head/main/CI must be rechecked before any
separately authorized publication; no new latest-main guest result is claimed.

Physical hardware, production Nagi ARM64/services, MMU/EL0, timer IRQ/GIC,
PSCI/SMP, VirtIO/DMA, graphics, Miri and x86 guest regression are NOT_RUN.
QEMU TCG is an emulator test. No physical media was written, and no publication
or merge was performed.

## Exact cloud command

From `/workspace/verification-pr57-20261104`, using a new output directory on
rerun:

```sh
env PATH=/workspace/.nagi-cargo/bin:$PATH \
  RUSTUP_HOME=/workspace/.nagi-rustup CARGO_HOME=/workspace/.nagi-cargo \
  NAGI_STORAGE_ROOT=/workspace \
  QEMU_AARCH64=/home/agent/.local/bin/qemu-system-aarch64 \
  sh experiments/nagi-aarch64-spike/verify.sh \
  0039f0c3ad6df56fee57fcdeca7ea02bd1f9db13 \
  0b2597ae1b707c0a2fa8d42e16e42d46566d56c5 \
  0362427747e769b00558053e4c7f4c756d303802 \
  /workspace/pr57-independent-evidence-20261104/run-01
```

`NAGI_STORAGE_ROOT=/workspace` selects the task-authorized cloud storage; it does
not claim execution of the Mac SSD guard or physical SSD verification. The fixed
verifier's source/config/output checks were unchanged.
