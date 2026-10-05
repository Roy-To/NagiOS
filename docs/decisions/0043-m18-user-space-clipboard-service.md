# ADR 0043: M18 user-space clipboard service

Status: accepted for M18
Date: 2026-10-05
Milestone: M18 — Albert Browser

## Context

M18 requires clipboard support in Albert. Servo's embedder clipboard hook
(`ClipboardDelegate`) receives `GetClipboardText`/`SetClipboardText` without
saying whether the request came from a trusted user shortcut or from page
script. The pinned Servo `navigator.clipboard.readText()` has no permission
check (its step 3.1 is a TODO); it is unreachable today only because the
`dom_async_clipboard_enabled` preference defaults to off. A provider that
answered every request would let any page read the clipboard once that
preference changes.

The kernel must not gain a clipboard syscall (AGENTS.md kernel boundary), and
browser content must not reach Nagi system APIs directly.

## Decision

- Add `user/nagi-clipboard`, a `no_std` + `alloc` user-space service with a
  bounded UTF-8 payload (64 KiB), at most 8 clients and 32 live gesture
  records.
- Split authority into a `ClipboardEndpoint` (READ/WRITE rights, attenuable,
  never strengthened) and a `GestureSource` that only the client's trusted
  input path holds. Init owns the `ClipboardServiceOwner` and registers
  Albert.
- A read requires READ and consumes a one-shot paste gesture recorded for the
  same client and scope; a write requires WRITE and an unexpired activation
  gesture in that scope. Scopes are per tab, with a tagged ID space that can
  never alias Albert's chrome scope. Navigation forgets a tab's pending
  gestures.
- Albert records a paste gesture only when it forwards a real Ctrl+V device
  event to the active tab, and an activation for other key presses and page
  clicks. Servo content never sees the gesture source.
- The guest uses a 5 s paste grant and 10 s activation window (Nagi timer
  ticks at about 100 Hz).

## Consequences

- Clipboard access is gesture-bound even if Servo's async clipboard
  preference is enabled later.
- In Nagi 0.1 Albert and the service run in the same guest process, so the
  endpoint is an in-process handle rather than a Channel IPC capability.
  Moving the service behind authenticated Channel transport is part of the
  same production IPC work still open for M19 and M21.
- `./nagi m18` now verifies a real QMP-driven copy/paste and an ungestured
  read denial in QEMU.

## Relationship to CLIP-01 (`crates/nagi-clipboard-core`, Nagi 0.2)

CLIP-01 is the 0.2 host-only clipboard / data-transfer contract: a typed
multi-representation content model, generations, and an injected
`ClipboardAuthorizer` seam keyed by `CallerContext`, with no target service
and no gesture policy. `nagi-clipboard` is the 0.1 guest service: text only,
and its substance is the user-gesture authority (one-shot paste grants and
activation-gated writes per scope). The two are complementary, not
alternatives:

- When CLIP-01 is adopted on the target, its service replaces
  `nagi-clipboard`'s text store and content model.
- `nagi-clipboard`'s gesture rules become a `ClipboardAuthorizer`
  implementation layered on the `clipboard.read` / `clipboard.write`
  permission decision: the permission says whether a caller may ever use the
  clipboard, the gesture says whether this particular read or write was
  requested by the user.
- Until then no second content model is added to 0.1, and CLIP-01 is not
  modified by the 0.1 work.
