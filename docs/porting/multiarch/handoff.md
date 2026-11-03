# PORT-MULTIARCH-01 handoff — provisional, unregistered

## Current-main refresh

The canonical `Roy-To/NagiOS` main was checked by full SHA and is `0362427747e769b00558053e4c7f4c756d303802`.
The candidate preserves PR #57 head `a7da7b76f69c9470c72eca3976876a1b666473ef`
and merges that exact main commit through a local merge commit. The branch update
therefore preserves the published ancestry and remains fast-forwardable. No push,
PR edit, or remote merge was performed.

The fixed diff from historical main `1b3637ea6dde81c46c2bc3aa25442545111668b0`
to `0362427747e769b00558053e4c7f4c756d303802` changes 35 paths, all in automation, calendar/search, Identity, and
TTS crates/workflows/tests. The diff is empty under
`docs/porting/multiarch/`, `experiments/nagi-multiarch-contracts/`, and
`experiments/nagi-aarch64-spike/`; the contract and spike sources remain unchanged
by main. The former `main-fa0e0415d381da76a190f049b07b950fcfdaca98` record directory is indexed as SUPERSEDED, while
the `0362427747e769b00558053e4c7f4c756d303802` records are indexed as CURRENT. Existing 379d historical evidence,
`validation-evidence.json`, and the handwritten EL2 summary remain byte-preserved.

Current A/B status remains PENDING and C is NOT_RUN. The new inventory and
authority records are generated from an explicit full `--base` and fixed source
commit. `attest.py` requires source commit/tree/main, archives that exact commit,
builds from a read-only snapshot, discards inherited build/runtime variables,
rejects external ancestor Cargo config, and sends only a newly built ELF to the
observational runner. It verifies exact manifests, source/build/run bindings and
both build hashes. A DIFFERENT two-build result reports reproducibility failure
with exit 2. Receipt JSON and hashes remain forgeable, so independent rebuild and
review are still required.

The model-free fixture suites pass 49 tests (25 audit/records and 24 runner/
receipt tests); Python syntax compilation passes for nine tools. The final fixed
source and record commits are checked by explicit-base inventory and records
verification. No Cargo build/check/Clippy, new ELF, QEMU/VM, Miri, independent
rebuild, push, or PR mutation was performed. Current runtime validation remains
NOT_RUN.

All candidate changes remain within `docs/porting/multiarch/**`,
`experiments/nagi-multiarch-contracts/**`, and
`experiments/nagi-aarch64-spike/**`. Root workspace/kernel/CI, registry, accepted
ADR, implementation status, and production sources remain unchanged. D/E remain
blocked by the existing M30 and owner checkpoint gates.
