# ADR 0045: M21/M22 actions requested by isolated clients over `action@1`

Status: accepted for M21/M22
Date: 2026-10-03
Milestones: M21 — Planner/Validator/Executor; M22 — AI Safety/Undo
Builds on: ADR 0043 (isolated process identity) and ADR 0044 (Search IPC)

## Context

The M21 `file.search` path and the M22 `file.move` path both built a
hard-coded `CallerIdentity` inside init. That value was then passed to
Context resolution, Plan validation, policy, execution, NH16 History, and
the Activity Ledger. Their policies only check whether a caller is the
granted application. Nothing tied the caller to a real requesting process,
so M21 and M22 remained `PARTIAL` for "authenticated M21/M22 authority".

## Decision

1. **Wire format.** Add `crates/nagi-action-ipc`, an allocation-free
   `action@1` codec:
   - **Launch intent:** Supervisor → client, the `argv` equivalent.
   - **Request:** client → service, an intent of at most 127 UTF-8 bytes.
   - **Result:** service → client, a status plus up to 15 Object IDs; only a
     `Succeeded` result carries Object IDs.
   - **No identity or authority fields:** a client cannot name a caller
     identity, capability, Object ID, or plan.
2. **Identity source.** `user/nagi-init/src/action_ipc.rs` spawns the real
   `nagi-action-client` ELF (ADR 0043) and records
   `ProcessId -> CallerIdentity` at launch. A request's caller is that
   record, matched by kernel-stamped sender ID. An unmatched sender gets
   `UnknownCaller`.
3. **Planning.** The service maps the intent to its plan with the existing
   deterministic fixture planner, which stands in for the generative
   planner. The plan then passes the unchanged Context → Validate → Policy →
   Execute path using only the resolved identity. History and Activity
   Ledger records therefore carry the identity of the process that asked.
4. **Acceptance.** Under the `m21-action-ipc` feature, used by `./nagi m19`
   and `./nagi m22`:
   - **Foreign client first.** Launched as a foreign application, it is
     denied by policy before any action handler is registered (`Denied`).
   - **Granted client second.** Launched as the granted application, it runs
     the real action:
     - `file.search` returns the live file's Object ID;
     - `file.move` commits the three-file NH16 group, which later boots undo
       and verify.

   The client reports what it received, and the Supervisor requires it to
   equal what the service sent.

## Bounds and non-goals

- The intent is supplied to the client by the Supervisor as a launch
  argument. Nagi Bar / Albert as real requesters are later work.
- Superseded 2026-10-03 (see ADR 0046):
  - `file.copy` is now also requested by an isolated client;
  - the plan-rejection and partial-execution fixtures now evaluate with the
    resolved caller and Supervisor grants;
  - the `./nagi m27` and `./nagi m30` images are built with
    `m21-action-ipc`.
- Grants now come from the ADR 0046 Supervisor launch registry and its
  manifests.
- One isolated client at a time (ADR 0043).

## Verification

- `cargo test -p nagi-action-ipc` covers the codec.
- Local QEMU/OVMF runs, 2026-10-03:
  - `./nagi m19`: foreign `file.search` denied; granted `file.search`
    executed.
  - `./nagi m22` (fresh disk, three boots): boot 1 printed the M21 and M22
    foreign-denied and isolated-caller PASS markers; later boots completed
    NH16 grouped Undo and restart verification.
