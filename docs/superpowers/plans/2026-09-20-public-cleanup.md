# Public Repository Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prepare the current Nagi M17 worktree for a later human-controlled public-release audit without changing M17 behavior, rewriting Git history, or changing GitHub settings.

**Architecture:** Keep the Nagi runtime, target build, Servo adapter, Mesa path, and acceptance scripts unchanged. Remove only generated artifacts from the current index, add nested Cargo-target protection, sanitize unnecessary machine-specific documentation paths, and add public-facing license-status, security-reporting, and third-party inventory documentation without inventing Nagi's own license.

**Tech Stack:** Git, PowerShell, Markdown, Cargo metadata, Rust formatting checks, existing Nagi CI and source-lock conventions.

## Global Constraints

- M17 remains `BLOCKED`; M18 remains `NOT STARTED`.
- Do not modify M17 runtime behavior, target build logic, Servo integration, Mesa integration, or acceptance behavior.
- Do not run history rewrite, author metadata rewrite, branch deletion, merge, rebase, cherry-pick, force push, visibility change, repository transfer, or GitHub settings mutation.
- Preserve the current branch `codex/m17-servo-bootstrap` and its starting HEAD `8c6b876` until the cleanup commit is created locally.
- Do not choose a Nagi root license. State that the license is not finalized and keep third-party licenses separate.
- Preserve pinned third-party revisions, source hashes, patch boundaries, and generated-source policy.

---

### Task 1: Remove tracked generated artifacts and protect nested targets

**Files:**
- Modify: `.gitignore`
- Remove from index: `samples/hello-nagi/target/`

**Interfaces:**
- Produces a source-only index for the sample build output while leaving local generated files available for development if needed.

- [ ] Add a repository-wide `**/target/` ignore rule while retaining the existing explicit generated-output rules and not ignoring authoritative source, fixtures, or third-party patch files.
- [ ] Remove only `samples/hello-nagi/target/` from Git tracking; do not delete unrelated source or generated third-party checkouts.
- [ ] Verify `git ls-files samples/hello-nagi/target` returns no paths.
- [ ] Verify `git check-ignore -v samples/hello-nagi/target/debug/example` reports the new nested-target rule.

### Task 2: Sanitize unnecessary local paths in tracked documentation

**Files:**
- Modify: `docs/superpowers/plans/2026-09-18-nagi-boot-sequence.md`
- Modify: `docs/superpowers/plans/2026-09-19-ci-normalization-plan.md`
- Modify: `docs/superpowers/plans/2026-09-19-m17-servo-bootstrap-plan.md`
- Modify: `m17-rendering-investigation.md`

**Interfaces:**
- Keeps reproducible command intent and CI evidence while replacing user-specific paths with neutral placeholders such as `<repo>`, `<worktree>`, `<temporary-checkout>`, and `<cargo-registry>`.

- [ ] Replace every unnecessary machine-specific user path and equivalent private worktree/cache path in the listed documents.
- [ ] Keep GitHub-hosted runner paths such as `/home/runner/work/...` when they are needed as CI evidence.
- [ ] Verify the tracked non-artifact tree has no occurrences of a concrete local user home, private Desktop/worktree path, or private Cargo registry path.

### Task 3: Add third-party notice inventory without making unsupported license claims

**Files:**
- Create: `THIRD_PARTY_NOTICES.md`

**Interfaces:**
- Documents the pinned components, source-lock locations, Nagi patch boundaries, declared license metadata, and components that remain subject to human/legal review.

- [ ] Record Servo, Surfman, Mesa Softpipe, relibc, Rust std/source patching, Tokio, mio, socket2, hyper-util, freetype-sys, libc, smoltcp, and aws-lc/aws-lc-sys.
- [ ] Link each entry to `third_party/sources.lock`, the tracked vendor/license location when present, or the pinned upstream repository.
- [ ] Mark Mesa component notices, Rust std redistribution metadata, aws-lc-sys bundled source, libz-sys and other transitive C sources as `UNKNOWN / HUMAN REVIEW` where the repository does not provide a complete notice.
- [ ] Do not state that Nagi itself is MIT, Apache, MPL, GPL, or otherwise licensed.

### Task 4: Add public-facing README and security guidance

**Files:**
- Modify: `README.md`
- Create: `SECURITY.md`

**Interfaces:**
- Makes the repository status, experimental scope, reference environment, license status, third-party notice location, and responsible security-reporting boundary clear to public readers.

- [ ] Preserve the existing developer workflow and add only concise public-facing sections.
- [ ] State that Nagi OS is a Developer Preview and that M17 Servo Bootstrap is currently `BLOCKED`; do not claim first-web-pixel acceptance.
- [ ] State that Nagi's own license is not finalized and link to `THIRD_PARTY_NOTICES.md`.
- [ ] Explain that security reports should not include credentials or secrets and should use a private owner-controlled channel; do not invent a public contact address.

### Task 5: Validate the cleanup boundary

**Files:**
- No additional source changes.

- [ ] Run `git diff --check`.
- [ ] Run `cargo metadata --locked --no-deps`.
- [ ] Run `cargo fmt --check` using the repository's configured toolchain where available.
- [ ] Run focused tracked-artifact, local-path, binary, and high-confidence secret-pattern scans.
- [ ] Confirm M17 implementation files are unchanged outside the explicitly approved cleanup files.
- [ ] Record the final branch, before/after HEAD, worktree state, remaining historical copies, license status, and prohibited-action status in the handoff response.
