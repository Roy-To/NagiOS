# MEMORY-BENCH-01 — V3–V5 host-only synthetic evaluation

> **2026-11-15 公開移管:** [DEC-050](../DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**Status:** REVIEW NOTE / measured experiment; no adoption or release acceptance  
**Run date (UTC):** 2026-11-04  
**Seed:** 42  
**Scope:** isolated Windows 11 x86-64 host only; Python 3.12.10; SQLite 3.49.1 with FTS5 `unicode61` and `trigram`  
**Protocol:** [MEMORY-BENCH-01](MEMORY-BENCH-01.md); current product boundary: [DEC-013](../DEC-013-lightweight-memory-engine-benchmark.md)

## V3/V4 work audit

The accessible V3/V4 work had no verifiable completed artifact, attached result, or committed V3/V4 source/report on Private `main`. This report documents a new standalone rerun and does not attribute it to the unfinished Work tasks.

## Experiment

The harness used seeded synthetic datasets of 1,000 / 10,000 / 100,000 records and 96 fixed synthetic judgments per size. It compared A (lexical), B (bounded context selection), V3 (lexical plus trigram recall), C (embedding/hybrid candidate with lexical fallback), and LOW_RESOURCE_C. Each variant/connection condition collected 1,000 latency observations; fresh-connection measurements retained the OS page cache. The 15-cell RSS recheck was a separate resource-only pass with one deterministic query per query class.

No local tokenizer or offline embedding runtime/model was installed. Context budgets are UTF-8-byte approximations. C hybrid retrieval and real token counts are **NOT MEASURED**; C rows measure lexical fallback. LOW_RESOURCE_C disables optional embedding/cache and bounds candidates/context, but the host OS memory limit was not forced.

## Results

Expected-record recall under the 1,024-byte context scenario:

| Records | A | B | V3 | C lexical fallback | LOW_RESOURCE_C |
|---:|---:|---:|---:|---:|---:|
| 1,000 | 0.484 | 0.484 | 0.828 | 0.828 | 0.828 |
| 10,000 | 0.484 | 0.484 | 0.022 | 0.022 | 0.129 |
| 100,000 | 0.484 | 0.484 | 0.022 | 0.022 | 0.183 |

The recall values are synthetic fixture outcomes, not human-use quality. A/B stayed faster on this host but had lower recall on the same judgments. V3/C fallback recall rose at 1k and fell as distractors scaled; LOW_RESOURCE_C improved modestly over V3/C fallback at 10k/100k but remained costly.

At 100k, warm retrieval p50/p95/p99 and the separately sampled per-variant RSS peak were:

| Variant | Context recall | Warm p50/p95/p99 ms | RSS recheck peak bytes | Full worker CPU s |
|---|---:|---:|---:|---:|
| A | 0.484 | 0.187 / 1.992 / 2.597 | 107,847,680 | 3.688 |
| B | 0.484 | 0.179 / 1.901 / 2.284 | 108,511,232 | 3.406 |
| V3 | 0.022 | 707.011 / 1,631.786 / 2,408.399 | 118,976,512 | 2,010.453 |
| C lexical fallback | 0.022 | 891.523 / 1,728.763 / 2,000.040 | 120,741,888 | 2,737.031 |
| LOW_RESOURCE_C | 0.183 | 793.478 / 1,563.841 / 1,792.416 | 121,843,712 | 2,183.578 |

At 100k, incremental add/update/delete p50/p95/p99 were 1.610/3.449/8.868 ms, 3.368/6.984/9.302 ms, and 1.565/3.077/6.158 ms. Restart/count was 13.244/21.762/36.712 ms. Background compaction took 6.547 s and increased database bytes by 115,021,712; the overlapping foreground-query cell had 9 observations (p50 736.779 ms, p95 946.551 ms, p99 **NOT MEASURED**). Lifecycle-block CPU was 29.969 s; its dedicated RSS sample was **NOT MEASURED**. Per-variant RSS figures above are from the separate recheck, not the full latency pass.

## Safety checks and limits

Twenty-two host unit tests passed. Tests cover explicit Forget across reindex/restart/backup-fixture restore, profile/team separation, current ACL and sensitivity/provenance inheritance, trash/permanent source deletion, duplicate merge followed by Forget, contradictory records, and lexical fallback when embedding is unavailable or fails. Full-run judged query cells recorded zero forbidden hits and zero context-budget overflows. These deterministic tests are not production authorization, secure-erasure, crash-durability, or backup-deletion proof.

No tokenizer, embedding hybrid, induced physical memory pressure, extended soak test, retention-aging or memory-leak trend, Nagi guest/QEMU, ARM64, Raspberry Pi, GPU/NPU, power, or real-user quality was measured. The benchmark-start code hash was not captured; the local package records current packaged-source and fixture hashes separately. The previous Linux A/B smoke used a different host and easy anchor judgments, so its scores and timings are not directly comparable.

**Decision:** None. Algorithm, database, embedding model, profile design, thresholds, and runtime behavior remain unselected. The 0.1 M30, 0.2, and 0.3 gates remain unchanged; this report does not authorize Nagi 0.3 runtime implementation. The local source, tests, raw JSON/CSV, and full report are delivered as a separate ZIP; source is not committed here.
