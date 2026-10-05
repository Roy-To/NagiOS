# ADR 0053: Trusted consent dialog and persisted grant decisions

Status: accepted
Date: 2026-10-06
Builds on: ADR 0046, ADR 0049, ADR 0051

## Context

ADR 0051 made a signed manifest's `grant=` line only a request. A grant
becomes effective once an authenticated, unlocked user decides. It left
three items open:

- the trusted consent dialog that turns `ConsentRequired` into a prompt
  (until now, acceptance decisions came from a fixture account);
- persisting decisions across restarts;
- foreground/background distinctions.

Spec §23 requires Allow / Ask / Deny / Allow once decisions, and requires
that "Trusted permission and credential dialogs are OS-owned".

## Decision

1. **Prompt queue in the registry.** `LaunchRegistry::request_consent`
   queues a `ConsentRequest` only when `check_grant` is `ConsentRequired`.
   Every other state is returned unchanged and no prompt exists.
   - The request carries the signed manifest's identifier and the
     capability. The launched process supplies none of its fields.
   - Repeated requests share one prompt.
   - The queue is bounded (`MAX_PENDING_CONSENTS = 4`). A full queue fails
     closed.
   - A prompt is dropped when its session exits.
2. **Resolving a prompt.** `resolve_consent` closes a pending prompt with an
   authenticated, unlocked user's decision. `None` (the dialog was
   dismissed) records nothing, so the capability stays `ConsentRequired`.
3. **Prompt input rules.** The prompt is `libnagi::consent::ConsentPrompt`,
   a pure, host-tested input state machine:
   - **Fresh input only.** Every event is ignored until the desktop has
     presented the dialog frame (`arm`). After that, a choice needs both a
     press and its release while armed, on the same key or the same
     button. A key or button held before the dialog appeared cannot answer
     it.
   - **Safe default.** Focus starts on `Deny`, so a stray Enter denies.
   - **Keys.** Tab cycles Deny → Allow once → Allow. Escape dismisses
     without a decision.
4. **OS-owned dialog.** The dialog is drawn by init's desktop
   (`user/nagi-init/src/consent_dialog.rs`) above every application. It is
   modal: while it is open, every key and button event goes to it.
   - Its text uses the shared localization catalog (`consent.dialog.*`) in
     en-US and ja-JP. The bitmap font gained the glyphs this text needs.
   - Launched processes have no route to the dialog, its state, or the
     decision API.
5. **Persistence.** `Allow` and `Deny` are written to the User Data file
   `consent-decisions` with `encode_decisions`. `AllowOnce` is per session
   and is never written.
   - **Format.** The file has a `nagi-consent 1` header. Each line is
     `allow|deny <AppId hex> <capability>`, and an `end <FNV-1a 64>`
     checksum line follows. At most 932 bytes, it fits one User Data
     small file.
   - **Restoring.** At boot, `restore_decisions` applies the file for the
     signed-in user. It is all-or-nothing: malformed or corrupted bytes,
     or a locked user, apply nothing, so every grant asks again.

## Acceptance

`./nagi consent` builds a desktop image with the signed acceptance packages
and feature `consent-dialog-acceptance`. On a fresh User Data disk, it runs
two real QEMU boots.

**First boot.** The Supervisor launches the signed
`org.nagi.acceptance.faulting-app`, whose manifest requests
`acceptance.consent-probe`. The grant is `ConsentRequired`, so the dialog is
part of the first desktop frame.
- **Before input.** QMP captures a screenshot. Then QMP sends real input:
  1. a pointer press on Deny, released off the button, which decides
     nothing;
  2. Tab, Tab;
  3. Enter, as a press and a release.
- **Markers.** `Nagi consent dialog SHOWN …`, `decision PASS
  decision=allow`, `decision persisted PASS`, and `Nagi consent dialog
  acceptance PASS`.

**Restart.** The same disk restores one decision. The relaunched session
is already `Granted`, so no dialog appears. The markers are
`Nagi consent decisions restored count=1` and `Nagi consent decision
restored PASS decision=allow`.

## Bounds and non-goals

- **Fixture user.** The signed-in user is still the fixture account
  (`supervisor::acceptance_user`), because the desktop has no login UI yet.
  The decision itself now comes from real input on the OS-owned dialog.
- **Single-user file.** User Data is one user's volume in Nagi 0.1, so the
  decision file is not keyed by user.
- **Integrity, not authenticity.** The checksum detects corruption, not
  tampering. Applications have no route to init's User Data volume.
- **Only Allow is accepted.** The acceptance drives `Allow`. `Deny`,
  `AllowOnce`, dismissal, and the fresh-input rules are covered by host
  tests in `libnagi`.
- **Not yet covered.**
  - Services other than the acceptance do not queue prompts yet.
  - Foreground/background distinctions and per-object (selected-file)
    consent remain later work.
  - There is no settings UI to review or withdraw persisted decisions.
- **Frame-change check.** The desktop's "a handled event changes the frame"
  check now hashes every pixel, not every 17th one. A moved 3×3 pointer
  could miss every sampled pixel and fail the old check. The printed
  surface checksum is unchanged.
