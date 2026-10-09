# Ordinary-session Granite service handoff

Status: PARTIAL. Branch: `codex/0.1-session-model-service`.
Base: `f74f128abe40e9e99d7cb05f3f8dc7b90b1448c9`.

## Implemented API

- `ResidentModelRuntime<B>` retains one checked `LoadedSession` between
  requests, reusing the existing manifest/hash/request/response validators.
- `LazyModelService<B, A, C>` constructs without artifact IO or backend load;
  selects the configured model by capability/role; rejects a preference for
  another model; validates current resource budget, exact terms acknowledgement,
  bounded UTF-8 input/output, output-token bound and deadline. Hashing and target
  callback reads poll cancellation/deadline. A failed/cancelled request discards
  output and unloads the context. Unload failure disables further loads.
- `user/nagi-init/src/model_service.rs::SessionModelService` binds the service
  to the OS desktop's authenticated `Session`, rejects locked/other sessions and
  external caller identity, and opens only the kernel-provided read-only Model
  Store. `new(&Session, model_store_capability, ResourceBudget)` is lazy.
- `generate(&Session, Option<&RoleId>, Option<&ModelId>, &ModelRequest,
  &dyn CancellationToken)` returns complete `ModelResponse` or typed failure.
  The Nagi Bar requests `text.generate` and role `standard`, with `caller: None`
  for this OS-owned route. The configured manifest remains Granite 4.2 3B.
- `terms_reference()` / `acknowledge_terms(&Session, exact_reference)` preserve
  the existing manifest's acknowledgement requirement. Do not call this from
  model text or pre-acknowledge on the user's behalf.
- `set_resource_budget`, `resource_report`, and `unload` support pressure and
  session lifecycle. One service has one model session and serialized requests.

`llama_backend.rs` extracts the existing, proven Rust/FFI adapter rather than
reimplementing inference. Backend library initialization now occurs on first
load; the existing acceptance explicitly initializes it before its trace.
The C++ inference implementation and source/model pins are unchanged. The
existing adapter clears llama context memory for each request. Compute threads
remain one under the cooperative guest scheduler. Structured grammar support
remains the existing bounded answer schema; arbitrary planner schemas and
streaming are not advertised as working.

## Shared-owner integration proposal

1. Add `m20-model-service` to init's features, with Model Manager, Nagi POSIX and
   `nagi-posix/m20-llama-memory`. Add the ordinary service module under this
   feature. Do **not** depend on `m20-model-store-acceptance` or
   `m20-llama-inference-acceptance`: those paths verify and exit. The current
   desktop boot branch excludes `m13-posix`/`m12-network`; adding these dependencies
   also needs the shared owner's normal-desktop cfg review so it still reaches
   the signed-in desktop. Preserve kernel/init memory-feature consistency.
2. Extend init `build.rs`'s libc++, llama archives, provider C++ adapter and
   constructor cfgs to the production feature using the existing target-built
   archive paths. Current linking is guarded by the inference acceptance
   feature; the leaf compile harness does not prove final linking.
3. Pass the existing read-only Model Store capability into the normal desktop's
   model worker. Construct/own the service **inside** that guest worker after
   desktop readiness and login. Supply a reserved guest memory budget, not an
   assumed report that the whole 8 GB VM is free. Do not send arbitrary Files,
   Network, microphone or window handles to the backend.
4. Nagi Bar owner sends actual bounded text, a nonzero increasing request ID,
   `text.generate`, role `standard`, and at most 256 output tokens. Worker owns
   the service across requests; only one request may execute at once. UI keeps
   a per-request cancellation token and polls while the worker runs. The service
   yields on artifact reads and llama cancellation/abort checks. Allocate a
   worker stack appropriate for the existing C++ adapter's bounded prompt/token
   buffers. No `Send` promise for raw llama pointers is introduced.
5. Display only a response matching the active request/session. On lock/sign-out
   set active cancellation, discard late responses, then unload on the worker.
   Supply the live OS-owned Session on each operation; an old copied Session
   cannot observe a later lock. Never treat request-supplied AppId or a session
   token from IPC as authorization. External app IPC still needs an existing
   supervisor/grant design; no AI grant definition was found or added here.
6. Provide OS-owned terms acknowledgement for Granite's unchanged manifest and
   persist it via the existing Store owner. This slice does not invent an
   installed-record receipt or silently change `acknowledgement_required`.
7. Build a disposable normal-session image using an already verified Granite
   artifact, sign in, show desktop readiness before first model load, enter two
   different Nagi Bar requests and verify real responses from one resident
   session. Exercise cancellation, lock/unload, missing artifact, and restart.
   Add this as a distinct acceptance, with normal desktop remaining alive.

The existing M23 `ModelPageSummaryProvider` requests 2048 output tokens, above
both this first service's 256-token admission bound and Granite's current
1024-token manifest bound. Its owner needs a provider-limit-aware request before
connecting page summaries. This slice prioritizes ordinary text requests and
does not claim M23 summary acceptance.

## Verification and limits

Evidence logs normalize machine-specific path prefixes to repository-relative
paths or generic cache/build locations. Test outcomes, diagnostics and counts
are retained.

Host orchestration and session-authentication tests and the Nagi-target leaf
compile are recorded under `evidence/session-service-*`. The independent
harness checks the production and acceptance modules together. It is not a
normal init ELF link, QEMU desktop test, or real inference acceptance.

The saved cloud environment contained pinned third-party source caches but no
Granite GGUF/model-cache directory or guest Model Store image. No large model
download, paid API, host inference, or Mac work was started. Use a verified artifact with the unchanged pinned hash. The saved environment
has no model weights. Existing dedicated acceptance evidence stays separate.

Resource admission uses the existing manifest requirements and a supplied
budget; backend resident memory remains unreported (`None`). Kernel-enforced
per-service RSS limits and AI process isolation are not implemented by this
in-process bootstrap adapter. Deadlines/cancellation are cooperative; long
uninterruptible target load/compute work can return late, after which the
service discards output/unloads. Desktop responsiveness and guest timing must
be measured after shared integration.

Qwen/Gemma inference, switching, speech, custom-model UI, runtime isolation,
installation/discovery persistence, and full M20/M23/M26 acceptance remain open.
`docs/implementation_status.md`, shared Registry/CI, root Cargo/lock, main.rs,
commands.rs, desktop.rs, Files/Search leaves, and image generation are unchanged.

## Delivery and latest target compilation

Draft PR: https://github.com/RT-NISH/NagiOS/pull/37.
Current code commit: `2258b08c5b8b0e41718156ceeab331f866a0e496`.
Latest CI: https://github.com/RT-NISH/NagiOS/actions/runs/37859329000.

The production sector reader yields before its first read and after at most
128 sectors. This also covers FAT32 directory/cluster-chain walks inside a
single artifact operation, where the outer request checkpoint cannot yield
yet. Exact Nagi-target leaf Clippy and init format passed again; guest latency
remains unmeasured. The dedicated acceptance reader/backend is unchanged by
this follow-up.

## Current-main compatibility

Main `f74f128` imports cleanly without source conflicts. After import, the
102 Model Manager/AI host tests, 3 Session-binding tests, manager Clippy, exact
Nagi-target leaf Clippy, formatting and development-State checks passed again.
State validation reports 30 registered workstreams and 23 State files. Logs
are in `evidence/merge-main-f74-*`. Model-service source is unchanged.
The existing Granite backend regression passed in Nagi/QEMU; ordinary-session
service acceptance still requires shared integration.
