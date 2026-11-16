# Nagi OS Foundation Checklist Implementation Plan

> **For agentic workers:** Use read-only delegated evidence review; primary agent integrates all documentation edits.

**Goal:** Preserve the approved 35-item foundation inventory as a permanent public ledger without inventing technical decisions or acceptance.

**Architecture:** A management DEC records adoption; the checklist indexes product decisions, technical design and scoped implementation evidence separately. Existing specifications, ADRs and workstreams retain authority.

**Tech Stack:** Markdown, Git, GitHub pull request.

## Global constraints

- Public `Roy-To/NagiOS` is canonical; do not write to the retired private repository.
- Preserve IDs 1–35, six inventory groups, seven future discussion bundles and proposed P0/P1/P2 labels.
- Priorities are REVIEW NOTE / PROVISIONAL; no new version assignment, implementation authorization, release gate or performance commitment.
- Preserve unrelated changes. Main baseline: `029bec16c0e7befa6856c4bd43805d1662bf0f58`.
- Host, historical custom-kernel guest, unmerged PR and Linux target evidence remain distinct.

## Tasks

- [x] Read current DEC/specification/roadmap sources and unmerged Share/Handoff/Connectivity/Power/Sleep records; delegate independent implementation evidence audit.
- [x] Create `docs/product-decisions/NAGI_OS_FOUNDATION_CHECKLIST.md` with complete inventory, per-item fields, update rules, implementation contract and proposed first topics.
- [x] Add a uniquely numbered management DEC and update `docs/product-decisions/README.md` index; preserve existing technical decisions.
- [x] Check IDs/priorities/bundle coverage, relative links, prohibited data and whitespace; obtain read-only independent review and fix supported findings.
- [ ] Commit only task documents, push topic branch, open PR and verify remote SHA; report PR as unmerged.

## Execution method

Subagent-Driven Execution: read-only source audit and independent review are bounded tasks. Primary agent owns the ledger and index edits. Documentation checks are appropriate; no runtime tests or target PASS are claimed.
