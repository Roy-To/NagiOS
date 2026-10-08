# ADR 0066: Changing the owner password from Settings

Status: accepted (implementation and guest acceptance passed, 2026-10-07)
Date: 2026-10-07
Builds on: ADR 0063 (desktop owner login), ADR 0064 (login throttling)

## Context

ADR 0063 named password change as later work. The only way to change the
owner password was to delete User Data. ADR 0064 already throttles failed
sign-ins (so "rate limiting" is no longer open), but a signed-in desktop
offered a second place to guess the password with no limit.

## Decision

1. **Entry.** A signed-in owner opens Settings and chooses "Change
   password" (en-US) / 「パスワード設定」(ja-JP) under the language options
   (and after Permissions when that view is built). It is offered only with
   `desktop-login`, and only after sign-in.
2. **Form.** `libnagi::login::PasswordChangeForm` is a host-tested input
   state machine with three fields: current password, new password, and
   confirmation. Tab moves, Enter advances or submits, Backspace edits and
   Escape cancels.
   - A new password shorter than 4 bytes, one that does not match its
     confirmation, or one equal to the current password is refused. Only the
     new fields are cleared; the current password stays.
   - A wrong current password, a throttled attempt or a storage failure
     clears every secret.
3. **Check before write.** The desktop verifies the current password against
   the stored credential (constant time) before deriving anything. Wrong
   attempts use the same persisted failure count and wait schedule as the
   lock screen (ADR 0064), so the change form cannot be used to bypass the
   throttle. A correct current password resets the count.
4. **New credential.** A fresh 16-byte salt from `SYS_RANDOM_GET`,
   PBKDF2-HMAC-SHA256 with the default iteration count, and the same owner
   name. The old salt is never reused.
5. **Atomic save.** The new record is written to `owner-account-next`,
   flushed, and swapped in with `Vfs::replace` (one directory-block update),
   then flushed. A crash leaves either the old or the new record readable,
   never a missing or half-written `owner-account`. A stale
   `owner-account-next` is overwritten by the next attempt.
6. **Session.** The signed-in `Session` is unchanged: it carries no
   password, and the change does not sign the owner out.
7. **Text.** New localized strings use only glyphs the bitmap font already
   has. The ja-JP wording is therefore terse: 「パスワード設定」,
   「今の」(current), 「今の一致」(same as current) and 「設定不可」
   (not saved). Better wording needs new glyphs and is later work.

## Markers

The guest prints `Nagi password change READY` when the form opens,
`REJECTED current` for a wrong current password,
`throttled remaining_ms=…` while waiting, `PASS` once the new record is
stored, and `cancelled` on Escape.

## Verification

- `libnagi::login` host tests cover the form (valid change, each refusal,
  secret clearing, cancel).
- `nagi-init` type-checks for the x86_64 Nagi user target with
  `desktop-login`, with and without `consent-dialog-acceptance`.
- **Guest acceptance passed:** `./nagi login` on QEMU exercised owner
  creation, persisted sign-in throttling, rejection of a password change
  with the wrong current password, a successful change, and a restart that
  refused the old password and accepted the new one. Evidence:
  `out/evidence/login-1791375220649789000/` (`password-change.log` and
  `verify-password.log`).

## Bounds

- One owner account; more accounts and Recovery reset remain later work.
- The throttle is local to the guest, as in ADR 0064; someone who can rewrite
  User Data offline can reset it.
- The keyboard map is US-layout ASCII (ADR 0063).
