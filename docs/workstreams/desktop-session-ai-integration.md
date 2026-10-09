# Ordinary Desktop / Nagi Bar model integration

Status: implementation PARTIAL; ordinary guest model acceptance NOT_RUN.
M20 and M23 remain PARTIAL. The inherited formal M30 gate remains BLOCKED.

## Dependency checkpoint and ownership

The dedicated branch is `codex/0.1-desktop-ai-integration`. Its local integration
checkpoint is `045fbc731dc464721adb0da0e7af4285c0e967db`, formed without conflicts
by ordinary merges of main `69fe92d35568649638dbb1964fd680f1b32aa432` and:

- PR37 model service: `41ff80a13a9e1697c4c7d7892451558245e0a552`;
- PR42 Files/VFS: `f1aa9b2d51f9424a25f002d2fb7e0c75dbd0a59a`;
- PR43 SessionServices: `909d8ee01da5ff5f62a422ea67057b5ea27cafa1`.

All dependency heads retain their ancestry. Existing branches are unchanged.
The Desktop delta owns only `desktop.rs`, the new `session_ui.rs`,
`bar_adapter.rs`, `bar_panel.rs`, `tests/session-ui`, this technical handoff,
and appended en-US/ja-JP Bar strings. Storage/kernel/provider code, shared
main/manifests, CI, registry and other workstream paths retain checkpoint
bytes. The shared status document is preserved; this document records this
slice's validation without editing the recovery owner's status/CI work.

## Behavior

F3 or the Nagi Bar button opens the text panel after sign-in. Enter or Send
queues one request. Input events never execute native inference. The main
Desktop loop sends one bounded service action per step and polls even with no
input or with the panel closed. A request and its response are bound to the
actual OS Session and a monotonically increasing request ID. There are no
production canned responses or synthetic inference imports.

The controller admits at most 24 KiB of UTF-8 input and 64 KiB / 256 tokens of
output. NUL/empty/oversized input and invalid, empty, oversized, excess-token
or wrong-request output are rejected. Output is inert display text. Up/Down
scrolls the retained response; this slice adds no Files/action/browser authority
or implicit browser context sharing. The first input adapter uses the existing
lowercase ASCII keyboard mapping. Full IME, shift handling, shaping and complete
Japanese glyph coverage remain outside this slice; controller text itself is
UTF-8 and every control/status has both locale entries.

The OS displays pinned model name and terms reference from the same manifest
that SessionServices consumes. Opening, typing and submitting do not acknowledge
terms. Tab cycles Input, Send, Cancel and the explicit session-terms action when
needed. Explicit acceptance queues only `acknowledge_model_terms`; a separate
Send is still required. Consent is ephemeral for the current Session, cleared
on lock/signout, and never described as durable Store consent. Submit terms
errors and worker license errors return the UI to the explicit terms gate.

F4 or Lock invalidates/cancels inside the Desktop event handler before locking
and erasing its Session. This also lets the existing nested Files consent loop
observe the invalidated Session immediately. Pending work, terms and displayed
responses are cleared before the lock screen appears. New sign-in reaches the
service only after `report_boot_ready` succeeds. Failed worker binding is shown
without repeatedly spawning on idle steps. An ordinary production Desktop never
enters the old milestone-focus acceptance halt.

The initial-frame equality failure is also restricted to builds without
`production-session`, matching the legacy acceptance configuration. An
existing-account boot, sign-in and F4 lock in the same locale reconstructs the
initial blank unlock form exactly; returning to that form is valid during an
ordinary session. Production builds omit this fixture failure path. Display
presentation, explicit boot readiness and sign-in readiness checks retain their
original behavior, and the fixture assertion remains active in acceptance builds.

Cancellation retains the controller's occupied slot until the bridge's terminal
result is drained; all cancellation results, including late Ready, are hidden.
A second submit stays blocked during that drain. Session replacement/lock uses
the bridge's generation invalidation. Request IDs survive lock/relogin, including
relogin with the same bootstrap token, so an old response cannot match a new ID.
All ordinary idle/locked steps explicitly yield to the guest scheduler.

## Admission and remaining limits

The product adapter supplies a configured 3072 MiB model admission policy inside
the existing `m20-llama-memory` 3584 MiB POSIX heap, excluding 512 MiB from model
admission for other native allocations. It admits 2560 MiB of read-only model
storage and one CPU, with no GPU. These are policy limits, never VM total RAM or
invented free-memory measurements. The inherited allocator does not implement
per-worker enforcement; measured containment, isolation and pressure acceptance
remain open. Construction performs metadata configuration only. The only worker
capability is the existing boot-provided read-only Model Store handle.

Normal model inference, native final image linking, visual guest input/display,
lock/relogin under active native inference and model-memory measurements are
NOT_RUN here. This cloud workspace has no native production image/archive/model
artifacts or QEMU installation for that acceptance. Durable terms storage and
the broader M23 browser summarization acceptance remain pending. No milestone
PASS, release, security setting or license-gate relaxation is claimed.

## Validation (cloud Linux x86_64, pinned nightly-2025-08-01)

- `tests/session-ui/check.sh`: PASS. 36 host orchestration tests (25 UI/adapter
  and 11 inherited bridge tests), host warnings-denied Clippy and formatting.
- Real Desktop regressions: PASS, three production and two legacy acceptance
  host tests. The pre-fix production run failed all three checks, including an
  exact initial/locked frame collision in the real renderer. The fixed run
  covers both locales, lock/relogin, failed readiness and preserved login
  acceptance. Only syscall transport/readiness/time/console are host seams;
  credential verification, VFS, login events and rendering are actual source.
  The host syscall-boundary crate also passes warnings-denied Clippy.
  The legacy login unit configuration omits Files runtime because inherited
  Files host-test initializers omit acceptance-only fields; no production
  feature dependency or Files source is altered to accommodate this harness.
  A separate host compile-only check of the full legacy login/Files acceptance
  source also passes; it does not execute the inherited acceptance fixtures.
- Actual Nagi service/worker/controller/adapter leaves: warnings-denied Clippy
  PASS for `targets/x86_64-unknown-nagi-user.json`, using build-std core/alloc.
- Actual Desktop/login/Files source check with `desktop-check`: compile-only
  PASS. This harness does not enable signed Files IPC/consent features and does
  not link native C++ archives. Existing POSIX/Files dead-code warnings remain
  visible, plus test-facing controller methods unused by the nested Desktop.
- Preserved Files standalone suite: 30 host tests PASS.
- Preserved release/session-evidence regressions: 37 Python host tests PASS.
- Owned-delta whitespace, paths and dependency ancestry: PASS.
- Actual normal-session model/guest acceptance: NOT_RUN.

The standalone check script is ready for the shared CI owner to adopt; it is
not silently registered in shared CI. Existing CI gates, including the formal
M30 blocker, are preserved. CI status at the published head is reported in the
PR separately from these focused local results.
