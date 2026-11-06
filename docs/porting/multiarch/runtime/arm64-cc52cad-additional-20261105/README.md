# Supplemental cc52 runtime metadata candidate

Status: COMPLETE_METADATA_ONLY. Phase C remains PENDING. This additional record does not
replace the source0039 records or any previous failed e6 attempt.

The independent comparison owner's report states that exact cc52's original
formal receipt passed the original verifier with 33 referenced artifacts and
exit 0. A different environment matched the source archive, ELF and EL1/EL2
serial artifacts by manual observation without generating a formal receipt.
The original formal verifier's exit 0 is retained as a recorded verification,
not a new formal-verifier execution here. The authorized transferred artifact
bytes have now been hash-checked to derive both summaries, as detailed below.
No Cargo, QEMU or formal attestation verifier was executed in this update.

## Fixed source and preservation

The additional record pins source
`cc52cadcf0ef7c8b6c0683b1a9be3a85ef4d7e4e`, tree
`83214d70643e016087b53f1fb5f27e45005676cb`, spike tree
`4dac311883b7e6299662514ece2c61576916a070` and fixed main
`0362427747e769b00558053e4c7f4c756d303802`. The public composition base remains
`b50e8ff181efcf5093c928472fc729d5c9e231cd`.

The old records are taken exactly from reviewed dc11
`dc11f55d934ae73f508858458a09a3994889520c`, tree
`26beb4905c44132d63a6db377e23da0521448ab3`. Their state still binds source0039;
the supplemental record is separate and does not repurpose that state. The old
schemas, verifier, runtime records, indexes and checkpoints are unchanged.
The current source's root status equals fixed main036; no root scope exception
or adoption of the e6 composition contract is needed for this source.

## New metadata contract

`record.json` uses a separate `nagi-multiarch-additional-runtime/1` contract.
The validator requires explicit public-base, current-source, historical-records
and runtime-main commit anchors, verifies trees/ancestry/subtree, and compares
the complete tracked tree with the exact nonoverlapping input overlay. Only
the four supplemental tool/record/document paths and two referenced summary
paths may be added. Modes are checked; guessed serial files and unrelated
paths reject. Existing schema/verifier changes reject.

The initial candidate `86444bdc8e4d594a46dca8a5526c98ef04b23eaf` deliberately
left both summary references null. This records commit replaces those null
references with hashes of the two newly derived, measured summaries. The
pending version remains in Git history. No hash is inferred from expected ELF
or serial values. No serial or receipt file is added to this candidate.

The summaries use these strict formats. Old runtime records and the original
receipt are not overwritten or rewritten:

- `formal-summary.json`: schema `nagi-receipt-verification-summary/1`, kind
  `REPORTED_FORMAL_RECEIPT_VERIFICATION`, exact `source` object, actual
  `receipt_sha256`, `verifier_stdout_sha256`, integer `verifier_exit_code` 0,
  integer `manifested_files` 33, and `artifacts` with measured
  `source_archive_sha256`, `elf_sha256`, `el1_serial_sha256`,
  `el2_serial_sha256`.
- `manual-summary.json`: schema `nagi-manual-artifact-comparison/1`, kind
  `MANUAL_OBSERVATION_ONLY`, exact `source` object, boolean
  `formal_receipt_generated` false, and the same four independently measured
  artifact digest fields in `artifacts`. No PASS, receipt or verifier-exit
  fields are allowed. Digest comparison is not a formal receipt verdict.

In both summaries `source` has exactly `commit`, `tree`, `main`, `spike_tree`,
matching the pinned cc52 identity. JSON source bytes are bound by each
`record.json` summary reference's SHA-256. A complete fixture validates metadata
consistency only; it is not proof of execution against an actor able to alter
both metadata and artifacts. The validator always retains Phase C PENDING,
physical/production ARM64 NOT_RUN, manual receipt NOT_GENERATED and raw receipt
rechecked here false, even after the public metadata is complete.

## Checks and remaining input

```sh
python3 -B docs/porting/multiarch/test-additional-runtime.py -v
python3 -B docs/porting/multiarch/verify-additional-runtime.py \
  --base b50e8ff181efcf5093c928472fc729d5c9e231cd \
  --source cc52cadcf0ef7c8b6c0683b1a9be3a85ef4d7e4e \
  --records dc11f55d934ae73f508858458a09a3994889520c \
  --main 0362427747e769b00558053e4c7f4c756d303802
```

The CLI returns exit 2 while either approved summary is missing. This is an
explicit input gate, not a permission denial. It returns exit 0 only for
complete, consistent metadata, while runtime adoption remains pending. Eighteen
isolated synthetic regression tests cover complete/pending metadata, wrong
source/tree/main, changed summary bytes, failed or incomplete formal checks,
manual PASS/receipt injection, invalid/mismatching digests, phase promotion,
false raw-recheck claims, historical artifact mutation, invented serial, root
status changes, summary path escapes and CLI pending/complete exit codes. No fixture is published as real
execution evidence.

Both authorized summaries are now present with measured hashes. Any formal
receipt claim for the separate manual environment would require its own new
receipt; this candidate makes no such claim. The new supplemental contract
and completed record still need independent review/owner adoption. No public
operation, push, merge, dispatch resend, physical/media write or network change
was performed.

Pending predecessor `86444bdc8e4d594a46dca8a5526c98ef04b23eaf` passed 235 host tests: 18 supplemental metadata/CLI tests,
8 source-scope tests, 11 runtime-summary tests, 10 backend-helper tests,
30 audit/records tests, 105 contracts, 16 BootInfo, 5 ABI and 32 runner/receipt
fixtures. Pinned formatting and warning-denied Clippy passed. On the actual
predecessor's pending record the CLI returned exit 2 with exactly the two missing summaries;
synthetic complete fixtures return metadata-only exit 0. There is no new guest
run or raw-receipt recheck behind these host checks.

## Measured summary derivation

Both packages were read through the authorized private Page's official file
reader as exact UTF-8 source bytes and copied only into private scratch storage.
The formal transfer is 94,383 bytes with SHA-256
`3ccda4d51abc57d1d061232b53cfbd5ee9d46fcf5437146398bdeb44bc188e9f`;
the manual transfer is 72,628 bytes with SHA-256
`bd6848184cc72596bcc33517efb098dde69e8c0d54877ba52140cfc47ccd4aab`.
The decoded formal gzip/tar and all 39 declared files matched their supplied
sizes and hashes. Only declared regular relative members were accepted; unsafe,
duplicate or unexpected members reject. The receipt manifest exactly covered
its 33 referenced files with matching hashes. The other five files retained the
original generator/verifier records. Original attestation SHA-256 is
`1d7ff8f4ae72a1b140833d6108c43854fb356cbec2bfd3ba1bd3fd212efff72c`.
The source commit/tree/main and clean before/after metadata matched cc52.
Every source-snapshot file matched its Git blob, and `source/spike.tar` matched
`git archive` of cc52 byte-for-byte. Both actual retained ELF files matched.
The stored verifier command selects exact cc52/tree/main and records exit 0;
its stdout SHA-256 is
`5816d581e29050c2c2fc9a048b0565436ae204abc5780ec2523137648bb75374`.

All 22 files in the manual package matched their individual sizes and hashes,
including gzip-encoded archive and retained ELF. No attestation.json was present;
the observation explicitly says formal attestation NOT_RUN. Its actual archive
and retained ELF matched the formal package. Both manual build descriptors
reported the same retained ELF hash and exit 0; the manual package retains one
ELF binary, not two separately retained binaries. EL1 and EL2 serial/stderr
digests, binary and QEMU before/after digests, result-file hashes, exit codes and
no-timeout observations were checked in both packages. Cross-package archive,
ELF and serial byte comparisons matched; the summaries contain those measured
digests without publishing any raw artifact or operational path.

`raw_receipt_rechecked_here=false` remains a property of the supplemental
metadata checker: it does not run the formal receipt verifier or reexecute a
build/guest. The separate derivation above checks received artifact bytes and
the stored original verifier record. It does not claim a newly executed formal
verifier result, a signed proof of execution or formal manual-environment
acceptance. No encoded raw package is included in this source candidate.

Derived formal summary SHA-256:
`35adb7c4c87814650c712afa0846575d05872a79e6d02b8299098ed0a0be84d0`.
Derived manual summary SHA-256:
`c2cb41ace14a40f5d521afad7d86c3a88a0cc84e3e205113d8ab5f8e14e14333`.

Measured records commit `3bddcdcfeb7dd43336e56595cedde58dce50af5a`, tree
`b7983fa00ea6d2ebe607674e9f3844c2c6b1ea2e`, passed the unchanged supplemental
validator with exit 0, COMPLETE_METADATA_ONLY and no missing input. Formal
verification remained REPORTED_VERIFIED, manual observation remained
DIGESTS_MATCH_NO_FORMAL_RECEIPT, and Phase C remained PENDING. Sixty-nine tests
were rerun: 18 supplemental positive/negative/CLI tests, 8 source-scope tests,
11 runtime-summary tests and 32 runner/receipt fixtures. No implementation,
schema or assertion was changed to accept the measured summaries. The final
documentation-only child records these results; no runtime was reexecuted.
