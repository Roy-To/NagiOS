# Attestation source scope separation candidate

Status: PARTIAL. This is a new source-preparation branch, not a replacement
receipt and not an adoption of the proposed composition contract.

## Why exact e6 does not meet the existing receipt scope

The independent execution owner reports two builds with identical ELFs and
EL1 success / expected EL2 rejection for exact source
`e6c2c08860df75c84a85c31151e8c32b2613433b`, tree
`4cafbdc218e87954cf5ef20c6779f2cd67a0d3ae`. The formal receipt verifier then
exited 2: `source commit contains out-of-scope paths:
docs/implementation_status.md`. This task did not receive or recheck the raw
receipt, archive, ELFs or runs. Build/run success and receipt verification
failure remain separate results.

The unmodified `verify_attestation.py:check_source` permits only
`docs/porting/multiarch/`, `experiments/nagi-aarch64-spike/` and
`experiments/nagi-multiarch-contracts/` in the net diff from the explicit main
baseline to the attested source. It checks exact source/main/tree identities,
ancestry, the generator/runner identities and source-clean/build-bound metadata.
Later records-only children may change only `docs/porting/multiarch/`.
e6 changes the root status document outside these prefixes. The original
generator's initial source validation does not perform this final scope check,
so successful generation/build/run can precede this formal rejection.

Using b50 as the main would not remove the root-document delta. Substituting
e6 as the main would redefine the fixed-main claim, and using current main87
would fail ancestry for e6. Relabeling a source0039 receipt as an e6 receipt
fails exact source/tree binding. None is a repair of the original e6 receipt.
The new composition-integrity contract keeps ownership adoption PENDING and
does not amend the formal receipt contract.

## Bounded source preparation

This branch starts at e6 and restores only `docs/implementation_status.md` to
the exact fixed main036 blob. That blob is also byte-identical at public parent
b50. It adds this source-scope note and eight source-contract regression tests
under the already permitted document prefix. It leaves the entire spike and
contracts experiment subtrees identical to e6. No verifier prefix, signature,
schema, source-bound assertion or original evidence is changed. The source
note remains on the original e6/composition branches for separate delivery;
the root status prepend is deliberately not included in this execution source.

This is explicitly a different commit and full tree. An identical spike subtree
does not make an e6 receipt valid for this source. `git archive` also binds its
source commit: archive hashes, source-snapshot metadata and build/run receipts
must not be retroactively rewritten. Any formal PASS requires a fresh complete
attestation run at the new exact commit/tree with fixed main036 and a new output
directory, followed by the unchanged formal verifier. The reported original
e6 attempt remains a formal verification failure.

## Regression checks and decisions

`test-attestation-source-scope.py` calls the original `check_source` on synthetic
source metadata in isolated Git fixtures; it never generates a receipt or claims
build/run acceptance. It verifies that a root-note source still rejects, a new
source with the root status restored meets the source contract, identical spike
subtrees do not permit source/tree rebinding, root-status changes cannot be
attached as a records-only child, owned document children retain source binding,
unrelated kernel changes reject and changing the recorded main cannot hide the
scope failure. Run:

```sh
python3 -B docs/porting/multiarch/test-attestation-source-scope.py -v
```

The current authorization permits preparing this isolated, narrower source and
testing it; it does not grant adoption or publication. Recommended next step:
independently review this distinct source and, if selected, perform a fresh
source-bound run. If an exact-e6 formal receipt is mandatory, the owner must
explicitly adopt a versioned contract that defines and validates the root status
exception; the pending composition proposal alone is insufficient. Neither
choice may turn the existing failed receipt verification into PASS by changing
labels, expected SHAs or the main baseline.

Phase C stays PENDING; physical hardware and production Nagi ARM64 stay NOT_RUN.
The old source0039 records and reviewed e6/composition candidates remain intact.
No public operation, push, merge, dispatch resend, media write or network
configuration change was performed.
