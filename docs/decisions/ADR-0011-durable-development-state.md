# ADR-0011: Durable, Workstream-Local Development State

- Status: Accepted for Nagi 0.2 development preparation
- Date: 2026-09-25

## Context

Nagi 0.1 work has spanned repeated long CI runs, host/worktree changes, and
M17 diagnostics recorded across status and decision documents. The current
repository already tracks useful evidence, but a new session must search
history to find the active failure and next experiment. A shared mutable
status file would also conflict when independent workstreams run in parallel.

## Decision

Keep 0.1 state in `docs/implementation_status.md`. For 0.2, make one
machine-readable JSON state authoritative per active workstream at
`.dev/workstreams/<id>/state.json`, validated against a checked-in schema and
read by `nagi dev status|resume|verify|diagnose`. Keep workstream ownership and
path boundaries in an integration-owned registry. Derive current branch/HEAD
from Git rather than duplicating it in state; record the last verified commit
to avoid a self-referential commit hash.

Do not yet split target build/acceptance artifacts or add build caches. First
measure artifact sizes and stage costs, define a complete immutable input
fingerprint, and prove clean-environment reproducibility. CI cheap jobs may
cancel stale runs; an in-progress expensive target run is allowed to complete,
with only the newest pending target run retained.

## Consequences

- Each stream can update its state without editing another stream's file.
- A new session can identify branch, checkpoint, blocker, evidence, and next
  action in a few commands without replaying chat history.
- The registry/schema are shared integration files and need a clear owner.
- The initial state CLI and diagnostics remain host-side tools; Nagi runtime
  and M17 acceptance criteria are unchanged.
- Cache and artifact transfer optimization stays deferred until provenance and
  reproducibility checks can prevent stale-artifact PASS.

## Alternatives considered

- One global Markdown ledger: easy to read but duplicates structured values
  and conflicts under parallel edits.
- One global JSON status file: machine-readable but still a shared write
  hotspot.
- State only in chat/task history: cannot resume reliably across sessions or
  PCs.
