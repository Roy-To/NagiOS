# NAGI — Product Decision Operating Rules

**Status:** ACTIVE / 2026-11-15 (rewritten by [DEC-050](DEC-050-public-specification-repository.md))
**Canonical repository:** `Roy-To/NagiOS` (public)
**Canonical directory:** `docs/product-decisions/`
**Audience:** ChatGPT / Codex / other agents / human contributors

## Principle

**Chat is the place for discussion; this public repository is the source of truth for agreed product decisions.** ChatGPT Memory, chat summaries, model prompts and temporary work artifacts are not the archive. A decision is **recorded** only when the Markdown is committed here and that commit is verified.

The earlier private ledger `Roy-To/NagiOS-internal` is retired as the source of truth. Do not record new decisions there.

**共同開発者の役割（[DEC-046](DEC-046-co-developer-governance.md)）:** 創設者が製品全体の方向性と最終判断を担い、共同開発者は割り当てられた担当領域の設計・実装を主体的に進める。

## 1. At the beginning of a Nagi task

1. Decide whether the task is a product decision, a design review, or implementation.
2. For product behavior, read this file, [README.md](README.md), the relevant `DEC-*.md` and, when relevant, [roadmaps/](roadmaps/README.md) from the current `main`. Do not rely on chat memory.
3. For implementation, also read the root `AGENTS.md`, the implementation specification and the current code.

## 2. When the user makes a new decision

1. Separate **CONFIRMED** (clearly approved by the user), **PROVISIONAL** (idea or undecided value), **REVIEW NOTE** (assistant observation) and **SUPERSEDED** (replaced). Never promote a suggestion to a user decision.
2. Update the existing `DEC-*.md` for the topic, or add a new one from [TEMPLATE.md](TEMPLATE.md). One topic per file.
3. Record scope, behavior, exceptions, open points and the date. Do not invent numbers or test results.
4. When a decision changes an earlier one, add a dated note to the earlier file instead of silently rewriting it.
5. Add the decision to the index in [README.md](README.md).
6. Commit to this repository and verify the commit. Small documentation changes may go directly to `main`; larger ones through a pull request.
7. Report the commit ID or link. If writing is blocked, say **未保存** and give a copyable draft.

## 3. What is never written down

- Credentials: passwords, API keys, tokens, private keys.
- Personal information, local paths that contain a user name, and chat or conversation identifiers.

Everything else about Nagi's product, roadmap and design may be recorded publicly.

## 4. Handoff to ChatGPT and Codex

- ChatGPT: paste the text in [CHATGPT_PROJECT_INSTRUCTIONS.md](CHATGPT_PROJECT_INSTRUCTIONS.md) into the project instructions. Saving the file alone does not change ChatGPT's settings.
- Codex: the root `AGENTS.md` of this repository points here, so Codex sessions in this repository read these rules automatically. See [CODEX_START_HERE.md](CODEX_START_HERE.md).

## 5. Completion report

Distinguish **discussed** (proposed in chat), **confirmed but unsaved** (agreed, commit not verified), **recorded** (committed here and verified) and **implemented** (code merged and tested). A recorded decision is not an implementation.
