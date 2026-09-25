# ADR 0025: Select Nagi Softpipe During EGL Initialization

- Status: Accepted for M17 implementation
- Date: 2026-09-25
- Milestone: M17 — Servo Bootstrap

## Context

CI run #183 (`36099071216`) passed the M17 two-boot persistence gate and reached
Servo software GL context construction, then timed out after 120 seconds. The
last existing serial marker was `Nagi M17 trace: GL context creation started`.
This evidence locates the stall inside the Servo/Surfman/Mesa context path but
does not identify the specific call.

Follow-up CI run #184 (`36106455335`) passed the same persistence gate and
repeated the timeout at the same application-level marker. None of the new
EGL or Servo/Surfman stage messages appeared in the guest serial log. This
does not prove whether the call path was missed or the stderr logging route
failed to expose the messages.

CI run #185 (`36113604201`) did not reach M17 acceptance. The Ubuntu host,
Windows, and target jobs all stopped during `nagi-bootstrap fetch` because the
direct `libc` dependency added to `servo-paint-api` was not yet present in
Servo's lockfile. The `--locked` check correctly rejected that incomplete
source patch set. Patch `0009` now records the corresponding lockfile entry.

CI run #186 (`36114799741`) passed pinned Servo bootstrap after patch `0009`,
then failed at the root workspace lock boundary: Ubuntu Clippy and the target
dependency feature check both found that the root `Cargo.lock` also lacked the
`servo-paint-api -> libc` edge. At that point both lockfiles recorded the
direct dependency; the callback repair below removes it because the trace no
longer needs a direct `libc` dependency.

CI run #187 (`36115897284`) passed the locked host checks, target dependency
validation, Mesa Softpipe build, Nagi user-init link, and UEFI loader, then
timed out during the real two-boot M17 QEMU acceptance. The serial log again
ended at `Nagi M17 trace: GL context creation started`. None of the Servo
checkpoints written through `libc::write` appeared, so their absence cannot
show whether the patched Servo constructor was entered or whether that output
route failed.

CI run #188 (`36120900543`) passed the same build stages after switching Servo
checkpoints to an Albert callback backed by `libnagi::console_write`. The
acceptance still timed out at the same application-level marker, and no
callback or Servo-stage trace appeared. The patch applies to
`SoftwareRenderingContext::new` before `Connection::new`, but the serial result
does not establish that the method body executed. The next run adds an Albert
callback self-test before the method call and a constructor-entry checkpoint
before its size guard.

CI run #189 (`36126812876`) printed the Albert callback self-test successfully,
then timed out before the patched constructor-entry checkpoint. This proves
that the callback route works and narrows the stop to the constructor call or
its entry sequence. The guest bootstrap process has only 8 stack pages (32
KiB), so a bounded stack increase is the next hypothesis; decision 0026 records
the 2 MiB experiment. The serial evidence does not yet prove stack exhaustion.

CI run #190 (`36134006498`) reached the patched constructor, completed Mesa
EGL's Softpipe initialization, and created the Surfman device and context
descriptor. `device.create_context` returned an error. Servo's existing error
diagnostic then panicked on `println!` because Nagi stdout returned `EIO`, so
the actual Surfman error remained hidden. Patch `0010` reports that error
through the existing Nagi console callback and keeps Servo's normal diagnostic
on other targets.

CI run #191 (`36139834714`) passed the same target build stages and reached the
same Surfman failure without the stdout panic, but the diagnostic line was
empty. The callback printed its static prefix and newline, while the formatted
error payload was missing. Servo's `format!` creates an allocated string; the
Nagi POSIX heap is backed by `mmap`, but the console syscall's preliminary
address policy allowed only image, stack, and TLS ranges. Its subsequent
readable-page check already understands mmap-backed user pages. The repair
allows bounded mmap ranges through the preliminary check and keeps the mapped
page validation and kernel-side copy in place. The callback also checks and
reports failed writes.

CI run #192 (`36145246098`) displayed the error and its enum value:
`ContextCreationFailed(BadAlloc)`. Surfman's source shows this variant is
returned only when `eglCreateContext` returns `EGL_NO_CONTEXT` and
`eglGetError()` returns `EGL_BAD_ALLOC`; this is before dummy-pbuffer creation
and `eglMakeCurrent`. Mesa can map either its EGL context-wrapper allocation
failure or a DRI/Softpipe context-setup failure to that code.

CI run #193 (`36157913731`) passed the two-boot persistence gate and completed
EGL driver initialization and DRI screen creation, then timed out after 120
seconds at `eglCreateContext`. The serial log ended immediately after Servo's
`GL context creation started` checkpoint, so it does not identify the
Softpipe initialization call that failed to return. Target compilation
reported `sp_context.c:190: unused variable 'sh'`, consistent with patch
`0022` excluding Softpipe's eager texture-cache loop under `__NAGI__`. The
run therefore shows forward progress beyond DRI screen creation but does not
prove which call inside `softpipe_create_context` is stalled.

The pinned Softpipe source eagerly allocates one texture tile cache for each
of its 6 shader stages and 128 sampler-view slots during context creation. A
cache embeds sixteen 32×32 RGBA-float tiles, so its tile storage alone is 256
KiB; the full 768-cache matrix exceeds 192 MiB. Nagi's current POSIX heap is
fixed at 8 MiB. The context creation loop returns failure when any cache
allocation fails, which can surface as `EGL_BAD_ALLOC`. This is a
source-confirmed memory-budget mismatch and a plausible cause of CI #192, but
the CI result alone does not prove this was the allocation that failed.

Patch `0022` avoids eager cache allocation on Nagi. It allocates a cache when
a non-null sampler view is bound, releases it on unbind, and fails explicitly
if a live binding cannot obtain its cache. Other platforms keep the upstream
eager path. This retains the existing sampler-view limit and sampling logic;
the target QEMU run must verify whether context creation now advances.

The target Mesa build compiles Gallium Softpipe as its only renderer and links
EGL and Softpipe statically into the guest. Surfman already requests its
software adapter. Mesa EGL, however, normally derives software selection from
`LIBGL_ALWAYS_SOFTWARE` and can try a Zink override or hardware-related paths
before falling back to swrast. Nagi should enter the renderer path its build
actually provides.

## Decision

Under `__NAGI__`, EGL initialization sets `ForceSoftware` and clears `Zink` so
Mesa follows its surfaceless software-rendering path backed by the statically
linked Softpipe driver. Other targets retain Mesa's existing
environment-controlled policy.

Nagi-only warning-level diagnostics bracket EGL device discovery, driver
initialization, surfaceless software and no-DRM probes, and DRI screen creation.
Servo's Nagi patch adds checkpoints through Surfman device/context creation,
GL function loading, surface binding, make-current, and swap-chain creation.
Mesa patch `0023` adds Nagi-only `_debug_printf` checkpoints around Softpipe
context initialization, including draw-context creation and blitter shader
caching. These logs are diagnostic and do not change rendering, surface
ownership, or M17 acceptance criteria.

CI #195 showed that all instrumented operations inside
`softpipe_create_context` completed, including the final context-completion
marker, while Surfman's `device.create_context` still did not return. Patch
`0024` therefore adds Nagi-only checkpoints after the Softpipe callback and
through Mesa state-tracker GL initialization, DRI context construction, and
EGL context linking. The additional markers do not alter context behavior or
relax the first-pixel acceptance criteria.

CI #196 showed that Mesa state-tracker, DRI context creation, EGL's driver
`CreateContext`, and `_eglLinkContext` all returned; its final marker was
`EGL context linking completed`. Surfman's `device.create_context` still did
not return. Surfman patch `0002` adds Nagi-only checkpoints around the EGL
context wrapper, dummy pbuffer setup, make-current, and GL function loading.
These checkpoints do not change the rendering path.

CI #197 completed the dummy-pbuffer setup and stopped immediately after
`Surfman make-current started`; no `eglMakeCurrent` return marker appeared.
That marker is in Surfman's initial context-creation path, where `Framebuffer::None`
uses the newly created dummy pbuffer for both EGL draw and read surfaces. It is
not the later surfaceless rebind used after a generic texture surface is
bound. The current log does not prove whether execution entered Mesa's public
EGL function. Mesa patch `0025` adds Nagi-only checkpoints before and after
display locking, handle validation, DRI2 binding, state-tracker framebuffer
validation, and the surfaceless pbuffer's backing-resource callback. The
checkpoints are diagnostic only and keep the same pbuffer and GL behavior.

CI #198 confirmed that the call entered Mesa's public `eglMakeCurrent`,
acquired the display lock, resolved handles, completed API validation, and
entered DRI2 `dri2_make_current`. Its last marker was
`DRI2 EGL binding started`, immediately before `_eglBindContext`; the helper
did not return before the 120-second timeout. The trace therefore narrows the
stall to `_eglBindContext` or one of its helpers, without yet proving whether
thread-info lookup, ownership/config validation, reference updates, or binding
is responsible. Mesa patch `0026` adds Nagi-only checkpoints across those
internal stages, marks each validation rejection, and brackets the separate
EGL debug-report global mutex. These checkpoints distinguish TLS access,
validation, error-report locking, and binding work without changing EGL
binding behavior or acceptance requirements.

After CI #184, Servo's trace helper wrote directly to Nagi descriptor 2 through
`libc::write` rather than Rust stdio. CI #187 still produced no such trace, so
that route did not provide reliable evidence. CI #188 also produced no trace
through the new Nagi-only Albert callback. The callback uses the already
working `libnagi::console_write` syscall. The next diagnostic checks the
callback directly before `SoftwareRenderingContext::new` and marks the first
instruction in the constructor body; other targets retain a no-op helper.

## Consequences

- Nagi will not request a hardware or Zink renderer that its static Mesa build
  does not provide. Mesa can still probe software-compatible DRM devices before
  falling back to no-DRM swrast; the new trace points make that path visible.
- Other Mesa targets keep their current renderer-selection behavior.
- CI #191 established that the formatted error was lost at the console-write
  address policy. The next target run will report the actual Surfman error, or
  a console-write failure marker if the bounded mapped-range check still
  rejects the buffer.
- CI #192 established that context creation returns `EGL_BAD_ALLOC`. Source
  inspection found Softpipe's 192 MiB eager cache matrix exceeds Nagi's 8 MiB
  heap; patch `0022` allocates only caches for bound sampler views on Nagi. CI
  #196 traced through EGL context linking; #197 confirmed dummy-pbuffer
  creation; #198 entered EGL/DRI binding but timed out inside
  `_eglBindContext`. Mesa patches `0024`–`0026`, plus Surfman patch `0002`,
  add Nagi-only trace checkpoints through context linking and EGL thread,
  context, and surface binding. They do not relax the M17 acceptance criteria
  or change the rendering path.
- M17 remains `BLOCKED` until real Servo content is rendered, presented to the
  Nagi surface, and produces a nonzero pixel checksum. M18 remains `NOT STARTED`.
