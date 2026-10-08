# Stage 1: host foundations adoption

Explicit current user authorization covers host implementation, fixes, tests,
commit/push, separate PR and merge after applicable checks pass. The source
branch is preserved. This checkpoint uses `codex/0.2-adopt-host-foundations`
and its own registry row/State, based on main
`1118f13dc6badf19b10a628eda244f8a0a635237`.

Source checkpoint: `c09572d182823d2c293a6a1885b6073c5878d0e6`. Adopt only
the existing fingerprint/Legal source, their docs, source owner State snapshots,
five feature/app registrations plus original checkpoint row, and the minimal
shared CLI/launcher/schema/test connections. Historical proposal drafts and
old CI exclusions are omitted. App specifications already exist on current main
and are retained. All current registry rows and other owner States are retained.

Current main's newer shared CLI, bootstrap dependencies/locks, POSIX toolchain
selection and 0.1 command surface are retained. The sole three-way conflict is
the help command list: keep all current commands and add fingerprint/Legal.
No root Cargo files or dependency locks are changed. The existing Nagi CI
workflow retains all current gates and steps. It adds standalone Legal
format/lint/tests before the full-workspace tests, priming Legal’s separate
locked dependencies before offline shared-CLI delegation. Its full host and
applicable 0.1 target regression gates remain active. A separate scoped Ubuntu/Windows host workflow
adds registration, fingerprint, Legal and direct PowerShell JSON/error checks.
Unrelated PRs keep the existing CI and do not acquire a new registration gate.

Fingerprint source and Legal algorithms are reused, not reimplemented. Source
owner States are immutable adoption evidence; current adoption facts belong
only to `.dev/workstreams/host-foundations-adoption/state.json`. The library
and binary share fingerprint dispatch; Legal preserves subprocess stdout,
stderr and exit codes using its standalone locked offline manifest.

Stage 1 excludes Calendar implementation. Kernel/Loader/init, protected
third-party sources, 0.1 Acceptance and Hark branches are unchanged. M30 is
PARTIAL; host PASS does not activate 0.2 guest/runtime, claim release success,
or start 0.3. Stage 2 has a separate branch, PR, CI and State.

Current-main compatibility repair: the NOTICE table now explicitly writes
`rust-std (Rust std)` and `mesa-softpipe (Mesa Softpipe)`. The inherited parser
normalized the entire cell and missed both identities. Recognize the leading
key and complete parenthesized label as exact review aliases. Regression
coverage verifies both entries remain manual-review/unknown-license records
and that the shorter unreviewed name `Mesa` still fails the evidence gate.
No NOTICE, source lock, upstream source or legal-policy criterion is changed.

The first full Ubuntu CI run (37785415269, job 113338787927) passed all 293
CLI library tests but failed two delegated CLI tests because `serde_spanned
v0.6.7` was not cached and Legal deliberately runs offline. Standalone
Legal Clippy resolves its pinned graph before offline Legal/CLI tests in both
existing host jobs; no test, assertion, offline CLI policy or target gate is
removed. Dedicated host workflow 37785490729 passed both operating systems
on source de73b115e8024cadeca25bd094340dcda78500fb.

Adoption ownership CI compares the checkpoint against its actual merge base
with current `origin/main`. This keeps the host-only allowed/forbidden checks
accurate if the 0.1 line advances; all protected paths remain fail-closed.
Source-owner branches keep their original immutable activation baselines.
Before merge, recheck current main, the PR diff and CI base; no old branch is
unconditionally merged. Legal README documents one-time locked Cargo package
preparation while preserving offline tool execution and evidence rules.

Host acceptance evidence on `8be5e5598f013a13d539f0e25f2862e4c3e9de8b`: 137
focused tests and Registry/State checks PASS. Dedicated Ubuntu/Windows host
CI [37786245593](https://github.com/RT-NISH/NagiOS/actions/runs/37786245593)
and both original full host CI jobs PASS. Existing M20 real Granite inference
also PASS. Full target CI passed M17, M18, isolated process, M19 Search/Files
and Undo, then M27 failed because QEMU VNC port 39419 was already in use;
QMP reset followed. M29/M30 did not run. This is not full target PASS.
Retry all applicable CI on a fresh runner at the latest checkpoint; change
no 0.1 source, assertion or validation threshold.
[PR #34](https://github.com/RT-NISH/NagiOS/pull/34) must wait for every latest
applicable check to PASS before merge. Review completed; source-owner States
are immutable. Host acceptance State and PR merge/regression gate are explicit.
