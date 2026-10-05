# ADR 0046: Supervisor launch registry with manifest-defined grants

Status: accepted for the M18–M23 shared service-identity prerequisite
Date: 2026-10-03
Builds on: ADR 0043 (isolated process), ADR 0044 (Search IPC), and ADR 0045
(action IPC)

## Context

ADR 0043–0045 resolved callers through ad-hoc launch tables kept inside each
acceptance module. Capability decisions used per-fixture "is this the
acceptance caller" checks. Three gaps remained:

- nothing declared which applications may be launched;
- application identity was not derived from a declaration;
- a capability was not tied to the lifetime of the session that held it.

## Decision

1. **Registry.** Add `libnagi::launch`, a bounded, allocation-free registry
   with host tests:
   - **`AppManifest`.** Text manifests use `app=` and `grant=` lines. The
     `AppId` is always `AppId::from_identifier(app)`. Malformed, duplicate,
     uppercase, or path-like names are rejected.
   - **`check_launch`.** It refuses an undeclared application, an already
     live application session, or a full table. The Supervisor calls it
     before spawning, because a spawned process cannot be withdrawn.
   - **`record_launch`.** It binds `ProcessId -> (AppId, AppSessionId,
     NodeId, WorkspaceId)`. PID 1 can never be relabeled.
   - **`has_grant`.** It is true only while a live launch holds that exact
     application session and its manifest grants the capability.
   - **`record_exit`.** It revokes the launch, and with it the session's
     grants.
2. **One Supervisor instance in init.** `user/nagi-init/src/supervisor.rs`
   is the only launch path for isolated applications:
   - manifests embedded from `user/nagi-init/manifests/` are loaded on first
     use, and an invalid manifest disables all launches rather than granting
     defaults;
   - `launch()` checks, spawns, then records;
   - `resolve()` maps kernel-stamped sender IDs;
   - `reap()` observes exit and revokes the launch.
3. **Consumers.** The isolated-process, Search IPC, and action IPC paths
   all launch and resolve through the Supervisor:
   - Search requires a live `search.query` grant.
   - `GrantSource::Supervisor` makes the M19/M22 action policies take
     `files.search` / `files.move` from manifests instead of the in-process
     fixture table.
   - Object ownership rules are unchanged.
4. **Declared applications.** The acceptance applications are now real
   declarations:
   - `org.nagi.acceptance.isolated-app` (no grants);
   - `org.nagi.acceptance.m19-search` (`search.query`, `files.search`);
   - `org.nagi.acceptance.m22-files` (`files.move`);
   - `org.nagi.acceptance.foreign-client` (`search.query` only).

   The M19 and M22 acceptance `AppId` constants are now derived from those
   identifiers. Their previous ad-hoc numeric values are gone, so User Data
   disks written by older images hold objects owned by the old IDs. CI uses
   fresh disks.

## Bounds and non-goals

- **Image-embedded manifests.** Manifests are compiled into the system image.
  Signed manifests delivered by the M16 package service, and user consent for
  grants, are later work.
- **Supervisor-chosen sessions.** The Supervisor chooses sessions, either new
  or restored. A real session manager is later work.
- **Remaining in-process paths.** The in-process acceptance caller
  (`GrantSource::InProcessAcceptance`) remains only for images built without
  `m21-action-ipc`. Update 2026-10-03: `file.copy` (granted by the
  `m22-files` manifest), the plan-rejection and partial-execution fixtures,
  and the M27/M30 images now run with the isolated caller and Supervisor
  grants.
- **Capacity.** The registry tracks four live launches, but the kernel still
  provides one isolated slot (ADR 0043).

## Verification

- `cargo test -p libnagi --lib launch` covers manifest parsing and rejection,
  derived identity, undeclared and duplicate launches, init relabeling,
  session-bound grants, revocation on exit, and table bounds.
- Local QEMU/OVMF runs, 2026-10-03:
  - `./nagi isolated-process`: the undeclared application was refused before
    spawn, and the forged system claim was denied by missing grant.
  - `./nagi m19` on a fresh disk: Search denied a revoked launch record with
    `Denied`.
  - `./nagi m22`: all three boots passed.
- One earlier `./nagi m22` attempt stalled in the kernel's M3 SMP scheduler
  workload before user space. That is the pre-existing intermittent stall
  recorded in the implementation status. Its log is preserved under
  `out/evidence/m22-m3-smp-stall-20261003/`; the immediate rerun passed.
