# Third-Party App DX Implementation Plan

## Scope and guardrails

Deliver an out-of-tree Rust `Hello Nagi` sample, public SDK dependency verification, and host manifest validation. Do not change runtime, guest installation/launch, authority enforcement, target code, release gates, CI, root Cargo workspace/lockfiles, or unrelated workstreams. Never copy private specifications into this public repository. Keep 0.1 M30 and 0.2 activation status unchanged unless authoritative public evidence proves otherwise.

The public `app-sdk-contract` host-side `PASS` must not be described as a target runtime `PASS`. The 0.1 M30 and 0.2 gates remain closed in this task.

## Steps

1. **Integration Owner registration checkpoint**: add `third-party-app-dx` to `.dev/workstreams.json` with owner, branch, exact allowed paths, and host-only gate; create `.dev/workstreams/third-party-app-dx/state.json` before implementation. Record that the checkpoint authorizes only sample build, manifest validation, and docs. Do not touch root shared manifests, CI, schemas, or implementation status.
2. **Sample and validator regression**: adapt the existing standalone `samples/hello-nagi` to pin `nagi-sdk` to the public Git revision; add a third-party manifest; extend `tools/nagi-pkg/src/contract_tests.rs` with a failing-first regression test for successful validation and third-party metadata; build the sample and validate its manifest.
3. **Developer guidance**: add bilingual out-of-tree quick-start instructions and link them from the SDK README. Include the public revision, cargo build/tree commands, package manifest command, and the host-vs-runtime/gate limitations.
4. **Evidence and state**: record exact commands, outcomes, commit SHA/revision, and remaining gates in workstream state. Keep M21 and M30 partial; claim only host evidence.
5. **Review and delivery**: inspect the final diff and allowed paths; run focused package tests, sample build, CLI manifest validation, `cargo tree` source check, and `git diff --check`; commit and push the dedicated workstream branch; verify remote branch SHA. Avoid broad CI runs unless the focused evidence exposes a reason.

## TDD sequence

For the manifest regression, add the test first and run it to demonstrate the expected failure. Then add the manifest and only the minimal implementation changes needed for it to pass. Run the focused suite after implementation.

## Ownership and file boundaries

- Integration Owner checkpoint: `.dev/workstreams.json` and `.dev/workstreams/third-party-app-dx/state.json`.
- Sample/validator: `samples/hello-nagi/**` and `tools/nagi-pkg/src/contract_tests.rs`.
- Guidance: `sdk/README.md` and `sdk/rust/docs/THIRD_PARTY_QUICKSTART.md`.
- Planning evidence: this design and plan under `docs/superpowers/`.

No other paths are in scope.
