# Supplemental cc52 runtime metadata candidate

Status: PENDING_INPUT. Phase C remains PENDING. This additional record does not
replace the source0039 records or any previous failed e6 attempt.

The independent comparison owner's report states that exact cc52's original
formal receipt passed the original verifier with 33 referenced artifacts and
exit 0. A different environment matched the source archive, ELF and EL1/EL2
serial artifacts by manual observation without generating a formal receipt.
These are recorded as owner-reported observations. This environment has not
received the public-summary bytes or measured hashes, and does not revalidate
the private raw receipt/artifacts or rerun QEMU.

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

The two summary references are deliberately null. No unknown hash is inferred
from matching source code, old ELF expectations or expected serial strings.
No serial or raw receipt is synthesized. Qualitative owner reports remain
distinct from complete hash-bound public metadata.

When the authorized measured public-summary bytes become available, retain
this pending record as history and prepare a new reviewed records commit with
the fixed summary references and SHA-256 values. Do not overwrite old runtime
records or rewrite any receipt. The required summaries are:

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
complete, consistent metadata, while runtime adoption remains pending. Sixteen
isolated synthetic regression tests cover complete/pending metadata, wrong
source/tree/main, changed summary bytes, failed or incomplete formal checks,
manual PASS/receipt injection, invalid/mismatching digests, phase promotion,
false raw-recheck claims, historical artifact mutation, invented serial, root
status changes and summary path escapes. No fixture is published as real
execution evidence.

The remaining input is the two authorized public summaries with actual measured
hashes, not another run of the already verified original cc52 receipt. Any
formal-receipt claim for the separate manual environment would require its own
new receipt; this candidate makes no such claim. The new supplemental contract
and completed record still need independent review/owner adoption. No public
operation, push, merge, dispatch resend, physical/media write or network change
was performed.
