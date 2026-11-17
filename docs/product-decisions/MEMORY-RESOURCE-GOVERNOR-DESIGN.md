# Memory Resource Governor — detailed design proposal

**Status:** PROVISIONAL design contract; no runtime implementation authorization  
**Scope:** Host-only research may validate abstractions. Formal runtime remains subject to the 0.3 revised roadmap gates.  
**Parent:** [DEC-013](DEC-013-lightweight-memory-engine-benchmark.md)

This note turns DEC-013's confirmed policy into a reviewable design boundary. It does not select algorithms, profile counts, thresholds, rankers, embedding models, databases, or physical APIs. Those remain PROVISIONAL until measured and decided.

## Invariants

- Canonical Memory is user-owned, model-independent, and retained across hardware changes. “Persistent” does not override DEC-004 natural decay, explicit forgetting, or deletion.
- Every retrieval begins with a candidate set authorized for the current Principal, Profile, Scope, source ACL/classification, source validity, and forget state. Authorization is re-evaluated from its authority; ranking cannot grant access.
- Exact/lexical lookup is the low-cost first pass. A query may escalate to N-gram, optional semantic/hybrid recall, then reranking only when query intent and first-pass evidence show a material need, and the resource budget permits it. Missing capability, pressure, cancellation, timeout, or error returns the best authorized first-pass result or an explicit no-result state.
- Hardware-dependent indexes, caches, and context adapters are derived and rebuildable. They never become the canonical record.

## Abstract contract (names are provisional)

```text
discover_capabilities() -> CapabilitySnapshot
observe_pressure() -> PressureSnapshot
authorize_candidates(principal, profile, scope, source_state, forget_state) -> AuthorizedSet
plan_query(query_class, AuthorizedSet, capabilities, pressure, budget) -> QueryPlan
retrieve(plan, AuthorizedSet, cancellation) -> RetrievalResult
allocate(request, capabilities, pressure, foreground_reservation) -> BudgetLease | Deferred
reconcile_index(canonical_revision, profile, source_events, cancellation) -> IndexState
explain(decision_id, audience) -> SafeExplanation
```

Snapshots may report CPU, RAM/available memory, GPU/NPU availability and memory, storage capacity/pressure, power source/constraints, thermal or throttling state, foreground/background load, and measured latency/pressure telemetry where discoverable. Unknown is an explicit value, never silently treated as abundant capacity. Physical discovery APIs are outside this contract.

The allocator reserves available memory and responsiveness for foreground work first, then admits bounded query work, then deferrable index maintenance. Work must be fair across eligible requests; aging or equivalent starvation protection is required, while policy/permission boundaries remain absolute. Budgets are leases with cancellation and observable completion. Hysteresis and cooldown suppress oscillation; their values are PROVISIONAL. Resource loss or stale telemetry invalidates leases and selects fallback.

## Query and index state model

```text
Received -> AuthorizedCandidates -> LightweightPass
                                  -> Sufficient -> Return
                                  -> Insufficient -> EscalationEligible?
                                     no -> ReturnFallback
                                     yes -> Semantic/Hybrid? -> Rerank?
                                           -> Return (authorized subset only)
Any active state -> Cancelled | TimedOut | CapabilityLost -> SafeFallback

Index: Current -> Dirty -> RebuildQueued -> Building -> Validated -> Current
                                    |             |          |
                                    +-> Deferred   +-> Cancelled/Failed -> Degraded
Forget/source invalidation -> immediately suppress -> purge/reconcile derived artifacts
Profile migration -> isolated target build -> validate -> switch -> retire old derived index
```

Index work is online only within an admitted budget; otherwise defer/offline rebuild. Forget/source invalidation must suppress affected results before asynchronous physical cleanup. Profile switches cannot expose another profile's index. Failed or partial rebuilds retain a safe readable canonical source and use a compatible prior index only if its authorization and revision checks remain valid; otherwise use lightweight search.

## Decision records and user explanation

Record decision id, coarse query class, selected capability flags (not device identifiers), pressure band, chosen stages, fallback/escalation reason code, elapsed-time bands, cancellation/outcome, and index generation. Do not persist raw query text, memory content, ACL labels, source names, embeddings, or result identifiers in diagnostic logs. Any detailed trace is opt-in, minimized, access-controlled, and separately specified. User-facing explanation uses plain reason codes such as “simple lookup”, “semantic search unavailable”, “system busy; used lightweight search”, or “index update deferred”; it must not reveal inaccessible candidate existence.

## Verification contract

An implementation of the abstraction must support deterministic fake capability/pressure providers, fake clock, seeded synthetic canonical records, injected cancellation/failures, and observable stage/budget traces. Negative tests must establish:

- denied, unknown, cross-profile, invalid-source, and explicitly forgotten records produce no content, metadata, or cardinality leakage at every stage;
- escalation and reranking receive only the authorized set; ranking scores cannot restore excluded records;
- absent accelerator/model, stale or unknown telemetry, budget denial, timeout, cancellation, crash, and corrupt/partial index all yield safe fallback without blocking foreground work;
- concurrent workload is bounded and fair; hysteresis avoids rapid mode oscillation under boundary-flapping signals;
- forget/source deletion suppression takes effect before cleanup; restart, backup restore, rebuild, and profile migration cannot resurrect forgotten data or mix profiles;
- logs and explanations contain no query, memory, source, permission, or inaccessible-result secrets;
- hardware transition changes derived artifacts only; canonical memory, provenance, protections, permissions, and forget state remain unchanged.

Acceptance candidates (not thresholds): zero permission/forget violations; measurable bounded foreground impact under query and maintenance load; reproducible retrieval-quality/resource tradeoffs; verified cancellation and fallback; index rebuild and migration integrity; explainable, secret-safe decisions. Numeric limits and final acceptance ownership remain OPEN.

## Release gates and workstream boundary

| Release | Permitted evidence | Gate |
|---|---|---|
| 0.1 | Isolated host-only synthetic research and contract refinement | No production data, kernel/runtime integration, or target acceptance claims |
| 0.2 | Forward-compatibility review across identity, search, storage, jobs, model runtime, permissions, and IPC | Confirm interfaces do not block the 0.3 design; no formal engine activation |
| 0.3 | Formal runtime only under revised roadmap specification, workstream ownership, and activation/integration gates | Tests above plus benchmark evidence and explicit release acceptance; this note alone grants no implementation authority |

Host-only PoC inputs: seeded synthetic multilingual memories at MEMORY-BENCH-01 scales; query judgments with allowed/forbidden IDs; injected capability and pressure traces; fake clock; cancellation/failure schedule; and instrumentation for stage, latency, resource budget, foreground jitter, index bytes, and leak checks. Success means reproducible contract-level evidence for fallback, authorization preservation, fairness, stability, lifecycle integrity, and tradeoffs. It does not mean a runtime, device, or 0.3 acceptance pass.

## Open questions

Capability APIs and sampling; telemetry retention; query-class taxonomy; escalation/ranking methods; budget units and numeric limits; hysteresis/cooldown values; fairness policy details; persistent index format; model/provider choice; profile migration transaction mechanics; explanation UI; final acceptance thresholds and owners.
