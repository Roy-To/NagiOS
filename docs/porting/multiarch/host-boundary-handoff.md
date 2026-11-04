# PR #57 independent host boundary candidate — 2026-11-04

Status: PARTIAL. Based on published records head
`b50e8ff181efcf5093c928472fc729d5c9e231cd`, in independent branch
`codex/arm64-host-boundary-57`. No runtime receipt, shared owner state or
historical evidence is regenerated or rebound to this candidate.

## Changes and reproduction

The EL2 negative predicate previously accepted any serial containing
`SPIKE FAIL exception-level`, exit 1, no timeout and no final completion marker.
Three confirmed false positives are a partial EL1 success marker after that
failure, an additional panic failure, and a prefixed/non-line failure marker.
The candidate requires exactly one failure line emitted by the current spike
source (with optional line terminator). Its runner remains FAIL while the
separate expected-negative condition may be met. This change is shared by
receipt creation and receipt verification through their existing import.
Old receipts with other diagnostic strings may now reject; immutable source0039
receipts should be checked using their source-bound verifier, as before.
Fixtures now match the current guest source's exact failure text, and tests
still reject failure followed by success. No guest source changed.

The Python API also rejects virtualization values other than `off`/`on` before
file or subprocess work. CLI argparse already did this; direct API callers
previously could add comma-separated QEMU machine properties.

The contracts experiment adds `arm64_boot`, a pure preparation proposal:
missing firmware/boot-source identity or captured DTB rejects, truncated or
misaligned/out-of-range/overlapping FDT v17 envelopes reject, unknown CurrentEL
and EL2/EL3 reject, unknown/enabled MMU rejects, and unknown/non-QEMU UART rejects.
Physical profiles always reject. A successful synthetic envelope is not a
complete valid DTB; structure/property parsing and provenance authentication
remain unimplemented. There is no firmware-pointer dereference or MMIO.
Header/layout rules were checked against
[DTSpec chapter 5](https://devicetree-specification.readthedocs.io/en/stable/flattened-format.html).

Reproduce using repository-pinned nightly-2025-08-01 and Python 3:

```sh
./experiments/nagi-multiarch-contracts/verify.sh
cd experiments/nagi-aarch64-spike
python3 -m unittest test_runner test_attestation -v
```

## Observed host checks

- Complete contracts verifier: exit 0; 8 audit + 22 records fixtures,
  104 contract tests (99 existing + 5 preparation tests), original BootInfo
  16 tests and ABI 5 tests; formatting and warning-denied Clippy pass.
- Spike runner/receipt fixtures: 32 tests pass (30 existing + 2 new).
- Three false positives reproduced against baseline `run.py` loaded from
  `git show HEAD:experiments/nagi-aarch64-spike/run.py` before committing this
  candidate; each is rejected by the modified predicate.
- `git diff --check`: pass. Checks ran on Linux x86-64; logs are local
  `/tmp/arm64-host-verify.log` and `/tmp/arm64-host-runner-tests.log`.

## Remaining gates

Fresh AArch64 build/QEMU execution and independent review are NOT_RUN for this
candidate. This work does not change the existing source0039 QEMU evidence or
Phase C acceptance. Firmware identities here are reported strings, not trust
proofs. Public source does not establish an identified physical machine's
firmware handoff, DTB properties, entry EL/MMU state or UART resources; those
must be captured and reviewed before designing a physical backend. The
preparation proposal deliberately cannot authorize physical execution.
No media/physical writes, network configuration changes, public push or merge
were performed.
