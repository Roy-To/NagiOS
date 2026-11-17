# MEMORY-BENCH-01 — Nagi Hardware-Adaptive Lightweight Memory Engine: Algorithm Comparison & Benchmark Protocol

> **2026-11-15 公開移管:** [DEC-050](../DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**Status:** REVIEW NOTE / execution plan (technical candidates and thresholds are **PROVISIONAL**)  
**Created:** 2026-11-04  
**Source decision:** [DEC-013](../DEC-013-lightweight-memory-engine-benchmark.md)  
**Existing product decisions:** DEC-003 / DEC-004 / DEC-005 / DEC-008 / DEC-012 / DEC-013 / DEC-014  
**Scope:** 0.1 design and isolated host-only benchmark preparation; 0.2 Forward Compatibility review; formal 0.3 implementation only after existing gates. This protocol is not runtime activation or Acceptance PASS.

## 1. Goal, hypotheses and non-goals

Test whether a lightweight, model-independent and Hardware-Adaptive Memory Engine can avoid prompt-history replay and mandatory always-on embeddings while preserving relevant, attributable, authorized memory across hardware generations. Keep user-owned canonical memory stable while measuring retrieval/context paths and regenerating device-dependent artifacts. Compare resource use and answer-support quality against a plain retrieval baseline. Do **not** assume any winning algorithm, database, embedding provider, scoring coefficients, fixed capability-tier design or performance pass thresholds.

No live user data, private repository content, model-specific user-memory state, production kernel changes or public release assertions belong in benchmark artifacts.

## 2. Candidate variants

| ID | Retrieval / learning | Role |
|---|---|---|
| A — lexical baseline | Structured canonical memory record; metadata/owner/ACL filters; lexical search and rank (e.g., BM25 / FTS). No mandatory LLM, embeddings or learned habit reranking | Establish minimal complexity and cost |
| B — compact & context | A + attributable duplicate/equivalence-group consolidation, repeated behavior aggregation, explicit-preference precedence, bounded relevance/recency rank, permission-first context packing, protected/archived states | Measure gain from deterministic selection and compaction |
| C — optional hybrid | B + optional locally computed embeddings with vector recall and hybrid fusion; lexical fallback on resource pressure / missing model | Measure incremental retrieval quality versus disk, RAM, build cost and latency |

**Equal test contracts:** All variants must implement the same profile/permission filters, explicit forget suppression and source invalidation to prevent unsafe options appearing artificially faster. Compare B with compaction disabled as an ablation. Compare C with embeddings turned off and check that ordinary lexical behavior survives. The database, retrieval engine, tokenizer and embedding model are interchangeable experimental components, not design decisions.

The A/B/C variants describe retrieval techniques, not fixed hardware profiles. Lite / Standard / Enhanced are provisional examples only; do not assume a fixed number of modes or a device-name lookup. A given technique may be selected only when measured capability and current resource headroom support it.

## 3. Minimal reference record for fixtures (not production schema)

`memory_id`, `profile_id`, `owner_scope`, `kind` (working/episodic/semantic/preference/habit), `content`, `normalized_key`, `source_refs[]`, `source_revision`, `permission_tags`, `sensitivity`, `created_at`, `last_meaningful_use_at`, `observed_sessions[]`, `explicit_user_directive`, `protected`, `lifecycle_state` (active/decay/archive/deleted), `forget_suppression_marker`, `confidence`, `importance`, `provenance`.

The fixture generator must seed all randomness and clock values. Separately track source data, memory records, derived summaries, indexes and caches so that deletion, retention and occupied bytes can be attributed correctly.

## 4. Fixture matrix

Sizes: **1k / 10k / 100k** memory records, recorded precisely before/after compaction.

A deterministic workload should include:
- English and Japanese short and long memories, paraphrases, similar-but-contradictory preferences, explicit overrides and time-limited task facts;
- repeated behavior both across distinct sessions and self-triggered by an agent (the latter **must not** count as user adoption);
- unrelated events, repeated content, multi-source corroboration, archived/protected facts, stale source revisions, renamed/moved resources;
- profiles A/B, team data lacking explicit share permission, sensitive/classification-inherited records and unknown/denied permissions;
- deleted and trashed source files, explicitly forgotten memories, derived summaries/index/cache remnants, backup/Wayback restore attempts;
- active-task/workspace matches, recency-only distractors, typo or paraphrase queries, Japanese segmentation issues and lexical-zero-hit queries.

Include **query judgments** with permitted relevant MemoryIds, forbidden IDs, expected preference precedence, source validity and minimum supporting evidence. Balance relevant and distractor results across datasets. Publish only synthetic fixture seeds and summaries safe to share; do not copy private decisions or source contents into Public artifacts.

## 5. Representative query classes

1. Direct lookup of explicit user settings and project decisions.
2. Search by related terminology, synonym and paraphrase (EN / JA).
3. Cross-session habit retrieval with correct task/workspace scope.
4. Conflicting old and new preferences; user's explicit override wins.
5. Recently active task with many older distractors.
6. Denied/cross-profile/sensitive data: must return **zero** disallowed records, including metadata and cardinality leaks.
7. A removed source: dependent memory must not appear; valid independent alternative provenance may remain.
8. Explicit forget, reboot, index rebuild and backup restore: forgotten facts must not reappear or silently relearn.
9. Model swap: reconstruct context from unchanged canonical memory regardless of model provider.
10. Tiny context budget: prioritize authorized, relevant, protected/explicitly important records and disclose truncation rather than invent evidence.

## 6. Measurements and collection procedure

For each variant, dataset size and query class, record:
- `ingest_ms`, `index_build_ms`, `index_update_ms`, `compaction_ms`, `cold_start_ms`;
- `retrieve_ms`, `permission_filter_ms`, `rank_ms`, `context_build_ms`, `end_to_end_ms` and **p50 / p95 / p99**, with warm/cold cache separated;
- `rss_peak_bytes`, steady RSS, peak allocated memory when observable, cache residency and process/OS metrics with method specified;
- CPU/core and accelerator capabilities (GPU/NPU where available), available RAM/storage, power source and measured energy/power draw where observable, current foreground/background load, and thermal or throttling state where measurable;
- `source_bytes`, `records_bytes`, `index_bytes`, `derived_summary_bytes`, `cache_bytes`, `total_bytes` and compaction ratio (define denominator explicitly);
- token count before/after selection for each tested tokenizer/model adapter, number of model calls for ingestion/rewrite/retrieval, and background CPU time;
- Recall@k, MRR and nDCG@k on **authorized** query judgments, plus false-memory/conflict errors, provenance fidelity, forget/ACL failure counts (zero tolerated by security acceptance), ranking explainability and factual support coverage;
- foreground UI-like workload latency/jitter while ingesting, indexing or compaction runs; cancellation and model-unavailable fallback behavior.

**Procedure:** record CPU architecture/cores, GPU/NPU when present, RAM, available storage and disk type, power source/measurement method, OS, runtime/library versions, sample/seed and exact commit; treat device model as descriptive metadata rather than the selection rule. Generate clean fixture; warm up query paths; collect repeated independent runs at a documented count; measure cold process/index open separately from warm; report median and tails with units and uncertainty; preserve structured CSV/JSON results and a human-readable summary. Missing or failed metrics must be marked **NOT MEASURED/FAILED**, never zero, and never claimed PASS.

## 7. Correctness / isolation gates (qualitative and deterministic)

Each candidate must prove:
- a query from Profile A cannot return Profile B's private memory or its existence;
- source sensitivity/ACL/provenance is inherited by summaries and merged memories; permission changes are honored **before** context composition;
- explicit user preference overrides inferred habits; internal agent repetition alone cannot promote a user preference; decay does not remove protected memory;
- forget suppression survives rebuild/restart/restore and prevents reuse of stale source; Memory deletion and Source/Activity/Wayback/backup deletion boundaries are reported truthfully;
- trash source disables dependent memory; permanent source removal removes single-source dependent derived material; independently authorized corroborated facts are handled separately;
- memory provider failure / embeddings disabled / no model loaded does not break the OS; lexical fallback stays usable;
- model hot swap preserves canonical profile/memory/learning state while provider-specific KV cache can be discarded;
- compaction does not merge contradictory or cross-scope facts and maintains provenance and editability;
- changing hardware generation does not relearn, rewrite, or lose canonical profile/memory, provenance, permissions, or forget suppression; only reproducible indexes, caches and context adapters may be rebuilt or discarded;
- source deletion or classification/permission changes are re-evaluated at rebuild/use time; stale derived artifacts cannot resurrect forgotten, revoked, or invalidated memory;
- selecting a Lite fallback, optional embedding/hybrid retrieval, or reranking never weakens privacy, permission, provenance, or explicit-forget enforcement.

Any isolation, disclosure or explicit-forget failure invalidates a performance comparison result as a deployable candidate, regardless of speed.

## 8. Hardware and execution boundary

1. **Host-only PoC:** isolated benchmark program on a development host, separate path/branch and synthetic fixtures; no claim of Nagi guest execution.
2. **Nagi official reference target:** when permitted under gates, QEMU x86-64 / 4 vCPU / 8 GB RAM (0.2 baseline) for later target acceptance; not interchangeable with host measurements.
3. **Future hardware study:** ARM64 generic QEMU followed by Raspberry Pi 5 real board where available. Clearly label board/OS/runtime and never infer Pi performance from x86-64 host results.

Memory pressure scenarios should limit resident cache, disable optional embeddings, reduce context budget safely, defer background compaction and preserve responsiveness. Power use only where the measurement setup supports it.

### Hardware-adaptive comparison additions

- Run the same synthetic canonical-memory fixture and query judgments on representative constrained, standard and higher-capability configurations, including more than one hardware generation where available. Compare measured response quality, foreground latency/jitter, RAM, storage growth and power/energy; do not classify by device name alone.
- Measure CPU, RAM, GPU/NPU, storage, power conditions and current load. Where the host cannot measure an accelerator or energy reliably, mark that dimension **NOT MEASURED** rather than inferring it from the model name or device class.
- Compare the lexical Lite path with optional embedding/hybrid recall and advanced reranking. Optional work must not run continuously merely because hardware supports it; foreground responsiveness has priority, and constrained resources, foreground contention or unavailable accelerators/models must safely fall back to lexical Lite retrieval.
- Migrate the same canonical profile and Memory between older and newer hardware fixtures. Rebuild derived indexes and adapters from canonical data, then verify record/provenance/permission/forget invariants and retrieval behavior without relearning.
- Treat Lite / Standard / Enhanced labels, their count, transition algorithm and numerical thresholds as **PROVISIONAL** until representative measurements and 0.3 Implementation Spec review.

## 9. Decision process after data collection

Compare each option as a Pareto tradeoff: correctness and permission guarantees first, then context relevance/quality, tail latency and foreground responsiveness, RAM/disk growth, battery/power where observable, maintenance/portability cost. Report **A/B/C comparison**, ablations, EN/JA results, dataset scaling, failures, a recommended starting configuration, risks and unknowns. Exact latency, storage, retention and ranking thresholds are decided only after representative measurement and 0.3 Implementation Spec review.

## 10. Implementation preparation checklist (not activation)

- [ ] Confirm Public/Private boundaries, AGENTS.md, workstream owner, base SHA, allowed paths and release gates before creating any code branch.
- [ ] Build seeded synthetic fixture generator and query judgment file.
- [ ] Implement minimal shared Memory API and permission-first candidate interface in isolated host harness.
- [ ] Implement A, then B with ablation flags, then C only if a pinned offline embedding dependency is available.
- [ ] Add correctness, restart/restore, permission and explicit-forget negative tests.
- [ ] Run 1k smoke; then 10k; then 100k without changing benchmark contracts.
- [ ] Collect reproducible raw metrics with resource conditions and build fingerprint.
- [ ] Review quality-versus-cost and record outcomes without declaring target runtime PASS.
- [ ] Propose 0.3 Implementation Spec choices once the required 0.2 interface gates are sufficiently stable.

## 11. Operational comparison details

### Algorithm axes and ablations

The candidate IDs in section 2 are the comparison groups. Apply these controlled ablations to isolate the cost and quality of each algorithm family:

| Axis | Comparison | Record |
|---|---|---|
| Canonical storage | Store repeated source text versus share canonical source references; exact-hash deduplication on/off | Record, source, index and cache bytes; provenance fidelity; update/delete propagation |
| Lexical/hybrid retrieval | Metadata and current permission filter followed by FTS/BM25; optional local embedding recall; measured hybrid fusion | Recall@k, MRR, nDCG@k, lexical-zero-hit recovery, build/open/search costs and fallback behavior |
| Context selection | Permission-first filtering then deterministic relevance/importance/recency selection within a budget | Included and omitted MemoryIds, truncation reason, provenance and context size; no unauthorized metadata in any trace |
| Learning and forgetting | Repeated user-adopted evidence across distinct sessions, explicit user instruction/correction, confidence/decay ordering, protected facts and Forget suppression | State transitions and reasons across restart/rebuild; agent self-repetition must not count as user adoption |
| Compaction | Flat records versus hierarchical compaction; explicit compaction versus asynchronous idle summary | Bytes saved, latency, foreground impact, cancellation/restart behavior and source/sensitivity/ACL inheritance |

Near-duplicate detection is a measured candidate only. Never silently merge contradictory facts, different owners, permission scopes or source classifications. Confidence and decay are ranking aids only: explicit user instructions, protected facts, current permissions and source ACLs take precedence. No LLM is required for retrieval or state transitions. If optional summarization invokes an LLM, report every call/token and compare against deterministic or extractive fallback.

### Required lifecycle run matrix

For each supported dataset size, run and report the relevant cases below. Keep lifecycle failures separate from ranking metrics.
Use fixture-controlled lifecycle states rather than elapsed-day or repetition cutoffs; DEC-004 leaves those numeric values provisional. Natural forgetting and explicit Forget are separate behaviors: new user evidence may relearn naturally deleted information, while explicit Forget remains suppressed unless the user explicitly authorizes resumption.

| Case | Setup and action | Expected behavior |
|---|---|---|
| Load/restart | Cold and warm profile/store/index open; clean restart; interrupted background indexing/summary followed by restart | Load/open and recovery latency are reported; canonical state remains valid; derived artifacts safely rebuild or resume |
| Model hot-swap | Retrieve context, change provider or remove the model, then repeat retrieval | Canonical Memory/profile is unchanged; provider-specific cache may be discarded; no mandatory LLM or embedding call |
| Profile isolation | Alternate same query across profiles, including warmed cache and summaries | No cross-profile content, count, explanation or cache disclosure |
| Explicit Forget and restore | Explicitly forget a record, restart, rebuild indexes, then restore an older backup/Wayback fixture | The item stays suppressed and is not silently relearned; only a separate explicit user-authorized resume may restore use; report Memory deletion separately from source, Activity and backup deletion |
| Natural decay/archive | Seed the same eligible record in Decay and Archive; retrieve internally without a meaningful user adoption | Internal retrieval alone does not extend retention; protected facts remain protected; no numeric age/decay threshold is inferred |
| Natural deletion and relearning | Seed a naturally deleted fact, then provide a new permitted user observation of that fact; run an explicit-Forget control in parallel | New evidence may create/reactivate a Memory with fresh provenance after natural deletion; it must not resurrect an old backup or bypass explicit-Forget suppression |
| Source and ACL changes | Trash/permanently delete source; revoke permission or change classification; query old summary and cached result | Current permission is checked before use; dependent results are invalidated/suppressed per DEC-005 |
| Resource pressure | Constrain RAM/cache and context budget; disable or rebuild embeddings while foreground work continues | Optional work defers/cancels; foreground impact is recorded; lexical fallback and permission enforcement remain operational |
| Hardware-generation migration | Move the same canonical profile/memory fixture from older to newer hardware capability; rebuild derived index/cache/context adapter | Canonical records and learning state do not change or retrain; derived artifacts rebuild; provenance, current permissions and forget suppression remain intact |
| Workspace checkpoint | Restore DEC-014 checkpoint referring to removed or newly denied resources | Recheck current permission; do not restore file content, Memory lifecycle, or interrupted Tasks through the Workspace checkpoint |

### Reporting requirements

For load/open, restore/recovery, search and context construction, report p50/p95/p99 per dataset, query class and cold/warm condition. Include measured sample count with every percentile; p99 with fewer than 1,000 observations in a cell is **NOT MEASURED**. This is a statistical reporting rule, not an acceptance threshold. Record RSS and peak RAM, and separate disk bytes for canonical data, lexical index, vector index, derived summaries and cache. Count LLM invocations and tokens by operation, including zero-call runs, and measure foreground latency/jitter during background work. Report Recall@k, MRR, nDCG@k together with memory correctness, provenance, permission/Forget failures and explainability.

### Release and parameter boundary

- 0.1 work is design and isolated synthetic host benchmark preparation only.
- 0.2 checks Forward Compatibility with the existing Identity, Permission, Search, Storage, Jobs, Model Runtime and IPC boundaries. It does not authorize Memory Engine implementation or alter existing gates.
- Formal 0.3 implementation remains behind the existing release/activation gates and Integration Owner boundary.
- Treat the QEMU 0.2 development profile of 4 vCPU / 8 GB RAM as the primary later Nagi development reference. Distinguish host harness overhead from guest measurements. ARM64 and Raspberry Pi 5 are separate future measurements.
- Before representative measurements, do not finalize numeric performance/quality thresholds, database, embedding model, production schema, scoring/forgetting coefficients, compaction schedule or cache/context capacities.

**Current outcome (2026-11-04):** Protocol plus a limited **host-only synthetic A/B smoke** completed, documented at [MEMORY-BENCH-01-HOST-SMOKE-2026-11-04.md](MEMORY-BENCH-01-HOST-SMOKE-2026-11-04.md). C remains **NOT_MEASURED**. The PoC source is delivered as a conversation ZIP and is **not yet in GitHub**. These simple synthetic timings do not establish an optimal algorithm, validated real-user quality, or Nagi target/runtime PASS.
