# Codex — Nagi product-decision handoff

**Status:** ACTIVE / 2026-11-15 (rewritten by [DEC-050](DEC-050-public-specification-repository.md))
**Source of truth:** `Roy-To/NagiOS/docs/product-decisions/` (this repository)

## Task startup checklist

1. Read the root `AGENTS.md` and the implementation specification.
2. For user-visible behavior, memory, policy, UI, cross-app Actions or roadmaps, read [OPERATING_RULES.md](OPERATING_RULES.md), [README.md](README.md) and the relevant `DEC-*.md` in this directory.
3. Distinguish `CONFIRMED` from `PROVISIONAL` and `REVIEW NOTE`.
4. When the user approves a new product decision during a Codex session, record it in this directory following [OPERATING_RULES.md](OPERATING_RULES.md), update the index, commit, and report the commit. If it cannot be written, report `UNSAVED` with a draft.
5. Never write credentials, personal information, user-name paths or chat identifiers into these files.

## One-line startup instruction

Before any Nagi product behavior change, read `docs/product-decisions/README.md`, `OPERATING_RULES.md` and the relevant DEC files in this repository; record newly approved decisions there and verify the commit.

## History

Until 2026-11-15 the decisions were kept in the private repository `Roy-To/NagiOS-internal`, and this file told Codex to read them there and never copy them here. [DEC-050](DEC-050-public-specification-repository.md) moved them to this public repository.
