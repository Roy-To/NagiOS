# ADR 0070: ordinary session image and explicit release gates

Status: implemented in part; formal 0.1 acceptance remains blocked.
Date: 2026-10-08.

## Evidence and ownership

The shared integration workstream owns init/Cargo/CLI/release wiring; ordinary
Files/Desktop and model-service components retain their separate ownership.
This publication candidate is based on reviewed main
`ef217b30c6074833ed81ff7a9a20e1a8fb0ed8b8`. The original wiring review used
`edad2e7` (PR31, PR34, PR35). Existing component registrations retain their
established scope and activation gates.

`execute_m30` built m10-desktop,m19-search,m20-model-store-acceptance,
m22-history,m21-action-ipc with no external model file, then printed PASS M30.
That proves a GPT/Recovery fixture; it does not prove an ordinary 0.1 session.
`m17-servo` returns into the terminating first-pixel or M18 test embedder before
Desktop. `m20-llama-inference-acceptance` exits after inference. `m19-search`
inherits m13-posix, which takes a separate boot path before Desktop. Combining
these milestone features therefore cannot deliver the requested integration.

## Decision

Preserve the existing fixture checks as `./nagi m30-layout`, with a distinct
`Nagi-OS-0.1-layout-fixture.qcow2` filename and scope in its success message.
`./nagi m30` fails closed while the formal gate is unavailable. No tests or
fixture assertions are removed; no milestone state is promoted.

`./nagi session-image [Granite.gguf]` builds a GPT image with separate Recovery
and ordinary init, using `production-session` (login and signed product Files
Search). The feature rejects incompatible milestone/acceptance boot paths at
compile time. The product package is passed through NAGI_FILES_SEARCH_PACKAGE;
no NAGI_ACCEPTANCE_PACKAGES are built or embedded. Owner setup occurs on first
use; no owner password or pre-created private User Data is baked into an image.
The existing developer package/slot signing conventions remain developer-only;
this work does not generate keys, change authentication, or authorize release.

An optional Granite file must match the existing pinned manifest and models.lock
size/digest/license metadata before streaming into the read-only guest Model
Store. Its source file is rechecked after image creation before provenance is
written. It is model presence, not a guest inference or redistribution claim.
No arbitrary directory or unpinned model is packaged. The source tree must be
committed and clean before and after image generation.

Version 2 image provenance binds image hash and source revision to profile,
init features, component inventory, and actual Model Store model digest. The
current producer emits `signed-in-files-v1` with login/files/search only. The
release tool can inspect old provenance but preflight/assembly/verify reject
legacy or partial profiles, fixture features, missing components, and absent or
wrong Granite bytes. Only a future implemented `production-session-v1` config
can satisfy that configuration gate. Configuration verification never proves
runtime acceptance, legal review, or release readiness.

## Open integration and distribution work

- Accept normal Files CRUD/Trash/Search proposal, plus real operation/restart
  evidence, without using login-only fixtures.
- Accept resident model-service proposal with capability/provider-neutral
  requests, bounded processing, cancellation, and clean teardown. Split its
  native link feature from acceptance; do not reuse an inference-and-exit boot.
- Implement a normal Servo lifecycle/surface/input API. Current public guest
  embedder entry points are acceptance-only and cannot share Desktop's loop.
- Integrate History/Undo, voice, and all M28 simultaneous-load measurements.
- The image writer currently supports one external model. Qwen/Gemma, embedding,
  STT/TTS packages and their notices/distribution decisions remain open; Gemma
  terms are never accepted implicitly.
- Review complete linked Rust/native/model/font notices and source availability,
  developer signing metadata, project license, build manifest and pristine
  artifacts. Tracked source licenses alone do not close distribution review.
- Run the same production image on the official reference VM, preserving logs,
  screenshots, input transcripts, reboot/offline state, and all earlier gates.

The proposed session event contract is implemented by
`tools/nagi-release/session_acceptance.py`; it requires one bound image and
writable-disk hash chain across initial/restart/offline phases, ordered sign-in,
real operation identity and state hashes, guest generation measurements and
confirmed policy/transaction/undo outcomes. Existing M1–M7 bootstrap diagnostics
are allowed only before ordinary session events. Offline browser use requires
local history restoration, not a fresh external HTTPS request. Submitted metrics
cannot select looser stress limits: the provisional gate uses 60 seconds,
250 ms desktop latency, no audio underruns/OOM, at most 64 MiB memory/32 handle
growth and 300% Granite CPU on four vCPUs. These are a reviewable bounded stress
probe and do not replace the full normative 0.1 demo or long-run validation.
The validator's positive tests are expressly synthetic orchestration fixtures;
no real production event evidence currently exists.

CI retains the formal `./nagi m30` gate and adds the separately labeled layout
fixture and partial session build before it. A production-incomplete target job
therefore fails honestly; neither expected failure nor a host regression suite
is converted into M30 PASS. Release/profile/session Python regressions run in
the Ubuntu host job.
