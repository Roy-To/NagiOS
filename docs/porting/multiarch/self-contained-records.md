# Offline review composition proposal

This single-parent packaging candidate extends reviewed records head
`2b394ca5e0795ae1b5a355f5e48fd9d02791d5cc`. It preserves that entire tree,
adds the four missing composition files byte-and-mode-exact from
`482cac3f13ca7ad78f37bf98c67a4a16ef004351`, and adds seven packaging files.
It does not claim 482 or dc are ancestors of this head.

`review-dependencies.bundle` contains the six missing tracked-source commits
on the 482 and dc histories, excluding objects reachable from 2b. Its exact
size, SHA-256, advertised heads and commit/tree identities are pinned in
`restore-review-objects.py` and `review-dependencies.json`. Its prerequisites
b50 and e6 are ancestors of 2b. A full-history clone of the packaging candidate
therefore supplies all prerequisites without auxiliary public refs.
The bundle contains reviewed tracked code, documentation and bounded records;
it does not add external private receipts, ELF, source archives or raw runs.

From a clean full-history checkout, run:

```sh
python3 -B docs/porting/multiarch/verify-self-contained-records.py --regressions
```

The bootstrap checks the bundle before importing objects with `git bundle
unbundle`. It checks that HEAD and all refs remain unchanged. It uses no network
and creates no merge commit. The validator checks the exact tree composition,
then runs unchanged validators in detached temporary checkouts of 2b and 482.
The 482 validator also checks the exact dc historical snapshot. Importing objects
does not make these snapshots the current source or create execution evidence.

The 2b records remain bound to cc52 source. Their formal receipt is
REPORTED_VERIFIED; this metadata checker does not rerun its raw receipt verifier.
The other environment remains a manual observation with no formal receipt.
The 482 records remain bound to their original e6 composition and source0039
historical execution. Root implementation_status.md remains exactly the 2b
version; the 482 prepend exists only in the fixed historical snapshot.
All old schemas, source anchors and evidence remain unchanged.
Phase C is PENDING; physical and production ARM64 remain NOT_RUN.

## CI adoption and remaining gates

`proposed-records-ci.yml` documents a dedicated read-only Ubuntu job using
full-history checkout and the command above. It is not an active workflow.
Owner adoption of a workflow under `.github/` is still required; it would also
need an explicit packaging path contract update because this validator permits
only the seven listed packaging additions. Do not silently widen that contract.
Existing receipt source/descendant scopes remain unchanged: run their verifiers
on the fixed snapshots, not on this packaging tree or a future PR merge commit.

This proposal needs independent review of the new bootstrap, tree contract and
CI adoption plan. It does not resolve owner scope adoption, replace a missing
manual formal receipt, attest a new run or authorize public operations.
