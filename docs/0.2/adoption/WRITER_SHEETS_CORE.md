# Writer / Sheets host adoption preparation

Checkpoint: `WS-HOST-PREP-20261008`, workstream `writer-sheets-core-adoption`.
The explicit user OK at 2026-10-08 21:48:44 UTC authorizes dedicated branch
push, draft PR creation and Windows/Linux CI verification. Main merge and
main/owner branch writes remain unauthorized. This is a bounded host checkpoint;
no runtime/provider/GUI/0.3 activation, release or product acceptance.

Original preparation base: merged Calendar main
`cb3f9f29552bdee3e9c99228cb2153d10a1634e9`. Current publication base:
`edad2e7a87aa0fe08982a5c5249bc4a3ad7de7d4`, after merged Hark PR #31. Latest main was
merged into this adoption branch without conflicts; its source and State are
retained unchanged. The audit compares against the current main merge base.
Reviewed source owner heads:

- Writer: `claude/0.2-writer-core-01` at
  `8cc5c5da04f622be95c79152bf709edb61272c59`, original 31 tests, W1-W5.
- Sheets: `claude/0.2-sheets-calc-01` at
  `b6e6efe13018cae4f3de188bb387e0584994609f`, original 19 tests, S1-S5.

Remote/main/PR/Registry audit found no competing Writer/Sheets adoption;
the original sole open PR was Hark #31, now merged. Publication re-audit found
no open PR or remote adoption branch competing with this checkpoint. The owner branches and the existing `work`
checkout remain untouched. Repository AGENTS, relevant 0.1/0.2 sections,
workstreams, Registry and owner/adoption State were inspected. No relevant
`.agents/skills/*/SKILL.md` exists in this cloud checkout or source trees;
`/workspace/.agents` is empty. Existing workstream MD proposal wording predates
the live approved Registry; preserve it and follow current registration.

Only owner cores/tests and immutable owner evidence are imported. Keep all
existing Registry rows and other owner evidence intact; add one local checkpoint
row. Current adoption facts belong exclusively to its own State/evidence.
Writer's source owner PASS and Sheets' source owner PARTIAL are historical
facts, preserved byte-for-byte even if adoption later passes Windows CI.

Writer replaces historical Git pins with current repository path dependencies
on canonical `nagi-model` / `nagi-history` 0.1.0. The model changed only by
additive ID Ord/Hash derives. Writer's used history Activity contract is unchanged;
current history has additive guest APIs and pinned sha2 0.10.9. Regenerate only
Writer's standalone lock. Sheets already uses canonical model by path; retain
its lock. No root Cargo, history source, model source, algorithms or original
acceptance tests change. DocumentId remains the explicit ObjectId role adapter;
local RevisionId semantics and deferred global allocation are unchanged.

Calendar already uses package-scoped fmt on main. Its command stays unchanged.
For Writer/Sheets only, remove `--all` in the dedicated workflow and Writer's
local gate. Verbose rustfmt must still include every owned module, example and
acceptance target while excluding the parent workspace/third_party. Clippy
remains all-target warnings-denied; tests remain locked/offline after package
preparation. Extend only the existing dedicated workflow's branch/path routing
and adoption merge-base audit for this branch. Existing `ci.yml` remains intact.

## Local verification and resumption

Use pinned `nightly-2025-08-01`. This cloud environment stores it under
`/workspace/.nagi-cargo` and `/workspace/.nagi-rustup`; add the former's `bin`
to PATH and set CARGO_HOME/RUSTUP_HOME accordingly. Dependencies are prepared
once with locked `cargo fetch`; subsequent gates run offline.

```sh
git status --short --branch
./nagi dev status
./nagi dev resume
./nagi dev verify
bash tests/writer-core/verify.sh
cargo fmt --manifest-path crates/nagi-sheets-core/Cargo.toml -- --check
cargo clippy --manifest-path crates/nagi-sheets-core/Cargo.toml --all-targets --locked --offline -- -D warnings
cargo test --manifest-path crates/nagi-sheets-core/Cargo.toml --locked --offline
python3 .dev/workstreams/writer-sheets-core-adoption/verify_adoption.py
git diff --check
```

The audit requires Python jsonschema and PyYAML, available in this environment.
Run Calendar's 61 original tests and shared CLI/Legal compatibility checks too.
Full raw output and exact-source evidence are retained in the checkpoint's
`verification.json`, with source preservation digests in `adoption-audit.json`.
Linux CRLF checkout tests are formatting regression evidence only, never native
Windows runtime acceptance.

## Next Integration Owner action

1. Read checkpoint proposal, State and verification evidence; inspect the diff
   against current main and the Writer standalone dependency graph.
2. Recheck current main, source owner heads, open PRs and Registry for competing
   adoption. Rebase/reprepare only this local integration branch if needed;
   rerun affected checks and preserve immutable owner evidence.
3. Publication authorization is recorded above. Use only the dedicated
   adoption branch and keep the PR draft; record exact published SHA and CI.
4. Publish this dedicated branch and create a separate
   host adoption PR. Require its exact latest source to pass dedicated
   Ubuntu/Windows CI (Writer 31, Sheets 19, Calendar 61) and every applicable
   existing host/target gate required by the unchanged CI and review policy.
5. Record CI in adoption State only. Main merge requires explicit approval
   and all applicable checks PASS; never infer it from local host success.

M30 remains PARTIAL. Guest persistence, Activity/Wayback/Search providers,
permission/session enforcement wiring, GUI, PDF/DOCX/XLSX/CSV, agent edits,
global revision allocation and 0.3 remain separate gates. A missing generated
mozjs-sys source initially caused one existing broad CLI test to fail.
The follow-up authorizes bounded dependency preparation: existing component
bootstrap reused cached mozjs_sys 153.0.0-2, whose archive SHA-256 and all
10,454 source files were verified, then applied registered patches and validated
the generated marker. Additional ignored checkout occupies 174 MiB; download
was zero and no Servo/bootstrap dependencies were fetched. The exact unmodified
bootstrap modules were called through a temporary harness because the public
`./nagi fetch` command also prepares Servo and other large components.
Full CLI now passes 293 library + 32 launcher tests. Raw commands, harness,
capacity/hash checks and logs are in `bootstrap-verification.json`. Initial
failures remain recorded as historical evidence. No test is skipped or weakened.
Native Windows/remote CI and all applicable existing CI must be checked on
the latest published source before this draft can be considered for a separate
merge decision. Current results belong to adoption State, never owner State.
