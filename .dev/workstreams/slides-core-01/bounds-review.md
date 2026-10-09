# Duplicate bounds review correction

The independent review found a real SOURCE blocker: DuplicateSlide counted zero
batch payload bytes and cloned the source plus an AddSlide copy before checking
the final snapshot. Many copies could therefore exceed the snapshot bound in
memory. The prior 21-test Host PASS claim was withdrawn pending correction.

`duplicate-bounds-red.txt` records a small-budget failing regression on the legacy
edit path: duplicate then delete succeeded although the intermediate snapshot
exceeded the cap. This run included new counter helpers but had not connected
the edit preflight. No large-allocation reproduction was performed.

The corrected path uses the native writer in count-only mode to measure exact
wire sizes without content cloning or an output buffer. Before every edit,
checked removal/addition accounting includes new 8-byte issued-ID tombstones and
rejects any intermediate snapshot above max_bytes. Duplicate resolves its source
from the current candidate (including earlier edits), charges its full native
size to the cumulative batch budget, checks index/count/IDs, clones once, remaps
internal targets, and moves that copy into the candidate. Final full validation
still enforces structural consistency and atomic commit. Failed accounting cannot
mutate the original snapshot, revision or issued IDs.

The 4 new small-budget acceptance cases cover: duplicate/delete intermediate
oversize; 8 duplicate/delete cycles with each intermediate fitting but cumulative
copied bytes exceeding budget; exact-wire-limit success and one-byte-less error;
and a source created/edited earlier in the same batch. Existing 21 tests remain.
The corrected 25-test/fmt/Clippy raw run is `duplicate-bounds-verification.txt`;
post-commit verification and exact corrected SHA belong to State.

Publication remains on hold. Shared registry/root Cargo/lock/CI/ABI/main and
runtime are unchanged. M30 PASS plus a separate runtime checkpoint remains required.
