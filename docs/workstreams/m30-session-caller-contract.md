# Ordinary session / M23 provider contract checkpoint

Status: BLOCKED for ordinary Browser summarization. The transferred M23
input/output-boundary source is integrated locally; external-app authority and
the live Browser worker route remain unimplemented.

## Source and candidate boundary

This checkpoint combines the M30 lineage through `335074f` with the source/docs
transfer identified by candidate `4ef8534a55c90b4601d7e8cbf3e612ea30531f35`.
Both transfer layers' byte counts/SHA-256, source-only tree
`a437843e3beec28ec5b7ab5a876a262cfe0c5588`, full tree
`518ca7262002444a45c3cb6ef800e245c5a657f5` and all six changed blobs were
verified against base `87b7dea`. Original commit metadata was not transferred
or independently verified; the local reconstructed commit has a distinct ID.
The transferred boundary source includes the earlier output-budget candidate;
this does not claim that its original commit object was imported. Only the
implementation-status append conflicted; both workstreams' sections were kept.
The remaining five candidate blobs match the verified transfer exactly.

## Current mismatch

- `services/nagi-ai/src/browser_context.rs` creates a `ModelRequest` with
  `caller: Some(prompt.context.caller().app_id)`, a separate trusted system
  prompt, serialized untrusted Browser page context and at most 256 output
  tokens (or a smaller trusted caller budget).
- `user/nagi-init/src/model_service.rs` requires an OS-supplied live Session.
  `ModelSessionAccess` admits only its bound unlocked token and `caller=None`.
  A request-side AppId does not establish an authenticated app identity.
- `SessionServices::submit_text` carries bounded text and its captured Session/
  generation. Its worker builds a text.generate request with caller=None,
  system_prompt=None and at most 256 output tokens. It is the OS Bar route,
  not a BrowserSummaryAdapter/GenerativeProvider app transport.
- The integrated summary adapter now checks serialized JSON plus system
  instructions against the trusted caller's input budget, capped at 24 KiB.
  The output cap is 256 or smaller. Native template/tokenizer capacity failures
  map to ContextLimit before KV clearing/prefill; the adapter maps this to
  InputTooLarge. These fixes align conservative admission bounds, but do not
  authenticate an app or supply the missing transport/system-instruction seam.

Thus accepting a model artifact alone cannot make the current ordinary Browser
summary route work. A missing-model Bar reply and dedicated M20 inference do
not establish this route.

## Required integration seam (proposal, not implemented)

The OS entry must bind the authenticated live Session/generation to the verified
app invocation, independent of fields supplied in ModelRequest/page content.
Use the existing Supervisor/kernel-stamped process identity and applicable live
grants/context resolution, then bind AppId/AppSessionId/node and the selected
page/navigation generation. Lock, Session replacement, navigation changes and
grant revocation must invalidate queued and late output as appropriate.

Normal Albert Context API resolution and the user's context-sharing permission
must authorize an immutable snapshot of exactly the selected text/visible page
fields. Model-license terms consent does not establish this sharing permission.
Preserve the context include flags and navigation binding across the asynchronous
request; neither a page-provided AppId nor an already accepted license replaces
the verified invocation and current context grant.

Keep ordinary `generate`'s external-caller rejection. A separately authenticated
app entry needs an OS-created authorization object or equivalent checked
binding; do not admit every Some(AppId), forge the Bar's None caller, reuse a
fixture allow policy, or let serialized page metadata confer authority.

The bounded worker seam must preserve trusted system instructions separately
from untrusted Browser content and carry the capability/role and coordinated
input/output/deadline budget. It must retain cancellation, output validation,
terms and exact-session checks. Model text stays inert; summarization does not
grant Files, actions, microphone, network or Owner override authority.

Before acceptance, test forged/mismatched callers, stale/locked Sessions, revoked
context, stale navigation, input/system-prompt boundaries and output/deadline
limits, then run a real Browser request through the live guest worker and pinned
model in the same ordinary image. Existing host/provider fixtures are distinct.
No 0.2+ runtime foundation is activated to bypass the 0.1 release gate.
