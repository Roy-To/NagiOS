# Albert runtime owner control core

M23 PARTIAL / M30 BLOCKED. Writer and guest HOLD remain. The implementation
originated on f35d3f25320339fa69d0860c12e1a354a3852701 (tree
dcc5c1bc306a755b095c831cc16f049f794edca8). Only user/nagi-albert is changed.
Private product decisions remain unverified; this implements the existing public
0.1 browser orchestration and freshness contracts without activating new policy.

## Integration onto the current PR43 head

Read-only GitHub metadata and Git refs on 2026-11-05 resolve PR43 to
222f9930b0e8c48f1631ee92f6de1f3e6852c30e (tree
651f61bb4ad29aa12498d841d28f59a7657752e4), open/draft/unmerged, and main to
2c9532943a6624980892d01ce7e91f6d32545d25 (tree
34fcfa03ba7e19081c4937a235924591d0e92e92). Main is already an ancestor of
that PR head. Its delta from f35d3f2 touches eight M30 release/preflight/status
paths and no Albert source or AGENTS.md. No additional main merge is needed.

A new isolated worktree applies the complete seven-path patch from reviewed
local 4dc12dcbd7b58c5578f6b3fb39bbc2c970d084a1 onto that exact current PR
head without conflicts. All six code/test/harness files are byte-identical to
4dc12dc; only this note is refreshed for the new integration base. Desktop,
Bar owner/endpoint/reaper, protocols, existing lockfiles, CI and release gates
are not modified. The independent review result for 4dc12dc (no new P1/P2)
was supplied by the parent; it is not an independent review of this new tree.

The safe host checks below are rerun on the integrated source. The parent's
separate review environment also reported session-ui 76/117/26 success; that
report does not replace this executor's previously recorded missing offline
libc prerequisite. No fake authenticated source or Browser binding is added.

Integrated-source validation: Albert host116 (new owner15), existing context12,
and release/session/preflight59 PASS. All-target/all-feature locked offline
Albert check and strict Clippy, affected-source format and whitespace PASS.
The existing six code/test/harness blobs are independently compared with
reviewed 4dc12dc and match exactly. Session-ui is not rerun here; no prerequisite
or source change resolved the previous offline cache blocker in this executor.

## Source findings and implemented behavior

The fixed source has no ordinary Albert runtime owner. The normal Desktop
model-service step passes page context None in user/nagi-init/src/desktop.rs.
The guest FirstPixelBrowser and m18_acceptance engine delegates are separate
acceptance paths, not ordinary Browser ownership. This change leaves those paths
intact and does not wire Desktop or replace its None.

AlbertRuntimeOwner now owns the existing BrowserState and BrowserContextOwner
together. It exposes local control functions suitable for a later ordinary
embedder; it does not acquire actual display/input resources or construct Servo.
Its public PublicBrowserContextApi implementation always returns Unavailable.
No public source-enabling switch, extraction closure, actor constructor or raw
identity-to-authorization conversion is added.

- attach validates routing metadata and returns an opaque, event-thread-only
  Attachment. It starts unfocused with fresh tabs and sharing disabled.
  SurfaceBinding and validate_binding compare local resource/session records;
  they do not prove resource ownership, launch provenance, visibility or consent.
- detach, lock, sign_out and close drop the context epoch and browser state;
  close is terminal. Identical reattachment gets a different opaque identity,
  so old callbacks cannot regain validity even when local numeric IDs restart.
- set_focus and trusted set_sharing_by_user revoke before changing state.
  Focus loss cancels address composition without committing preedit. Sharing
  controls do not acknowledge terms or grant admission, object or model access.
- dispatch_chrome uses existing ui::dispatch for tabs, URL submission, history,
  reload, bookmarks and IME actions. A focused attachment and increasing nonzero
  input sequence are required. Accepted sequences are consumed even when the
  action fails; wrap/replay cannot repeat a tab/navigation side effect.
- primary_press uses existing chrome hit testing. The complete chrome and status
  strip consume rows before chrome_surface::PAGE_TOP (62); page coordinates
  start at zero there. Page presses blur the address bar. page_key refuses page
  delivery while the address bar has focus. InputOutcome is routing data for the
  eventual engine adapter, not evidence that an engine received an event.
- PageTicket binds output to one attachment, active tab and document generation.
  validate_page fences deferred frame/callback use; validate_page_input also
  checks page focus. Every accepted input conservatively changes the generation.
  page_will_change must run before engine content/selection mutations and returns
  the new ticket. Stale tickets cannot mutate state or write a destination frame.
- Navigation callbacks require the attachment plus the existing pending tab and
  NavigationId. redirect_navigation, complete_navigation, fail_navigation and
  content_navigated reuse BrowserState. Reload keeps the URL but changes the ID;
  old or duplicate completions are ignored without invalidating newer state.
- compose_page validates a ticket and uses existing place_page/render_chrome
  with attachment dimensions. It composes caller-owned buffers only. The real
  presenter must revalidate the ticket before presenting without an intervening
  yield. No Surface acquisition or present is performed by this API.

BrowserState and the context owner remain private to the core. Page data in the
test-only SyntheticSource is explicitly synthetic; that seam exercises real
owner transitions and BrowserContextOwner::extract, whose captured revision is
checked after extraction. The production API cannot call that seam. Actual
coherent extraction and authenticated request binding are still absent.

## Host validation

Run from the repository root:

```sh
sh user/nagi-albert/host-tests/check.sh
```

The isolated workspace compiles the actual ../src/lib.rs, including all its
host modules/tests and optional m18-acceptance host tests. It reuses existing
dependencies, pins its own in-scope Cargo.lock and runs offline. It avoids the
unavailable unrelated Servo target graph, not any Albert host source. It adds
no production dependency or changes to root/existing lockfiles.

116 tests pass, including 15 new runtime-owner tests. All-target/all-feature
locked offline check, strict Clippy (-D warnings), affected-source formatting and
diff whitespace checks pass. Coverage includes invalid/missing/foreign owner,
attachment identity ABA, session/workspace/node/surface/epoch mismatch, focus,
address IME, status-strip/page routing, repeated input, navigation/reload,
redirect/failure/content navigation, stale async callbacks, frame composition,
sharing ABA, extraction-time invalidation, detach/lock/signout/close/drop and
production source denial despite plausible raw actor data.

The existing tests/albert-context/check.sh also passes all 12 tests and strict
Clippy. The existing tests/session-ui/check.sh stops before tests because its
unchanged locked libc 0.2.191 is absent from this executor's offline cache/index
(only 0.2.190 and 0.2.189 were available). No dependency download or lock change
was used to bypass that prerequisite. No full Albert/Servo target build, final
link, guest, OS/Desktop input, IPC/authentication, Surface present, model or actual
Browser extraction ran. These host results do not establish formal acceptance.

## Required later integration

Task 1 continues to own signed Bar supply, authenticated endpoint and reaper.
This core assumes none of its future interface and must not be passed to the
normal Desktop summary path yet. Later integration needs all of the following:

1. A real ordinary Albert Servo owner on its event thread, with actual acquired
   Surface/input capabilities and live Session ownership, distinct from
   FirstPixelBrowser. Retain those resources; bind their verified dimensions,
   surface and logical metadata to attach only after validating the real record.
   Numeric SurfaceBinding fields or Attachment alone are never authorization.
2. Trusted Desktop/session lifecycle delivery before further dispatch: focus,
   workspace/session changes, resize/replacement (detach/attach), lock, signout,
   app exit. Clear engine pressed keys/capture/modifiers on focus loss and
   teardown. Route chrome IME through existing translators, deliver page input
   once on the engine thread and recheck validate_page_input for deferred events.
   Pointer motion/release/scroll, full IME, permission dialogs and other engine
   services still need the actual normal adapter; this core implements primary
   press and page-key routing only.
3. Real per-WebView/tab callback mapping. Retain Attachment and matching
   NavigationRequest; invalidate before navigation, reload, document replacement,
   frame/content/DOM/selection mutation and tab activation. Use page tickets to
   reject stale async data; never relabel old results with a newly issued ticket.
   Validate before composing/presenting and avoid a yield after final checks.
4. An authenticated context connector using the existing review-approved wire
   and kernel-stamped sender plus live Supervisor launch state. Bind the actual
   requesting Bar and target Albert to the live trusted user Session/workspace
   and active request. Bar AppSessionId is not Albert AppSessionId; do not equate
   them or use caller payloads/foreground Albert identity as the Bar actor.
5. A bounded coherent ordinary Servo extraction adapter with complete mutation
   invalidation, active-page/loading checks and request-field filtering, bound
   to this owner/ticket. Only after source and actor bindings exist may a later
   authorized integration replace the current Unavailable implementation.
   Retain all existing ContextResolver visibility, sharing, terms, budget,
   admission and cancellation gates; context availability supplies none of them.

No public push, PR/CI action, branch merge, guest/writer/model/hardware work,
private ledger read, permission change or authentication retry was performed.
