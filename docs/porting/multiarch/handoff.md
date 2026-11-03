# PORT-MULTIARCH-01 handoff — provisional, unregistered

## Current-main refresh

Explicit base: fa0e0415d381da76a190f049b07b950fcfdaca98 (current main at
the start of this refresh). The candidate preserves PR #57 head
a7da7b76f69c9470c72eca3976876a1b666473ef as an ancestor and includes the
current main commit as an ancestor. Updating the existing PR branch can be an
ordinary fast-forward; this phase did not push, edit the PR, or merge remotely.

The 12 paths introduced by main after the old 1b3637ea base are limited to
Identity crate/test/workflow files. The fixed diff from 1b3637ea to
fa0e0415... is empty under docs/porting/multiarch/,
experiments/nagi-multiarch-contracts/, and
experiments/nagi-aarch64-spike/. This confirms the spike/contract source is
unchanged; it does not refresh any run or source-inventory evidence.

Current A/B/C status is PENDING/NOT_RUN. Old evidence files, validation record,
and handwritten EL2 summary remain byte-for-byte historical inputs indexed by
[evidence/HISTORICAL.json](evidence/HISTORICAL.json). The new inventory is
generated from an explicit full --base SHA. attest.py accepts only source
commit/tree/main plus a fresh output path; it builds two fresh isolated ELFs
and passes only one of those ELFs to the observational runner. The runner
cannot claim source identity. Receipt verification checks source ancestry,
tree, scope, clean status, build/run artifact hashes, QEMU identity, EL1 PASS,
and the EL2 expected-negative runner FAIL. JSON and hashes can be fabricated;
independent rebuild and review are still required.

No Cargo build, new ELF, QEMU/VM, or remote operation was performed in this
phase. The model-free Python tool tests, formatting/syntax checks, and static
inventory are tied to the fixed source commit in private SSD logs. C current
runtime validation and independent rebuild remain NOT_RUN.

All candidate changes remain within docs/porting/multiarch/**,
experiments/nagi-multiarch-contracts/**, and
experiments/nagi-aarch64-spike/**. Root workspace/kernel/CI, registry,
accepted ADRs, implementation status, and production sources remain unchanged.
D/E remain blocked by the existing M30 and owner checkpoint gates.
