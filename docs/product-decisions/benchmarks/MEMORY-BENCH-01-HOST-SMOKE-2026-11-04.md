# MEMORY-BENCH-01 — Initial host-only synthetic smoke report

> **2026-11-15 公開移管:** [DEC-050](../DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**Status:** REVIEW NOTE / measured experiment; no adoption or release acceptance  
**Run date (UTC):** 2026-11-04  
**Scope:** isolated Linux x86-64 host, Python 3.13.5, SQLite 3.46.1, FTS5 `unicode61`  
**Reference documents:** [DEC-013](../DEC-013-lightweight-memory-engine-benchmark.md), [MEMORY-BENCH-01 protocol](MEMORY-BENCH-01.md)

## What was actually implemented and measured

A small standard-library-only host prototype uses synthetic seeded memories for:
- **A:** structured, owner/permission/sensitivity/state-filtered SQLite FTS5/BM25 lookup;
- **B:** A plus bounded retrieval-time duplicate projection, explicit-preference-weighted reranking, and UTF-8 byte-bounded context selection;
- **C:** NOT_MEASURED (no pinned offline embedding model/vector index). No semantic-model score is claimed.

Nine focused host unit tests passed: profile isolation, permission/sensitivity/scope/source exclusion, archive exclusion, duplicate non-merging of contradictions, explicit forget and reingest suppression after index rebuild, trash/restoration/permanent-source invalidation, bounded context, DB reopen, and C not silently executing.

| Synthetic records | Database file (MiB) | A p95 lookup+context (ms) | B p95 lookup+context (ms) | A/B synthetic Recall@5 | Negative-query failures |
|---:|---:|---:|---:|---:|---:|
| 1,000 | 0.32 | 0.1352 | 0.1861 | 1.0 / 1.0 | 0 / 0 |
| 10,000 | 2.54 | 0.1091 | 0.1155 | 1.0 / 1.0 | 0 / 0 |
| 100,000 | 25.68 | 0.1509 | 0.1501 | 1.0 / 1.0 | 0 / 0 |

Run settings: seed 42, up to 48 generated anchor queries plus fixed negative/positive cases, 3 repetitions. The actual number of generated queries varies by dataset, and both modes share the same DB/cache/process. Results are **not** controlled independent-hardware benchmarks.

## Interpretation and limitations

- The positive judgments are mostly easy synthetic unique-anchor matches; Recall@5 of 1.0 **does not establish real-world search quality**. The tiny timings do not justify choosing A over B or vice versa.
- B reduces duplicate selections within a bounded search result, but does **not** implement physical compaction; storage savings and complete provenance aggregation are NOT_MEASURED. Both A/B use the same 0.32 / 2.54 / 25.68 MiB database file.
- UTF-8 context byte budgets are **not tokenizer token counts**. No LLM inference, actual model swap, Japanese segmentation/paraphrase quality, materialized compression or security-grade multi-user authorization was tested.
- RSS was measured as a shared host-process peak; this cannot be attributed fairly to A/B, and this report makes no per-variant RAM claim. QEMU, ARM64 and Raspberry Pi have not been benchmarked.
- The PoC includes example-only deterministic HMAC suppression, not production key management, secure erasure, backup deletion, crash durability or permission-enforcement proof. Host unit test PASS is **not Nagi target/runtime PASS**.
- No private user data were used. The runnable Python PoC and raw JSON report were delivered as the conversation artifact `Nagi_MEMORY_BENCH_01_host_poc.zip`; **that ZIP/source has not been committed to GitHub**. ZIP SHA-256: `4f431c46c40ded6f23d19d83e7c655d55b3618d30d43e2a3b1d375338b08118a`.

## Recommended next work (NOT YET COMPLETED)

1. Version the tested PoC source in an authorized **Private** isolated research branch, or explicitly approve a sanitized public source location; do not modify product runtime/AGENTS/registries without ownership and gates.
2. Replace trivial anchor queries with realistic, graded multilingual relevance judgments; compare nDCG/MRR and false memories.
3. Measure A/B in separate cold/warm processes with repeatable RSS/latency/CPU budget and background foreground-impact tests.
4. Build a non-destructive provenance-preserving materialized compression candidate, and test real storage savings plus deletion/recovery.
5. Add a pinned offline embedding provider for C and real tokenizer measurements before making an architecture decision.

**Decision:** None. Algorithm selection, DB choice, thresholds and release gates remain unchanged.
