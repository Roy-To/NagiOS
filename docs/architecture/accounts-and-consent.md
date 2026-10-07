# Accounts and Consent

This document describes who is signed in to a Nagi 0.1 desktop and how that
user's decisions authorize applications. The decisions are
[ADR 0049](../decisions/0049-signed-package-launch-manifests.md),
[ADR 0051](../decisions/0051-user-consent-for-manifest-grants.md),
[ADR 0060](../decisions/0060-trusted-consent-dialog.md) and
[ADR 0063](../decisions/0063-desktop-owner-login.md). Spec §23 (permission
model) and §24 (Developer Mode) are the requirements.

## Owner account

**First run.** With `desktop-login`, the first desktop frame is an
OS-owned onboarding screen drawn by init:

1. The user chooses the system language (English / 日本語).
2. The user creates the owner account: a name, then a password entered
   twice.

**Later boots.** The same screen unlocks the owner account.

**Stored credential.** Only a credential is persisted, never a password:

- a random 16-byte salt from `SYS_RANDOM_GET`;
- the iteration count;
- the PBKDF2-HMAC-SHA256 output.

They live in a versioned, SHA-256-checksummed `owner-account` record in
User Data. Verification is constant-time.

**Corruption.** A corrupt record fails closed: nothing can sign in, and
Recovery is the way out.

**Throttling.** After three failed unlocks, attempts wait 5 s, doubling up to
60 s. Attempts during the wait are refused without running the key
derivation, and the failure count survives restarts
([ADR 0064](../decisions/0064-login-attempt-throttling.md)).

**Password change.** A signed-in owner can change the password in Settings.
The current password is checked first, against the same persisted throttle,
and the new record replaces the old one atomically
([ADR 0066](../decisions/0066-owner-password-change.md)).

**Readiness.** The desktop reports boot readiness only after sign-in. See
[boot-trust-and-system-updates.md](boot-trust-and-system-updates.md).

The input state machines (`libnagi::login`) and the credential format
(`libnagi::credential`) are host-tested. The screen itself, its rendering
and its key routing belong to init. Launched applications have no route to
it.

## What an application may ask for

An isolated application launches only from a signed M16 package (ADR 0049).
The Supervisor takes the application's identity and its `grant=` requests
from the signed manifest, never from the process.

A capability is effective only when all of these hold:

1. the application session is live;
2. its signed manifest requests the capability;
3. the signed-in user's recorded decision allows it.

Otherwise `check_grant` returns one of:

- `NotLive`;
- `NotDeclared`;
- `ConsentRequired`;
- `Denied`.

Services treat anything but `Granted` as denied. Developer Mode and the
Owner role do not imply consent.

## Deciding

A `ConsentRequired` use queues a bounded prompt in the Supervisor. The
desktop shows the **OS-owned consent dialog** (Deny / Allow once / Allow).
It is modal, localized, and drawn by init over every application.

**Input rules** (`libnagi::consent::ConsentPrompt`, host-tested):

- Input counts only after the dialog frame is presented.
- A choice needs both a press and its release, on the same key or button.
- Focus starts on Deny.
- Escape records nothing.

**Recording.** The answer is recorded with the signed-in owner's
`Session`.

**Review and withdraw.** Settings → Permissions lists the recorded
decisions. Withdrawing one records `Ask` and persists it, so the next use
prompts again ([ADR 0065](../decisions/0065-consent-settings-view.md)).

**Persistence.** `Allow` and `Deny` are written to User Data
`consent-decisions`:

- the file is versioned and FNV-checksummed;
- it is restored all-or-nothing at sign-in;
- `Allow once` lasts only for that application session.

## Acceptance

| Command | What it proves |
| --- | --- |
| `./nagi login` | Language choice, then owner creation; after a restart, a wrong password is refused and the right one unlocks, in the chosen language |
| `./nagi consent` | Sign-in, then the dialog for a signed app's requested capability; a press released off a button decides nothing; Allow is persisted and restored without a prompt after the next sign-in |
| `./nagi isolated-process` | Grants need consent; `AllowOnce` is scoped to one live session; Deny, Allow and Ask each take effect |

## Open work

- Standard and guest accounts, and Recovery reset. (Password change is in
  Settings, ADR 0066; its guest acceptance is pending.)
- Production services that queue prompts. Today the acceptance application
  does.
- Foreground/background and selected-file consent.
