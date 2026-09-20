# M11 Login / Permissions / Security Design

## Goal

Deliver a bounded local account/session and Permission Broker path that proves
an untrusted app cannot obtain file or microphone access, including while
Developer Mode is enabled.

## Architecture

M11 is user-space policy code in `libnagi`; the kernel keeps its existing
low-level capability checks and receives no high-level permission syscalls.
`AccountStore` authenticates fixed-size local accounts, creates a lockable
session, and permits Developer Mode only through an authenticated Owner action.
`PermissionBroker` evaluates role, app trust, requested resource, and explicit
consent. Untrusted apps are denied for file and microphone access regardless of
Developer Mode. The M11 guest acceptance app exercises these real policy
objects inside the existing bootstrap user process because general process
spawning is not yet available.

## Data flow

```text
local account records -> authenticate -> Session
Session + AppIdentity + PermissionRequest -> PermissionBroker
  -> Allow / Ask / Deny
untrusted file/microphone request -> Deny -> no VFS or audio operation
```

The policy is fail-closed. Passwords are represented only by a deterministic
bounded hash in the in-memory test store; no host credential store is used.
The microphone capability is a policy resource in M11 and remains denied until
the real VirtIO Sound service exists in M14.

## Acceptance markers

The guest prints successful local login and lock/unlock, Owner Developer Mode
enablement, trusted-dialog `Ask`, malicious file `DENIED`, malicious
microphone `DENIED`, and `Nagi M11 acceptance PASS`. The host validates only
these guest-produced markers and ordered log evidence.

## Constraints

- No kernel filesystem, microphone, UI, or permission syscall is added.
- Developer Mode does not disable capability or Permission Broker checks.
- No host file, microphone, or credential APIs are used.
- No test assertion is weakened to obtain PASS.
