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

## Independent-review P2 repair — 2026-11-04

Review of `d5bd77b9a5d4ec59e2bb797e73dcefdf9d6096ef` found that the envelope
check accepted structure lengths 1/2/3 and other nonmultiples of 4. The repair
requires at least one complete four-byte token and a length divisible by four.
A new fixture holds all ranges within bounds and keeps the strings block out
of the tested ranges, isolating the length condition. It rejects lengths
0/1/2/3/5/6/7/9/10/11/13/14/15 and accepts envelope lengths 4/8/12/16 without
claiming that those synthetic envelopes contain valid structure tokens.

The new regression failed against the preceding implementation at length 1
(`Ok(())` instead of `Err(DeviceTreeEnvelope)`), then passed with the repair.
The complete pinned contracts verifier exits 0: 105 contract tests, 30
audit/records fixtures, 16 BootInfo tests, 5 ABI tests, format and warning-denied
Clippy. The unchanged runner/receipt suite was rerun: 32 pass. Total host tests
for the repaired candidate: 188. Local repair logs are
`/tmp/arm64-host-p2-before.log`, `/tmp/arm64-host-p2-after.log` and
`/tmp/arm64-host-p2-runner.log`; prior logs and runtime receipts are untouched.

The delegation reports that the independent reviewer also verified 187 host
tests, a pinned AArch64 build, QEMU EL1 success and expected EL2 rejection for
the preceding candidate. Those reported results are not reruns of this repair
and do not establish a source-bound receipt for it. Repair review, fresh
repaired-source AArch64/QEMU checks and attestation are NOT_RUN here. Physical
profiles remain unsupported; firmware authentication and full DT property
parsing remain unimplemented. No hardware or publication action was taken.
