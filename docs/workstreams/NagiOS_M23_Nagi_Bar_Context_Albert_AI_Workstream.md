# Nagi OS M23 — Nagi Bar / Context / Albert AI

**Status: PARTIAL — bounded AI context/provider boundary implemented.**

## Implemented slice

- `services/nagi-ai` now carries the logical caller identity (`AppId`,
  `AppSessionId`, originating `NodeId`, and optional `WorkspaceId`) plus an
  optional selected `ObjectId`. The selected object and any workspace are
  checked through the trusted `ContextAuthority`; hidden selections and
  unauthorized workspaces are rejected before Browser Context API access.
- Browser page data enters AI only through `PublicBrowserContextApi`, with an
  explicit `SummarizeCurrentPage` purpose. The API implementation owns user
  policy checks. Its snapshot is converted to a separate
  `UntrustedBrowserContext`, bounded by UTF-8-safe byte limits, and never
  becomes an Object ID or authority grant.
- Planner and page-summary provider inputs include the logical context and
  explicitly label browser data `untrusted`. Provider instructions treat URL,
  title, selected text, and visible text as data rather than instructions.
- `summarize_current_page` fails closed when the API is denied/unavailable,
  there is no active page or usable page text, no provider exists, the provider
  fails, or the result is blank, oversized, or has a stale/mismatched request
  ID.
  It does not synthesize a summary when inference is unavailable.

## Verification

Using the pinned Rust toolchain and dedicated M23 target directories:

```sh
PATH=<local-path> \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=<local-path> \
RUSTDOC=<local-path> \
CARGO_TARGET_DIR=<local-path> \
<local-path> test --locked --offline -p nagi-ai
```

Result: 23 unit tests passed; doc tests passed (none defined). Added tests cover
API routing, selected-object denial, untrusted provider input, bounded UTF-8
context, workspace and object authorization, API/provider unavailability,
empty page content, and invalid summary results. Test providers are
orchestration fixtures and do not claim real model inference.

```sh
PATH=<local-path> \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=<local-path> \
RUSTDOC=<local-path> \
CARGO_TARGET_DIR=<local-path> \
<local-path> -Z build-std=core,alloc check \
  --manifest-path Cargo.toml -p nagi-ai \
  --target targets/x86_64-unknown-nagi-user.json --locked --offline
```

Result: the updated `nagi-ai` service compiled for the Nagi `no_std` target.

```sh
PATH=<local-path> \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=<local-path> \
RUSTDOC=<local-path> \
CARGO_TARGET_DIR=<local-path> \
<local-path> clippy --locked --offline \
  -p nagi-ai --all-targets -- -D warnings
```

Warnings-denied Clippy and `cargo fmt --package nagi-ai -- --check` also
passed. Cargo emitted the existing three `unexpected_cfgs` warnings from
vendored `third_party/libc-servo` in host commands; they did not affect the
results.

## Remaining M23 work

- Albert does not yet implement the public Browser Context API against live
  Servo state. The current API and provider tests use an explicit test fixture;
  they do not claim a real page-text extraction path.
- The API's user-policy check and caller identity still need binding to
  authenticated guest service/IPC context. The current `ContextAuthority` is
  an interface, not the guest policy service.
- The unified Nagi Bar, its current-app/selection/workspace presentation, and
  the user-facing “Summarize this page” action are not connected to this
  service. The M20 target inference service is also unavailable, so no live
  summary acceptance was run.

Therefore the M23 acceptance criterion (“Summarize this page” through the
public Browser Context API) remains unmet; this slice advances the fail-closed
contract but does not claim M23 complete.

## Completion Sweep — shared IPC and capability boundary audit (2026-10-02)

The cross-milestone audit in `docs/implementation_status.md` confirms that
`ContextAuthority` remains an interface and the shared service layer does not
authenticate a caller or deliver capabilities across isolated processes.
Albert page context must stay filtered by a trusted provider when a production
service path is added. Caller-supplied identity and local function-pointer
dispatch do not establish that authority; M23 remains `PARTIAL`.

## 2026-10-08 normal-session provider service boundary

The new Model Manager service can retain the proven Granite backend across
ordinary requests; a guest leaf binds it to one OS-owned authenticated desktop
Session. The shared owner still needs to connect the normal Nagi Bar, maintain
the live Session, cancel on lock/sign-out, and discard late responses. This
in-process adapter does not authenticate external app IPC or establish AI
process isolation. Generated text receives no execution authority.

The existing page-summary provider requests 2048 output tokens, above Granite's
current 1024-token manifest bound and this first service's 256-token admission
bound. A provider-limit-aware request is needed before connecting it. No live
normal-session summary was run; M23 remains `PARTIAL`.

[API and integration proposal](../../.dev/workstreams/model-runtime/handoff.md).

## 2026-11-04 isolated session-summary bridge

`SessionModelService::summarize_page` provides a distinct OS-owned operation:
the service first validates its bound authenticated desktop Session, and the
bridge compares the request's full logical identity with the foreground identity
supplied by the trusted desktop. The existing Context Resolver and public
Browser Context API perform object/workspace visibility and sharing checks
before any inference. Only this private bridge translates the resolved
page-summary provider invocation into the existing OS-owned model request.
The ordinary `generate` entry and `ModelSessionAccess` continue rejecting
`Some(AppId)`; this does not implement or authorize external app IPC.

The bridge preserves separate system instructions and serialized untrusted
context and caps output at the ordinary service admission limit. Cancellation
is checked before context access, before inference and after the reply;
mismatched response IDs are still rejected by the existing summary adapter.
Terms acknowledgement, model selection, artifact verification and resource
admission remain in the existing service. No new context-sharing policy is
introduced. Private product decisions were unverified because the configured
GitHub credential could not authenticate; this slice follows existing 0.1
specification only.

Verification in the independent `tests/model-session-service` workspace, using
the repository-pinned nightly-2025-08-01 Linux host toolchain:

```sh
cargo test --locked --offline --manifest-path tests/model-session-service/Cargo.toml
cargo clippy --locked --offline --manifest-path tests/model-session-service/Cargo.toml --all-targets -- -D warnings
cargo -Z build-std=core,alloc check --locked --offline --manifest-path tests/model-session-service/Cargo.toml --target targets/x86_64-unknown-nagi-user.json
```

All nine host tests pass (six new summary orchestration tests and three existing
session access tests); Clippy passes with warnings denied. The target leaf
compile passes with eight pre-existing `nagi-posix` warnings. Tests exercise
instruction/context preservation, output admission, full logical-session
mismatch, wrong authenticated Session, lock denial, sharing denial, hidden
selection, cancellation and stale reply IDs. They use explicit orchestration
fixtures and do not run model inference.

**Integration remains blocked by source availability in this checkout:** base
`0362427` has the model service leaf but no ordinary-session model worker or
Nagi Bar model UI. The current mainline owner must call this operation from
the trusted explicit summary action, transport owned system/input fields
without flattening their trust boundary, cancel the active worker on lock or
sign-out, and gate display on both the active Session and request. A copied
`Session` cannot observe a later lock. A request-ID check alone does not prove
that a reply belongs to the currently signed-in session; this bridge does not
claim that missing UI gate. The init feature graph and full guest ELF link
also remain owned by mainline. No model, VM, push or merge was performed.

### Isolated review follow-up: combined byte admission and typed errors

The follow-up to candidate `7dd84a2` corrects two bridge-local issues. The
bridge now receives the complete production `ModelServiceLimits`, checks the
serialized input plus separate system instructions against `max_input_bytes`,
and returns `Summary(InputTooLarge)` before invoking the service if their
combined size exceeds the bound. It does not truncate instructions, relax the
service limit, or silently turn size rejection into provider unavailability.

`SessionPageSummaryError::Service(ModelServiceError)` preserves exact Store,
resource, clock, model selection and runtime failures; `Denied` is preserved
separately. The generative adapter's narrower error interface is bridged by a
per-invocation error cell scoped to this synchronous operation. The original
error is returned after summary orchestration; it never escapes as the
adapter's temporary provider-unavailable error. A cancelled token still
discards any reply. License acknowledgement remains explicit and enforced by
the ordinary service; nothing acknowledges terms on the user's behalf.

The host harness now passes 11 tests, including real `LazyModelService`
admission at combined sizes 24,575 and 24,576 bytes, bridge rejection at 24,577
bytes, and the original regression where serialized input alone is 24,576
bytes. Page control characters exercise JSON escape expansion within public
API field bounds. Synthetic artifact integrity is checked by the actual
service; the explicitly opted-in fake backend returns fixture output only.
Separate regressions verify actual unacknowledged-license, low-resource and
unavailable-clock failures remain distinct and invoke no model backend.

Validation used the same standalone-workspace test, Clippy and no-std check
commands above, plus changed-leaf `rustfmt --check` and `git diff --check`.
Clippy passes with warnings denied; the no-std leaf check passes with the same
eight existing POSIX warnings. The `test-support` feature and synthetic hash
dependency are confined to the host harness's dev-dependencies.

Worker/UI message transport, lock/sign-out cancellation, session-plus-request
display gating and alignment of `NAGI_BAR_APP_ID` with the trusted foreground
logical identity are still unverified. Those source paths are absent from
this checkout. This follow-up makes no claim to implement their lifecycle,
full guest linking, live page extraction or real inference. M23 stays
`PARTIAL`; the previous candidate and shared branches remain unchanged.


## Local M30 integration checkpoint

The preceding isolated bridge notes describe the producer's base0362427
checkout. The e3500f4-based integration has an ordinary plaintext Bar worker;
that does not wire this synchronous summary entry into it. A/B transfer sizes,
hashes and both reconstructed trees were verified independently. No conflict
occurred on application. The production feature now includes nagi-ai, and Albert
and the bridge share the exact NAGI_BAR_APP_ID through nagi-ai. The identity is
the requesting assistant, not the foreground page's application identity.
Full identity equality and a live bound Session are necessary checks, but a
freely constructed CallerIdentity is not proof of OS provenance. The future
transport must supply a verified OS invocation rather than copying page fields.

The exact wrapper is exercised on the host with fail-fast syscall/artifact/FFI
hooks, including locked/replaced Sessions, non-assistant/mismatched identities,
actual Albert policy denial, explicit model-license refusal, external AppId
refusal and pre-context cancellation. The bridge fixtures also verify typed
errors, combined bytes and cancellation overriding a simultaneous service error.
These are orchestration/admission fixtures, not actual model execution. No
production terms acknowledgement occurs. See tests/model-session-service/README.md.

Summary worker/UI hookup, lifecycle cancellation, Session+request late-display
filtering and context/navigation/sharing freshness remain NOT_EVALUATED. Existing
plaintext worker lifecycle tests do not prove summary lifecycle. Guest extraction,
real inference and complete M23/M30 acceptance are not claimed; M23 PARTIAL and
M30 BLOCKED. No public operation, denied Browser-export retry or private-source
access is part of this local integration.
