# PORT-MULTIARCH-01 handoff — provisional, unregistered

## 2026-11-04 publication and runtime follow-up

Fixed candidate ae9a56544d2c66cbbf6e641b80cd144ed52314a4 was published by an
ordinary fast-forward to the existing PR #57 branch, preserving its SHA and
ancestry. A fresh source-bound ARM64 build and actual QEMU EL1/EL2 runs were
performed after the older FE52 static/host checkpoint below. EL1 passed, the
EL2 negative condition was met, and two fresh ELF hashes matched. Phase C is
PENDING independent rebuild/review. The overall verify.sh is still failing
(two concrete entry-point defects); explicit-SHA receipt verification passed.
See [runtime follow-up](runtime/arm64-ae9a5654-20261104/README.md) for exact
results, failed attempts, commands and remaining physical-hardware limitations.
The older NOT_RUN/no-push statements below describe their earlier checkpoints.

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

Current A/B status remains PENDING and C is NOT_RUN. The inventory and authority
records are generated with explicit full base 0362427747e769b00558053e4c7f4c756d303802 and fixed source commit
fe52fedcbfe4bb90dd8a6f66ad5912f0bd3b2b50. The fixed-main diff still has no changes under the owned
multiarch, contract, and spike prefixes. Existing 379d historical evidence,
validation-evidence.json, and the handwritten EL2 summary remain byte-preserved.

The 60 model-free fixture suites (30 audit/records and 30 runner/attestation),
nine-file Python syntax check, two shell syntax checks, and explicit-base
inventory check pass for the fixed source snapshot. The pinned host verification
entry point now selects nightly-2025-08-01 for Cargo and Rust tools and passed
locked/offline contract tests (99 unit tests, 0 doctests), cargo fmt, strict
Clippy with warnings denied, 16 BootInfo tests, and 5 ABI tests.

Earlier host attempts remain failures. The first unpinned stable invocation
exited 101 with E0658 for unsigned_is_multiple_of; its per-command metadata did
not capture the effective rustc version, so the exact old compiler version is
not asserted here. A nightly-labeled run also failed E0658 while the effective
cargo child compiler was not attested. A later temporary feature-gate attempt
failed, and the pinned-nightly test run on 38f95bac passed tests but strict
Clippy rejected the now-unnecessary feature attribute. Those attempts remain
separate from the FE52 pinned-nightly PASS.

The records-only verifier passed at direct child 274d52f883cf0a348b1ce429fcca7d7a0a7f6e02 (tree c23394d8cc02ad3bfd81f83ba5dff169d5c5a8ee); its captured stdout SHA-256 is 172e7648eafbbb59ec7dabbee61e83c79c16f4bbbd300c347694f984f6df6194 and is recorded in current-main evidence.

Current-status schema /2 uses target_execution for AArch64-target/runtime attempts. Host Cargo and Clippy outcomes are recorded under tool_acceptance.host_Cargo_contract_tests.

Phase A remains PENDING while live ownership/ref review is NOT_RUN. Phase B
remains PENDING because the architecture proposal is PROPOSED_UNREGISTERED and
still needs owner review, despite the host-test subcheck passing. Phase C stays
NOT_RUN: no AArch64-target Cargo build, new ELF, actual QEMU/VM, Miri, or
independent rebuild was performed. No push, PR mutation, or remote merge was
performed.
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

## R1 verifier attempt history

The first R1 records-verifier attempt used commit `a67f34574be1f45b5adeb59205eae3dfe9a7d3de` (tree `1c28083f37c82222bbbc8a2849b6346f98145d44`) and exited 2 because `last_verified.tests[8].outcome` was set to the invalid enum `PENDING`. Its stdout was empty; stderr SHA-256 was `53b28fa4adec98b756a9f88afae30e8ebbb5a13b5900fa1eece662a9920d8b5f`. This attempt remains recorded as a failure, not a PASS.

After correcting the state to schema-valid `NOT_RUN`, R1 was committed as `5bb809b522aaeba6cc5b4ea545a083d92861dac5` (tree `6357c9b4181ea89d4b77ac8447ce2a5662d6ee3c`), a direct child of source `2c2103e3c87f487221b64117014797d1b2f52be4`. Its verifier exited 0; committed stdout SHA-256 is `548acec7a938250999334375f7d7d219358c2d6c57c005e238780f131e78c4c4`.

The original failed and successful R1 metadata used the same `name` and omitted `cwd`. Their metadata now has distinct attempt names and records the guard workspace `/Volumes/SSD/NagiOS-Development-20261012`. The raw stdout/stderr files and their hashes were not changed; `records-r1-meta-correction.json` records the metadata update hashes. R2 verifier metadata separately records its unique name and repository working directory.

The P3 follow-up adds negative fixtures for a source-snapshot Cargo config that
mismatches the Git blob, a Cargo config symlink in the snapshot, and a fake
Cargo process that changes the committed config during a build. `build_once`
now persists Cargo stdout/stderr before its post-build config check; a config
change fails with an error pointing to that build's logs. The fake build test
checks the rejection and retained log contents. These are model-free fixture
checks; no real Cargo process was run.

All candidate changes remain within `docs/porting/multiarch/**`,
`experiments/nagi-multiarch-contracts/**`, and
`experiments/nagi-aarch64-spike/**`. Root workspace/kernel/CI, registry, accepted
ADR, implementation status, and production sources remain unchanged. D/E remain
blocked by the existing M30 and owner checkpoint gates.
