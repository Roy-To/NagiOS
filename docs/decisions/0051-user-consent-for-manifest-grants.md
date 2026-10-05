# ADR 0051: User consent for manifest grants

Status: accepted
Date: 2026-10-03
Builds on: ADR 0046 and ADR 0049

## Context

Since ADR 0046/0049, a capability named by a signed manifest's `grant=`
line became effective as soon as a live session of that application
existed. The package author alone therefore decided what an application
could do. Spec §23 asks for Allow / Ask / Deny / Allow once decisions made
by the user through OS-owned dialogs, and §83.2 forbids skipping the
permission broker.

## Decision

- **Grants are requests.** A manifest `grant=` line is now only a request.
  `LaunchRegistry::check_grant` returns `Granted` only when:
  1. the application session is live;
  2. its signed manifest requests the capability;
  3. the user's recorded decision allows it.

  Otherwise it returns `NotLive`, `NotDeclared`, `ConsentRequired`, or
  `Denied`. `has_grant` is `check_grant == Granted`, so every existing
  service check (Search `search.query`, the M19/M21/M22 action policies)
  now also requires consent.
- **Decisions.** `GrantDecision` is one of:
  - `Ask`: the default; recording it withdraws an earlier decision;
  - `Allow`: covers every session of the application;
  - `AllowOnce(AppSessionId)`: covers one *live* session and is dropped
    when that session's launch exits, so a relaunch of the same session ID
    asks again;
  - `Deny`: overrides the manifest.

  Decisions are bounded (`MAX_CONSENT_DECISIONS = 16`). Replacing a
  decision needs no new slot.
- **Who decides.** `record_decision` requires an authenticated, unlocked
  `security::Session`, which only `AccountStore::authenticate` can make. A
  locked session gets `ConsentUnavailable`. The registry lives in init's
  Supervisor, and launched processes have no route to it. Developer Mode and
  the Owner role do not imply consent. A decision for a capability the
  manifest does not request creates nothing (`NotDeclared` still wins).
- **Fail closed.** `ConsentRequired` is treated as denied by services. The
  trusted consent dialog, which would turn it into a prompt, is not wired
  yet.

## Acceptance

`./nagi isolated-process` adds `grant=acceptance.consent-probe` to the
signed faulting-app manifest. With two live sessions of that application, it
verifies:

- the grant is `ConsentRequired` before any decision;
- an undeclared capability is `NotDeclared`;
- a locked user session cannot record consent;
- `AllowOnce` for the first session grants it and not the second.

After that session faults and is reaped, the same session ID relaunched is
`ConsentRequired` again. `Deny`, `Allow`, and `Ask` then each take effect.
The markers are `Nagi Supervisor grant consent required PASS` and
`Nagi Supervisor grant decisions PASS`.

The M19/M21/M22 scenarios record the acceptance user's `Allow` decisions
(`supervisor::record_acceptance_consents`) before their first launch.

## Bounds and non-goals

- The acceptance decisions are made by a fixture account
  (`supervisor::acceptance_user`) in place of the trusted dialog UI. They are
  explicit inputs, not inferred consent.
- Decisions live only for the boot; persisting them in User Data is later
  work.
- Foreground/background distinctions and per-object (selected-file)
  consent are not yet modeled.
