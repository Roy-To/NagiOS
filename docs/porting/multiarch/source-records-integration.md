# ARM host source and fixed-source records integration preflight

Status: PARTIAL, local composition only. Phase C remains PENDING. Physical
hardware remains NOT_RUN. This is not a publication-ready records checkpoint.

## Exact input reconstruction

Repository: `https://github.com/Roy-To/NagiOS.git`.
Both supplied transfers were decoded and verified before applying them to an
isolated checkout. `git apply --index` followed by `git write-tree` matched each
expected tree. The original supplied commit-object bytes were written with
`git hash-object -t commit -w --stdin`, reproducing both original commit IDs.
No author/committer metadata was synthesized and no `git am` was used.

| Input | Commit | Tree |
|---|---|---|
| Public parent | `b50e8ff181efcf5093c928472fc729d5c9e231cd` | `d7bb5a69687e0c604791c277008d60635dddab15` |
| First records transfer | `334cdc530bfe2ec139283defac9f4cbd685fa198` | `8d686c2f256d17297f4d49a63a88f89cd0636a7e` |
| Repaired records transfer | `dc11f55d934ae73f508858458a09a3994889520c` | `26beb4905c44132d63a6db377e23da0521448ab3` |
| Reviewed ARM host source | `e6c2c08860df75c84a85c31151e8c32b2613433b` | `4cafbdc218e87954cf5ef20c6779f2cd67a0d3ae` |

First-transfer base64 length 15,368 (chunks 6,000/6,000/3,368):

- gzip: 11,526 bytes, SHA-256
  `650de22c9270f04bb1f8049b4dd503f3a20240fbb261459a3de59ea247d3e71d`.
- patch: 46,655 bytes, SHA-256
  `477126a07f02df6c54cbeda34120c8f6d50c228929de277b512953884057d8a6`.

Second-transfer base64 length 10,864 (chunks 6,000/4,864):

- gzip: 8,147 bytes, SHA-256
  `0a1f9eace29e24b9ace5c9dd0f5b905e4fcf7bad65e9a22255a1766f24aec9b4`.
- patch: 26,796 bytes, SHA-256
  `d2a7582c08b6b56b1b29dd991c9302ed4641f4b50cff7601c87f6fded8ae5d42`.

Every supplied size and digest matched. The first records commit has the public
head as its parent; the repaired records commit has the first records commit as
its parent. The ARM host candidate has `d5bd77b9` as its parent, whose parent is
that same public head. Original candidate worktrees and commits are retained.

## Local composition and checks

Composition commit `6e3bf6564440fdc226aa24d819701772ad4ea32b`, tree
`604f6e9a8db90283324f3f77f31c2eed615f3b5b`, has the ARM host candidate as its parent.
It applies only the exact public-parent-to-dc11 records diff. The 8 ARM-source
paths and 20 records paths have no intersection; there were no application
conflicts. Every contributed file equals its reviewed input byte-for-byte.
This document is a subsequent documentation-only handoff, not a new execution.

All 90 pre-existing files under `docs/porting/multiarch` other than the two
intentional dc11 narrative/state updates are byte-preserved from the public
parent. `handoff.md` and `state.json` are taken exactly from dc11. No historical
checkpoint, original summary, serial log or evidence index was regenerated.

The restored dc11 candidate passed the unchanged complete records verifier:
579 source files, 13 authority files, 32 registry entries, 50 immutable historical
files. Its 10 backend-helper fixture tests also passed. The integrated source
snapshot then passed 209 host tests: 105 contracts, 30 audit/records, 16 BootInfo,
5 ABI, 32 runner/receipt, 10 backend-helper and 11 runtime-summary tests.
Repository-pinned formatting and warning-denied contracts Clippy passed.

The unchanged runtime-summary validator accepted both the original source0039
summary and the independent-cloud source0039 summary with their exact fixed
source/tree/main/subtree and serial/stderr digests. Substituting e6 commit/tree
for either summary was rejected with `runtime summary source/main identity
mismatch`. This is unsigned metadata consistency, not fresh guest execution.

The public refs were read again after these checks: PR head remains b50e8ff1;
main remains `87b7dea4061be2572526181ab559d63037f823d0`, tree
`72a8ef90aa92858c5140dea1793ae9d6f2f0e885`. A non-branch-mutating tree prediction
for composition commit versus main reported no conflicts, tree
`b362e881f5635015d2493259eefb38cfebaf8dd3`. No merge commit or run at that
predicted tree is implied. These observations do not resolve the outcome of
any prior external dispatch, and no dispatch was resent.

## Source boundary and remaining decisions

Both transferred execution summaries remain bound to source
`0039f0c3ad6df56fee57fcdeca7ea02bd1f9db13`, tree
`0b2597ae1b707c0a2fa8d42e16e42d46566d56c5`, spike subtree
`eb27c8571b5af7f5c252f010f04fc68baf348712`, fixed main
`0362427747e769b00558053e4c7f4c756d303802`. They are not execution results for
e6, this composition, or current main. The raw receipt's 33 files, source
archive, ELF and complete raw runs were not transferred or rechecked here.
Their recheck and actual QEMU runs remain the prior creator's report. No new
raw private artifacts are incorporated. Unsigned receipts cannot prove execution
against an actor able to change artifacts and metadata together.

The unchanged full records verifier rejects the composition with exit 2:
`records-only child changed non-record paths`. Its state remains intentionally
bound to source0039, and ARM host source changes are not records-only changes.
The separate unchanged owned-scope check also rejects
`docs/implementation_status.md` outside its declared prefixes. These are
repository validation gates, not permission-denial or network errors.

Before treating a combined branch as a new accepted checkpoint, the owners
must decide the explicitly bounded source-plus-historical-record representation
and the root status-document ownership. Keep the old fixed-source checkpoint
independently verifiable; do not loosen source binding or silently broaden scope
to make this composition pass. If runtime acceptance of the new source is
required, generate separately authorized, source-bound evidence for that exact
source and retain the original records. Formal Phase-C/schema adoption,
A/B owner/proposal acceptance and M30/D/E/registration gates remain pending.
Independent review of this composition and its handoff is still required.

No production Nagi ARM64, MMU/EL0, timer IRQ/GIC, PSCI/SMP, VirtIO/DMA, x86 guest
regression or physical hardware acceptance is claimed. No public operation,
push, merge, dispatch resend, hardware/media write or network configuration
change was performed.

## Proposed explicit composition contract (not owner adoption)

A new, separate `nagi-multiarch-source-record-composition/1` manifest is in
`source-records-composition.json`. It distinguishes the reviewed current source
e6 from the immutable historical records commit dc11 and that record's source0039
runtime. It does not change any earlier schema, record or validator. The
historical `state.json` stays byte-identical to dc11; its `last_verified` is not
repurposed to represent current-source verification.

The proposed composition integrity checker requires full base/source/records
SHAs on its command line independently of the manifest. Each anchor's tree is
verified. The tracked tree must be the exact, nonoverlapping overlay of the two
pinned input deltas, plus four specifically named composition metadata/tool
files. File content, file set and modes are compared. Existing schema and legacy
verifier changes are prohibited. It executes the original historical verifier
in a temporary detached worktree at dc11, with the original main and evidence
directory, then removes that owned checkout. This preserves the old verifier's
records-only and source-binding rules; failure propagates instead of being
ignored. It never presents the integrated head as a records-only dc11 child.

The sole root-document exception is the exact `docs/implementation_status.md`
blob from reviewed source e6, including its exact base/source SHA-256 values.
Its only permitted content change is a nonempty PR57-prefixed prepend; every
byte of the public-parent document must remain the unchanged suffix. No other
root document, kernel path or registry path is added to the allowed scope.
The exception expresses an inspectable ownership proposal, not permission:
`scope_owner` is `Integration Owner/os-core`, and `owner_scope_adoption` must be
`PENDING`. Substituting `PASS` or silently granting ownership is rejected.

Run from a clean committed checkout with Python 3 and local Git objects:

```sh
python3 -B docs/porting/multiarch/test-integration-records.py -v
python3 -B docs/porting/multiarch/verify-integration.py \
  --base b50e8ff181efcf5093c928472fc729d5c9e231cd \
  --source e6c2c08860df75c84a85c31151e8c32b2613433b \
  --records dc11f55d934ae73f508858458a09a3994889520c
```

The 17 focused tests exercise exact composition without acceptance promotion,
bad commit/tree anchors, short SHAs, dirty/untracked inputs, changed/deleted
historical evidence, unrelated tracked files, source content/mode changes,
root status hash/content changes, rewriting the old root-document suffix even
with updated anchors/hashes, promotion of runtime/Phase C/hardware/ownership,
historical-to-current source rebinding, wrong subtree, unsafe/duplicate summary
paths, kernel-scope expansion, replacement of the old verifier and historical
verifier rejection with temporary-worktree cleanup. The tiny historical
verifier in these fixtures tests orchestration only; the actual composition
invokes the original full verifier from dc11.

Policy decision remains explicit: adopt this separate composition-integrity
contract plus the exact root-document ownership proposal after independent
review, or keep e6 and dc11 as separate delivery branches under the original
records-only contract. The former permits combining reviewed inputs without
inventing runtime evidence, but introduces a new validator/manifest that needs
review. The latter preserves the existing single-source contract unchanged,
but does not accept one combined checkpoint. Neither option proves e6 runtime
execution or authorizes publication. Current-source runtime, physical and
Phase C remain respectively NOT_RUN, NOT_RUN and PENDING in the new manifest.

Validation at code commit `c57b633cbef2e63e01f5c47ca74c10b9d81e4970`, tree
`7ab9ef1edfc00cec96a4d394ce0db73d7d4e2312`: the actual composition command
returned integrity PASS, historical verifier PASS and exact reviewed prepend,
while runtime/physical remained NOT_RUN and Phase C/ownership remained PENDING.
The 17 new tests plus all 209 existing host tests passed, with pinned format and
warning-denied Clippy. Experiment source and the root status blob remain exactly
e6; historical records/state/checkpoints and old schemas remain exactly dc11.
This subsequent results entry is documentation only. No guest run or acceptance
promotion was performed.
