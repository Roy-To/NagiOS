# ADR-0012: M27 Recovery Boot Selection and Journal Semantics

**Status:** Accepted for the M27 recovery slice

**Date:** 2026-09-30

## Context

M27 already persists System A/B selection and consumes a guest readiness record
before promoting a trial. A recovery path must remain usable when either system
payload is malformed, and selecting Recovery must not spend a trial attempt or
alter the pending/confirmed journal. The current data-volume mount helper can
format an unrecognized device, which is unsafe in Recovery and will also be
unsafe when a GPT disk is introduced.

M30 adds the reference GPT System A, System B, and Recovery partitions. The
release loader must apply the same durable boot policy to those partitions
without creating a synthetic update on a new installation. The existing M27
acceptance fixture needs an initial pending System B trial to exercise rollback
and promotion, but that test setup is not a production updater.

## Decision

- Before loading a system image, the M27 loader presents System A, System B,
  and Recovery choices for a bounded interval. On timeout it keeps the existing
  automatic A/B policy.
- The M30 release loader enables the production `m27-ab-slot-boot-control`
  feature for the GPT System A/B/Recovery partitions. An empty journal boots and
  confirms System A under the existing policy; it does not stage System B.
- `m27-ab-slot-acceptance` includes `m27-ab-slot-boot-control` and adds only the
  acceptance fixture's empty-journal pending-B seed. It is not enabled in the
  release image and is not an update mechanism.
- The loader consumes a valid readiness record before showing the menu, so a
  guest that reached the readiness gate is confirmed even if the operator then
  enters Recovery.
- Recovery does not call `begin_boot`, `stage_update`, or `mark_boot_success`.
  It receives separate kernel and init payloads. The Recovery init checks the
  persistent data volume with the read-only VFS checker, then uses a
  no-format mount only when that check succeeds.
- Manually selecting the confirmed slot leaves a pending update and its retry
  count unchanged. Selecting the pending candidate goes through `begin_boot`
  and consumes one persisted attempt. An unstaged inactive slot is unavailable.
- Recovery's bounded local console provides help, integrity check, current
  kernel-log display, root-file listing, and explicit Undo of a valid NH16
  transaction. Undo persists `UndoPending` before changing files and persists
  `Undone` after flush. It uses the checksummed caller context already recorded
  in NH16; physical console input is the operator action required to start it.
- The log command can show only the current kernel's bounded volatile log ring;
  it does not claim to recover prior-boot serial logs. The current VFS checker
  is read-only and format-specific. Recovery does not attempt automatic repair
  of corrupt filesystems.
- The Recovery payloads live under `EFI/NAGI/RECOVERY` in the current FAT
  acceptance fixture. M30 will place the Recovery environment in its dedicated
  GPT Recovery partition; that packaging change does not alter the boot-menu
  or journal rules.

## Consequences

- Recovery remains independent of A/B system payload validity and leaves the
  boot journal untouched when selected.
- M27's persistent journal policy now selects payloads from the M30 GPT
  partitions. An authenticated updater and authenticated slot manifests are
  still required before an installation can stage a production update.
- A failed data-volume integrity check leaves the device unmounted and
  unmodified while log and help remain available.
- Human-directed Undo is recoverable across restart through the existing
  NH16 two-slot archive. Unsupported archive states and corrupt filesystems
  fail closed.
- Recovery does not create update manifests, authenticate new system images,
  or supply a general-purpose ext2 repair utility; those remain separate M27
  and M30 acceptance work.
