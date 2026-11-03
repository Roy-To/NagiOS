# PORT-MULTIARCH-01 handoff — provisional, unregistered

## Current-main refresh

The canonical `Roy-To/NagiOS` main was checked by full SHA and is `0362427747e769b00558053e4c7f4c756d303802`.
The candidate preserves PR #57 head `a7da7b76f69c9470c72eca3976876a1b666473ef`
and merges that exact main commit through a local merge commit. The branch update
therefore preserves the published ancestry and remains fast-forwardable. No push,
PR edit, or remote merge was performed.

The fixed diff from historical main `1b3637ea6dde81c46c2bc3aa25442545111668b0`
to `0362427747e769b00558053e4c7f4c756d303802` changes 36 paths, all in automation, calendar/search, Identity, and
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
hash-verifies committed snapshot Cargo config, rejects external ancestor and
isolated Cargo-home config, and sends only a newly built ELF to the observational
runner. It verifies exact manifests, source/build/run bindings and
both build hashes. A DIFFERENT two-build result reports reproducibility failure
with exit 2. Receipt JSON and hashes remain forgeable, so independent rebuild and
review are still required.

The model-free fixture suites pass 57 tests (30 audit/records and 27
runner/receipt tests); Python syntax compilation passes for nine tools. The
final fixed source and record commits are checked by explicit-base inventory
and records verification. No real Cargo build/check/Clippy, new ELF, QEMU/VM,
Miri, independent rebuild, push, or PR mutation was performed. Current runtime
validation remains NOT_RUN.

## Review remediation: generator and records validation

attest.py allows Cargo config only when .cargo/config or .cargo/config.toml is
part of the archived source commit and its SHA-256 matches the committed
blob. It rechecks the config around each build and rejects external ancestor
config and any config in isolated CARGO_HOME. The generator now returns the
attestation object expected by its CLI, preserves raw Git blob bytes for source
identity, and binds the post-build HEAD to the checked source commit.

A model-free end-to-end test uses fake rustup, Cargo, QEMU, and synthetic ELF
files. It exercises source snapshot creation, two builds, EL1/expected-negative
EL2 runner branches, receipt generation, receipt verification, and DIFFERENT
build exit 2. This validates orchestration only; it is not a real build or QEMU
run.

records_validation now binds a zero exit code and saved stdout path/hash to a
fixed evidence log. That log includes the main/source/record commit and tree
identities. Verification requires the recorded head to be the direct
records-only child of the source commit and checks the file hash and exact
PASS identity prefix. Receipt JSON and logs remain unsigned and can be
fabricated; independent review/rebuild remains required. Same-host build hash
comparison remains diagnostic rather than path-independent reproducibility
proof.

All candidate changes remain within `docs/porting/multiarch/**`,
`experiments/nagi-multiarch-contracts/**`, and
`experiments/nagi-aarch64-spike/**`. Root workspace/kernel/CI, registry, accepted
ADR, implementation status, and production sources remain unchanged. D/E remain
blocked by the existing M30 and owner checkpoint gates.
