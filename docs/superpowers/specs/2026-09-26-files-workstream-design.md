# M-APP-05 Files Workstream Design

## Goal

Implement a usable, testable Files application slice with navigation, selection,
typed operations, permission enforcement, sandboxed host preview, and explicit
integration boundaries for Activity, Wayback, Search, and future Nagi storage.

## Current behavior and constraints

- `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md` defines Files
  acceptance in section 51 and M-APP-05 in Part XII.
- M7 provides low-level persistent block/VFS support, but its current user-space
  API is root-directory based and is not a general Files provider for nested
  copy/move/Trash operations or scoped app capabilities.
- M10 draws a fixed `Files` desktop client and filename; it is not a Files app.
- No shared Resource, Action, Search, Activity, or Wayback service contract is
  available to this workstream. DF-01's `.dev` registry is for the separately
  gated 0.2 runtime streams and is not this first-party status record.
- The prepared `codex/app-files` worktree started clean at `be18b02`; its
  product base is the committed DF-01 base `ab9a580` plus the first-party status
  preparation commit. The target runtime does not expose the Files provider,
  scoped capability, or shared Action/Activity/Wayback service contracts here.

## Scope and non-goals

Implement only M-APP-05 in a standalone `apps/nagi-files` package: domain and
provider contracts, navigation/selection/view state, an explicitly scoped host
sandbox provider, a deterministic in-memory provider, an explicit capability gate,
typed Files Action dispatch, Activity/Wayback/Search hooks, bilingual resources,
an interactive host preview, and focused automated tests. Do not change M17,
other first-party apps, the root Cargo workspace, third-party code, or DF-01's
registry/state.

The host preview is not Nagi target integration. Full Nagi desktop process/UI
integration and a production capability-backed filesystem adapter remain
separate follow-up dependencies; no host result will be reported as target
acceptance.

## Design

- Keep UI state independent from provider operations and render the required
  Sidebar / Resource View / Inspector layout from a view model.
- Give every provider opaque resource IDs and relative locations. Keep host
  paths sandbox-relative; reject parent traversal, absolute paths, reserved
  internal metadata paths, selected-root symlinks, and checked symlink paths.
  Host path validation does not prevent a concurrent process replacing a path
  between the check and the filesystem operation; the preview must not be
  treated as a production adversarial sandbox.
- Route mutation through an operation coordinator. Each operation carries an
  intent, precondition, transaction ID, cancellation handle, and reversible
  classification. Permanent deletion requires a one-use confirmation token.
- Enforce a scoped capability decision before returning resource data or
  performing a mutation. Trash listings are filtered by each item's original
  location. Host preview grants are explicit and scoped to the selected
  sandbox; they are not Nagi production permissions.
- Persist Trash entries under one reserved, sandbox-internal metadata store.
  Keep Activity, checkpoint, and Search as typed optional provider contracts;
  unavailable hooks remain visible in the result state.
- Use a memory provider for deterministic orchestration tests and a real
  sandbox provider for host filesystem behavior and preview.

## Verification

Run package unit tests first, including operation conflicts, copy/move rollback,
Trash/restore, sandbox traversal and symlink rejection, permission denial,
Unicode names, bounded previews, large directory selection, Action dispatch,
search filtering, hook outcomes, and localization coverage. Then run the
interactive host preview against a newly created temporary sandbox and run
package formatting/lint checks. Do not run the Nagi-wide build, Servo, Mesa, or
M17 acceptance for this workstream.
