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
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m23-final2-host-arm64 \
/Users/tozawa/.cargo/bin/cargo test --locked --offline -p nagi-ai
```

Result: 23 unit tests passed; doc tests passed (none defined). Added tests cover
API routing, selected-object denial, untrusted provider input, bounded UTF-8
context, workspace and object authorization, API/provider unavailability,
empty page content, and invalid summary results. Test providers are
orchestration fixtures and do not claim real model inference.

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m23-final2-target-arm64 \
/Users/tozawa/.cargo/bin/cargo -Z build-std=core,alloc check \
  --manifest-path Cargo.toml -p nagi-ai \
  --target targets/x86_64-unknown-nagi-user.json --locked --offline
```

Result: the updated `nagi-ai` service compiled for the Nagi `no_std` target.

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m23-final2-clippy-arm64 \
/Users/tozawa/.cargo/bin/cargo clippy --locked --offline \
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
