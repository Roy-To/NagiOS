# ADR 0065: Reviewing and withdrawing consent decisions in Settings

Status: accepted
Date: 2026-10-06
Builds on: ADR 0060 (trusted consent dialog), ADR 0063 (desktop owner login)

## Context

ADR 0060 persisted `Allow` and `Deny` answers. The owner then had no way to
see them or take one back, short of deleting User Data. Spec §23 asks for
user-controlled Allow / Ask / Deny decisions.

## Decision

1. **Listing.** `LaunchRegistry::decisions` lists the recorded decisions
   (`DecisionView`). Each entry carries the application's `AppId`, the
   signed manifest identifier when that application is registered this
   boot, the capability and the decision. The listing is host-tested,
   including that a withdrawn decision disappears.
2. **Permissions view.** When consent is enabled, init's Settings panel
   gains a **Permissions** entry.
   - It sits after the two language options, in both the Tab order and
     the Up/Down order.
   - It opens an OS-owned list of decisions
     (`user/nagi-init/src/consent_settings.rs`). Each row shows the
     application (identifier, or the `AppId` in hex), the capability and
     the localized decision.
   - Up, Down and Tab move between rows; Escape closes.
3. **Withdrawing.** Enter or Space withdraws the focused decision.
   - It records `Ask` for the signed-in owner's `Session`.
   - It rewrites `consent-decisions` in User Data immediately.
   - The next use of that capability prompts again.
4. **Isolation.** Launched applications cannot read or change the view.
   Without the consent feature, the Settings panel and its focus order are
   unchanged (M29).
5. **Markers.** As with the consent dialog, the view is announced
   (`Nagi consent settings OPEN decisions=N`) only after its frame is
   presented.

## Acceptance

`./nagi consent` passed on the arm64 macOS host (evidence
`out/evidence/consent-dialog-1791292190602639000`). It runs two boots.

**First boot.**
1. Language and owner creation.
2. The dialog appears; Allow is chosen and persisted.

**Restart.**
1. Unlock, then `decision restored PASS decision=allow`.
2. QMP keys open Settings, move to Permissions, and open the view:
   `OPEN decisions=1`. The view is captured in `permissions-list.png`.
3. Enter withdraws the decision:
   `decision withdrawn PASS capability=acceptance.consent-probe`.
4. `withdrawn grant asks again PASS`: the live session's grant is
   `ConsentRequired` again.

`./nagi m29` and `./nagi login` still pass.

## Bounds

- **Withdraw only.** The view withdraws to `Ask`. Switching a decision
  directly between Allow and Deny is left to the next prompt.
- **Visible rows.** Five rows are visible, and focus scrolls through up to
  16 decisions.
