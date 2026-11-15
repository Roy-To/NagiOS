# Kernel policy documentation alignment implementation plan

> **2026-11-15 公開移管:** [DEC-050](../../product-decisions/DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

> **For agentic workers:** Use subagent-driven-development for the bounded independent audit; primary agent owns all edits and integration.

**Goal:** Reconcile Private specifications and roadmaps with DEC-042/043/044 without changing release Gates or publishing private material.
**Architecture:** Preserve the historical implementation baseline; add explicit dated precedence, a Private 0.2 companion and one source-bound impact/evidence matrix. Separate confirmed policy, scoped development authorization, provisional technical choices and observed implementation.
**Tech Stack:** Markdown; authenticated GitHub connector and atomic Git data commit.

## Global constraints

- Destination: Private Roy-To/NagiOS-internal, existing main; no Public writes.
- No new checkout/worktree/clone, environment, build or VM operation.
- Existing workstream owner/path, SSD/Git, Files protection and unknown-result restrictions remain.
- No Linux target PASS, release activation, compatibility guarantee or migration version inferred.

## Task 1 — Source and impact audit

- [x] Read Private AGENTS, operating rules, index, DEC-042/043/044 and migration ADR at 9e0b185e2cf3fffe371437bf015e7035c672798b.
- [x] Inspect Public main 48b4eeb12a4a6674badd0a530149570ff4c30d30 implementation status, build/image/ABI inputs and workstream registry through read-only access.
- [x] Complete independent audit of 0.2 duplicated entry points, 0.3 roadmap and release/ownership Gates.

## Task 2 — Reconcile Private documents

- [x] Add docs/product-decisions/KERNEL-TRANSITION-ALIGNMENT-2026-11-13.md with source refs, impact matrix, evidence limits and next owner handoff.
- [x] Add docs/Nagi_OS_0.2_Codex_Implementation_Spec.md as a clearly scoped Private companion; retain Public 0.2 acceptance without mirroring its whole specification.
- [x] Link the impact report and companion from Private AGENTS, 0.1 specification, implementation status and decision index.
- [x] Add dated kernel/authorization notes to the Private 0.3, 0.4+ and 0.5–1.0 roadmaps and their index.
- [x] Correct stale summary wording in DEC-017 to acknowledge DEC-043/044 while preserving history.

## Task 3 — Validate and save

Checklist snapshot: independent candidate review complete, no Critical/Important findings. Publication and post-publication verification below follow this snapshot; the final execution report records their actual outcome.

- [x] Check relative links against repository trees, unique edit markers, exact original-text preservation, source refs and docs-only allowlist.
- [x] Obtain independent review of draft; resolve substantive findings without changing product decisions.
- [ ] Recheck account, Private visibility and main head; create one atomic documentation commit and update main only against expected parent.
- [ ] Fetch committed files and compare exact contents; verify changed paths and Public head unchanged.
- [ ] Report verified Private commit and unperformed runtime validation.
