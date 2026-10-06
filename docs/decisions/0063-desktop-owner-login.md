# ADR 0063: Desktop owner account and lock-screen sign-in

Status: accepted
Date: 2026-10-06
Builds on: M11 security, ADR 0060 (trusted consent dialog), ADR-0011 (M27
readiness)

## Context

M11 proved local login, lock and Developer Mode with fixture accounts in an
acceptance program. The desktop itself never asked anyone to sign in. As a
result, two things relied on fixtures or on any boot at all:

- the consent dialog (ADR 0060) records decisions for a fixture user;
- M27 readiness is reported as soon as the desktop draws its first frame.

The M27 workstream lists "account-authenticated readiness" as open. M29
lists onboarding as open.

## Decision

1. **Credentials.** `libnagi::credential` stores only a salted
   PBKDF2-HMAC-SHA256 credential:
   - a 16-byte random salt from `SYS_RANDOM_GET`;
   - 20,000 iterations, with records accepted only within 10,000–1,000,000;
   - a 32-byte output, compared in constant time.

   The implementation is checked against the published PBKDF2-SHA256
   vectors.
2. **Owner record.** The owner account record (`owner-account` in User
   Data) is versioned and carries a SHA-256 of its body. A corrupted or
   unreadable record is never read as another account. The screen stays
   locked, and Recovery is the way out.
3. **Fixture accounts.** `AccountStore` accepts such credentials alongside
   the M11 fixture accounts. Authentication and unlock check the derived
   credential, and no password is kept after the check.
4. **Login screen.** `libnagi::login::LoginForm` is a host-tested input
   state machine:
   - **Create mode** (first run): name, password and confirmation, with
     validation for the name rules, a minimum length of 4, and a matching
     confirmation.
   - **Unlock mode**: password only, with a failure count.

   Keys map to lowercase letters, digits and `-`.
5. **Desktop flow.** With feature `desktop-login`, init's desktop draws the
   OS-owned login screen as its first frame and routes every key event to
   it.
   - It reports boot readiness only after a successful sign-in, so an M27
     trial is confirmed only by a signed-in desktop.
   - The signed-in `Session` is kept by the desktop for later
     authorizations, such as consent.
6. **Text.** Localized en-US/ja-JP strings (`login.*`) are added, along
   with the bitmap glyphs they need.

## Acceptance

`./nagi login` passed on the arm64 macOS host (evidence
`out/evidence/login-1791243371662416000`). It runs on a fresh User Data
disk.

**First boot.** The `create` login screen is captured, then real QMP keys
type the name `owner` and the password twice. The log shows:
- `owner created PASS name=owner`;
- `unlocked PASS`;
- acceptance.

**Restart.** The `unlock` screen shows the owner name. A wrong password
prints `unlock REJECTED`, and the correct one signs in.

A temporary ja-JP render of the same screens was checked visually.
`./nagi m29` and `./nagi consent` still pass.

## Rollout (2026-10-06, follow-up)

- **Consent.** The consent acceptance now enables `desktop-login`.
  - Persisted decisions are restored, and the dialog opens only after
    sign-in.
  - Decisions are recorded for the signed-in owner's `Session` instead of
    the fixture account.
  - The dialog is announced (`SHOWN`) only once its frame is presented and
    armed.
  - `./nagi consent` creates the owner, answers the dialog, then unlocks
    after a restart and sees the restored decision.
- **M27.** The M27 GPT images enable `desktop-login`. The healthy System B
  trial creates the owner through QMP input, and the acceptance requires
  `Nagi M27 readiness persisted slot=B` *after* `Nagi login unlocked PASS`.
  This is account-authenticated readiness. `./nagi m27` passed (evidence
  `out/evidence/m27-ab-rollback-1791243852197747000`).
- **Not yet enabled.**
  - The `m30-update` payload enables `desktop-login`. Its System B trial
    creates the owner, and readiness must follow sign-in.
  - The legacy FAT12 M27 fixtures and M10/M29 are unchanged.
  - The M30 release image runs the M19/M22 service flows, not the desktop.

## Bounds and non-goals

- **Rollout.** See above.
- **Single account.** There is one owner account. Standard/guest accounts,
  password change, and recovery reset are later work.
- **Rate limiting.** There is no rate limit beyond the cost of the KDF,
  and failures are only counted.
- **Keyboard.** The keyboard map is US-layout ASCII. Japanese input
  methods do not apply to credentials.
