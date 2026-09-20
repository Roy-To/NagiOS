# M17 Workstream B: Real Servo Guest Rendering Investigation

Date: 2026-09-19
Worktree: <m17-worktree>
Servo pin: b820a9679a784877f91b4acc90c2c6e849f18d3b

This is a read-only investigation. No build, test, fetch, reset, clean, QEMU
run, or source checkout mutation was performed. The only write is this report.
The pre-existing dirty files listed below were preserved.

## Verdict

The minimum real guest path is:

    Servo event loop
      -> WebView::paint
      -> WebRender using a Nagi-selected real GL/GLES context
      -> guest Mesa Softpipe/EGL surfaceless generic surface, 320x200
      -> RenderingContext::read_to_image (RGBA glReadPixels)
      -> NagiSurface::copy_rgba_frame
      -> capability-checked display_present
      -> existing Surface VMO / compositor / QEMU scanout

The existing Servo SoftwareRenderingContext is the closest reference and
contains most of the required GL/readback logic, but it is not a drop-in Nagi
solution. The pinned Servo paint manifest currently enables Surfman's sm-x11
feature. On a Nagi target, Surfman classifies the target as free Unix, and its
default aliases can select Wayland/X11 before the Mesa surfaceless backend. Its
EGL loader also expects a dynamically loaded libEGL.so.1 or libEGL.so. Nagi's
current relibc excludes its header and ld_so modules for target_os = "nagi",
so the dynamic loader path is not currently available.

The smallest credible implementation is therefore a Nagi-owned RenderingContext
adapter over the Surfman Mesa-surfaceless scaffolding, with a Nagi-specific EGL
symbol-loading path. Prefer statically linked guest Mesa/EGL archives and
direct/static symbol resolution for M17; completing a general Nagi
dlopen/shared-object runtime is a larger alternative. The adapter must read
back real pixels and must not use Windows OpenGL, host surfaces, synthetic
pixels, or CPU HTML substitution.

## Repository and revision state

The generated Servo checkout is clean and exactly pinned:

    third_party/servo/REVISION                  b820a9679a784877f91b4acc90c2c6e849f18d3b
    git -C third_party/servo rev-parse HEAD      b820a9679a784877f91b4acc90c2c6e849f18d3b
    git -C third_party/servo status --short ...  ## HEAD (no branch)

The parent worktree was already dirty before this report. The existing changes
are:

    M  .gitignore
    M  third_party/sources.lock
    M  tools/nagi-cli/src/commands.rs
    M  tools/nagi-cli/src/lib.rs
    ?? third_party/mesa-patches/
    ?? third_party/surfman-patches/
    ?? tools/mesa/
    ?? tools/nagi-cli/src/mesa.rs

These changes add a Mesa source-lock/bootstrap candidate, not a rendering
adapter or a verified guest build. third_party/mesa itself is absent, and
third_party/mesa-patches currently contains only its README, no numbered patch.
A later read-only status snapshot also showed one untracked
third_party/surfman-patches/0001-nagi-static-mesa-backend.patch. It was not
created or modified by this investigation; it is described below as a
candidate only and was not applied to any Cargo source.
The baseline docs/implementation_status.md and ADR 0019 still record M17 as
blocked for lack of a Mesa path; that remains accurate for acceptance purposes
because the dirty candidate has not been fetched into its authoritative
generated path or built.

## Pinned Servo APIs and exact render sequence

### RenderingContext contract

In third_party/servo/components/shared/paint/rendering_context.rs:35-88, the
mandatory adapter contract is:

    prepare_for_rendering()
    read_to_image(DeviceIntRect) -> Option<RgbaImage>
    size() -> PhysicalSize<u32>
    resize(PhysicalSize<u32>)
    present()
    make_current() -> Result<(), surfman::Error>
    gleam_gl_api() -> Rc<dyn gleam::gl::Gl>
    glow_gl_api() -> Arc<glow::Context>

create_texture, destroy_texture, connection, and refresh_driver have defaults.
For the first M17 page, WebGL/WebGPU/WebXR can remain disabled so the optional
surface-sharing methods do not enlarge the adapter.

### Existing GL context construction

SurfmanRenderingContext in
third_party/servo/components/shared/paint/rendering_context.rs:114-165
shows the real initialization requirements:

1. Connection creates a Device from an Adapter.
2. Context attributes request alpha, depth, and stencil.
3. GL uses version 3.2; GLES uses version 3.0.
4. gleam::GlFns::load_with or GlesFns::load_with obtains function pointers
   from device.get_proc_address.
5. glow::Context::from_loader_function is built from the same real loader.

The software implementation at
third_party/servo/components/shared/paint/rendering_context.rs:284-390
creates a connection, calls create_software_adapter, creates a generic surface,
binds it, makes the context current, and creates an attached swap chain. It is
explicitly an OpenGL software implementation; it is not a CPU-only renderer.

The generic framebuffer readback implementation at
third_party/servo/components/shared/paint/rendering_context.rs:614-729
creates RGBA/UNSIGNED_BYTE storage, binds the framebuffer, calls gl.read_pixels,
checks the GL error, vertically flips the rows, and returns RgbaImage. This is
the smallest proven readback behavior to reproduce or reuse in the Nagi adapter.

### WebRender and WebView sequence

third_party/servo/components/paint/painter.rs:153-160 makes the context current
and obtains the gleam API. The actual frame path at painter.rs:408-444 makes
the context current, calls prepare_for_rendering, updates WebRender, clears the
target, and calls renderer.render(size, 0).

The pinned Servo embedder documentation at
third_party/servo/components/servo/lib.rs:16-34 and WebView model at
components/servo/webview.rs:61-82 require the embedder to:

1. Create Servo with a custom EventLoopWaker.
2. Create a custom RenderingContext.
3. Create WebViewBuilder::new(&servo, Rc<dyn RenderingContext>), set a
   WebViewDelegate, set the guest URL, and build the WebView.
4. On WebViewDelegate::notify_new_frame_ready, call WebView::paint.
5. Present/read the resulting context according to the context's buffering
   semantics.

The relevant APIs are WebView::paint at
third_party/servo/components/servo/webview.rs:747-750, builder methods at
webview.rs:1092-1184, Servo::spin_event_loop at
third_party/servo/components/servo/servo.rs:1088-1095, and
ServoBuilder::event_loop_waker at servo.rs:1450-1487. The waker contract is
third_party/servo/components/shared/embedder/lib.rs:236-248; it may be called
from another thread and must cause the owning thread to call
spin_event_loop. notify_new_frame_ready is the delegate hook at
third_party/servo/components/servo/webview_delegate.rs:918-952.

For a single-buffered/generic Nagi target, RenderingContext::present should not
be a host/window swap. It can perform the required guest flush/synchronizing
operation, followed by read_to_image; the distinct Nagi display presentation is
NagiSurface::present after the VMO copy. This ordering must be documented and
tested rather than hidden behind a fake present.

## Existing Nagi framebuffer boundary

The existing user/nagi-servo crate is deliberately no_std and already provides
the final handoff:

- NagiSurface::acquire and surface validation:
  user/nagi-servo/src/lib.rs:38-80.
- Exact 320x200 RGBA validation and bounded row copy:
  user/nagi-servo/src/lib.rs:82-118.
- Capability-checked presentation:
  user/nagi-servo/src/lib.rs:121-126.
- Event-loop atomic wake bridge:
  user/nagi-servo/src/lib.rs:202-224.

The kernel owns the Surface VMO and display authority at
kernel/src/display.rs:20-35,69-117. User space obtains the description and uses
the existing syscalls at user/libnagi/src/lib.rs:203-233. No new high-level
browser/display syscall is required for this rendering path.

The smallest frame operation is therefore:

    webview.paint();
    rendering_context.present(); // guest flush/no window swap, if required
    let image = rendering_context.read_to_image(DeviceIntRect::new(0, 0, 320, 200));
    surface.copy_rgba_frame(image.as_raw(), 320, 200, 320 * 4)?;
    surface.present();

The adapter must use the actual RgbaImage produced by glReadPixels; the
existing NagiSurface tests only validate the bounded copy and do not prove
Servo/WebRender rendering.

## Surfman/WebRender reuse assessment

The pinned Servo workspace already locks the useful Rust crates:

    third_party/servo/Cargo.toml:247       surfman 0.13.0, feature chains
    third_party/servo/Cargo.toml:282-283   webrender/webrender_api 0.70
    third_party/servo/Cargo.toml:103,106   gleam 0.15, glow 0.17
    third_party/servo/Cargo.lock:3185      gleam 0.15.1
    third_party/servo/Cargo.lock:3260      glow 0.17.0
    third_party/servo/Cargo.lock:4879      khronos-egl 6.0.0
    third_party/servo/Cargo.lock:10425     surfman 0.13.0
    third_party/servo/Cargo.lock:11904     webrender 0.70.0

The Cargo registry has source trees at:

    <cargo-registry>/surfman-0.13.0
    <cargo-registry>/gleam-0.15.1
    <cargo-registry>/glow-0.17.0
    <cargo-registry>/khronos-egl-6.0.0

Surfman 0.13.0 has a reusable mesa_surfaceless backend:

- src/build.rs:20-33 defines free_unix for all Unix targets except
  Apple/Android/Emscripten/OHOS, and defines Wayland as the default Unix
  platform.
- src/lib.rs:33-67 exposes mesa_surfaceless but aliases the public Connection,
  Device, and Surface types to the default platform.
- src/unix.rs:1-19,22-58 defines the default as a Wayland/X11 plus surfaceless
  multi-backend.
- src/multi/connection.rs:64-70,131-141 tries the default display first and
  uses the alternate only after that connection fails.
- src/mesa_surfaceless/connection.rs:35-63 directly requests
  EGL_PLATFORM_SURFACELESS_MESA and initializes EGL without a native display.
- src/mesa_surfaceless/device.rs:139-152,364-371 creates a pbuffer context and
  only accepts SurfaceType::Generic.

This is useful Rust scaffolding for Nagi, but the default aliases must not be
used unchanged. A Nagi target-specific patch must select the surfaceless types
directly or add an explicit Nagi backend. Removing only sm-x11 is not enough,
because wayland_platform is still true for free Unix.

Surfman's EGL loader at
src/base/egl/device.rs:5-18,29-43,62-68 loads libEGL.so.1/libEGL.so with
dlopen and resolves symbols with dlsym. The repository's relibc implementation
exists at third_party/relibc/src/header/dlfcn/mod.rs:89-169, but
third_party/relibc/src/lib.rs:42-79 excludes the entire header and ld_so
modules for target_os = "nagi". Consequently, this existing loader cannot be
assumed to work in the guest.

The untracked candidate
third_party/surfman-patches/0001-nagi-static-mesa-backend.patch shows the
smallest direct Surfman change: it excludes Nagi from Wayland/X11 native
dependencies, defines a Nagi platform cfg, makes mesa_surfaceless the Nagi
default, and replaces the non-Windows dlopen/dlsym path with a direct
extern "C" eglGetProcAddress lookup for a statically linked guest EGL. This
is the right adapter direction, but it is not yet a reproducible applied patch.
It also does not remove the pinned Servo shared-paint dependency's explicit
sm-x11 request at
third_party/servo/components/shared/paint/Cargo.toml:46. Cargo feature
resolution for target_os = "nagi" must therefore be checked; a small Servo
manifest patch may still be required.

## Mesa/Softpipe source and build reuse

The pre-existing dirty worktree now contains a source-lock candidate:

    third_party/sources.lock:54-61
      repository  https://gitlab.freedesktop.org/mesa/mesa.git
      revision    f1f246cfda65eff82fba3be1caf2d23bdeda60cc
      source_hash git:f1f246cfda65eff82fba3be1caf2d23bdeda60cc
      vendored    third_party/mesa
      patches     third_party/mesa-patches

tools/nagi-cli/src/mesa.rs:83-135,248-371 is an untracked bootstrap helper that
would clone the source, detach at the revision, apply numbered patches, write
.nagi-mesa-checkout, and validate the generated state. It was not run.
tools/mesa/README.md:1-25 and
tools/mesa/nagi-x86_64-user.meson.cross:1-25 describe an intended static Nagi
cross build using:

    -Dgallium-drivers=softpipe
    -Degl-native-platform=surfaceless
    -Dglx=disabled
    -Dllvm=disabled
    -Dshared-glapi=enabled
    -Dosmesa=true

An ignored inspection checkout is available at:

    out/cache/mesa-inspect-24.3.0
    HEAD:       f1f246cfda65eff82fba3be1caf2d23bdeda60cc
    tag:        mesa-24.3.0
    repository: https://gitlab.freedesktop.org/mesa/mesa.git
    state:      clean, detached, shallow, ignored by .gitignore:5 (/out/)
    size:       approximately 317 MB, 10,660 files

It contains real source for src/gallium/drivers/softpipe, src/egl, and
src/egl/drivers/dri2/platform_surfaceless.c, and its options include softpipe
and osmesa. It has no generated build.ninja, static archive, shared library, or
guest artifact. It is not the locked generated checkout at third_party/mesa,
has no .nagi-mesa-checkout marker, and is not acceptance evidence by itself.

The current host has cmake, meson, and ninja, but the commands required by the
checked-in Mesa cross file are absent:

    clang clang++ llvm-ar llvm-ranlib llvm-strip ld.lld lld-link link pkg-config

Rust is the Windows MSVC host toolchain (rustc 1.90.0-nightly, active
nightly-2025-08-01-x86_64-pc-windows-msvc); installed targets are only
x86_64-pc-windows-msvc and x86_64-unknown-uefi. The Nagi JSON target is custom
(targets/x86_64-unknown-nagi-user.json:2-16) and has no installed target
standard library. This confirms that the Mesa source candidate is reusable
after a real cross-toolchain/build integration, but no current local toolchain
can validate that build.

C:\Windows\System32\opengl32.dll exists, but no system libEGL.dll,
libGLESv2.dll, or osmesa.dll was found. The Windows OpenGL DLL is a host
library and is explicitly not a guest rendering dependency.

## Smallest Nagi-owned patch/adapter surface

The minimum surface should be:

1. third_party/servo-patches/0001-...patch (numbered and applied by the
   existing patch boundary): remove the unconditional shared-paint sm-x11
   selection for Nagi and select a Nagi-only surfaceless/static-EGL path.
   Avoid direct edits to third_party/servo. The untracked
   third_party/surfman-patches/0001-nagi-static-mesa-backend.patch is a useful
   concrete candidate for the Surfman portion, but its application/lock
   boundary still needs to be made reproducible.
2. A small Nagi Surfman/EGL patch or adapter: use the public
   mesa_surfaceless construction, or equivalent direct Surfman device types;
   provide a guest EGL loader that does not require a host display. Static
   Mesa/EGL symbol resolution is the smaller M17 choice. A general dynamic
   loader can be pursued only if the target std/relibc/ELF work is already
   available.
3. A new std-enabled adapter crate at the plan-prescribed
   apps/albert/servo-nagi/ (currently absent), or an explicitly chosen
   equivalent. It implements the mandatory RenderingContext methods, owns the
   320x200 context/surface, supplies gleam and glow, performs real readback,
   and delegates the final copy to user/nagi-servo::NagiSurface.
4. A small EventLoopWaker implementation wrapping
   user/nagi-servo::EventLoopSignal, plus a WebViewDelegate whose frame callback
   calls paint, reads the image, copies it, and invokes the existing
   capability-checked present operation.
5. Target build integration for the Mesa static archives, patched std/relibc
   and the final Nagi linker. user/nagi-servo itself should remain the small
   no_std boundary unless a narrowly evidenced API addition is necessary.

No new kernel browser syscall is needed. The adapter must not enable X11,
Wayland, WGL, host OpenGL, WebRender screenshot plumbing, or arbitrary host
filesystem access.

## Nagi-fixable blockers versus external assets

| Category | Finding | Ownership |
|---|---|---|
| Backend selection | Current Servo shared paint requests sm-x11; Surfman default Unix aliases can choose Wayland/X11. | Nagi-owned Servo/Surfman patch |
| EGL loading | Current Surfman non-Windows path uses dlopen/dlsym; Nagi relibc excludes the required modules for target_os=nagi. | Nagi target/runtime work; static symbol path is smaller |
| Adapter | No RenderingContext implementation exists in the Nagi tree. | Nagi-owned adapter |
| Frame handoff | NagiSurface, Surface VMO, capability, and present ABI already exist. | Reusable; no kernel expansion needed |
| Target build | No Nagi std target artifact; prior status records missing std/linker/CRT support. | Nagi target/relibc/linker integration; host MSVC errors are development-host evidence only |
| Mesa source | A matching Mesa 24.3.0 source cache exists, and a dirty lock/bootstrap candidate exists, but the authoritative third_party/mesa checkout is absent and no build is verified. | Upstream source is external; Nagi must pin, fetch, patch, preserve notices, and integrate it |
| C cross tools | clang, LLVM binutils, lld, and pkg-config are absent. | External toolchain asset or reproducible toolchain provisioning |
| Host GL DLLs | opengl32.dll is present but is host Windows GL; no guest EGL/OSMesa binary exists. | Not reusable; prohibited by the architecture |

## Required next proof, not performed here

The following are the smallest later checks that would turn this investigation
into M17 evidence. They were intentionally not run because this request was
read-only and prohibited builds/fetches:

    # only after the dirty Mesa candidate is explicitly approved for execution
    nagi fetch
    meson setup out/mesa-build third_party/mesa \
      --cross-file tools/mesa/nagi-x86_64-user.meson.cross \
      -Dgallium-drivers=softpipe -Dvulkan-drivers= \
      -Dplatforms=auto -Degl-native-platform=surfaceless \
      -Dglx=disabled -Dllvm=disabled -Dshared-glapi=enabled \
      -Dosmesa=true -Dprefix=<absolute-staging-prefix>
    ninja -C out/mesa-build
    cargo test -p nagi-servo-adapter
    cargo build -p nagi-servo-adapter --target targets/x86_64-unknown-nagi-user.json

Before accepting those results, inspect the resolved feature graph to prove that
Nagi does not select sm-x11 or Wayland, link the actual guest Mesa/EGL archives
into the user image, render a bundled local page, read back exactly 320x200 RGBA
pixels, copy them through NagiSurface, and require a nonzero guest framebuffer
checksum/serial marker. A host build or a successful source fetch alone is not a
rendering result.

## Read-only evidence commands

The investigation used read-only metadata/search commands, including:

    git status --short --branch
    git diff --stat
    git -C third_party/servo rev-parse HEAD
    git -C third_party/servo status --short --branch
    rg -n 'RenderingContext|SurfmanRenderingContext|SoftwareRenderingContext|read_pixels|create_webrender_instance|notify_new_frame_ready' third_party/servo
    rg -n 'surfman|webrender|gleam|glow|sm-x11' third_party/servo/Cargo.toml third_party/servo/components
    git -C out/cache/mesa-inspect-24.3.0 rev-parse HEAD
    git -C out/cache/mesa-inspect-24.3.0 status --short --branch
    git check-ignore -v -- out/cache/mesa-inspect-24.3.0
    rg --files out/cache/mesa-inspect-24.3.0 | rg 'softpipe|src/egl|platform_surfaceless|build.ninja'
    Get-Command cargo,rustc,rustup,cmake,ninja,meson,clang,llvm-ar,ld.lld,lld-link,link
    rustup toolchain list
    rustup target list --installed
