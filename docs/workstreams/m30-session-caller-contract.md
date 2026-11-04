# Ordinary session / M23 provider contract checkpoint

Status: BLOCKED for ordinary Browser summarization. Investigation only; no
external-app authority is enabled and no summary budget candidate is merged.

## Source and candidate boundary

This checkpoint inspects the local integration lineage through `d3a4ea2`.
The separate budget candidate `ed67d36954e520122ee889715846ad513bd0ccdf` was
reported as preparing independent review. It is absent locally; a read-only
fetch from origin returned `not our ref`. Its implementation and tests are
unverified here. The observations below describe the current source, not a
finding that the uninspected candidate still has the same budget behavior.

## Current mismatch

- `services/nagi-ai/src/browser_context.rs` creates a `ModelRequest` with
  `caller: Some(prompt.context.caller().app_id)`, a separate trusted system
  prompt, serialized untrusted Browser page context and 2048 output tokens.
- `user/nagi-init/src/model_service.rs` requires an OS-supplied live Session.
  `ModelSessionAccess` admits only its bound unlocked token and `caller=None`.
  A request-side AppId does not establish an authenticated app identity.
- `SessionServices::submit_text` carries bounded text and its captured Session/
  generation. Its worker builds a text.generate request with caller=None,
  system_prompt=None and at most 256 output tokens. It is the OS Bar route,
  not a BrowserSummaryAdapter/GenerativeProvider app transport.
- The current summary adapter checks JSON input against 24 KiB; the service
  checks combined system+input against 24 KiB. Its 256-token ceiling also
  differs from the current adapter's 2048 request. A future budget fix must
  coordinate with actual service limits; it cannot silently relax them.

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
