# Decision 0017: Servo pin and Nagi adapter boundary

## Decision

Nagi 0.1 uses Servo revision
`b820a9679a784877f91b4acc90c2c6e849f18d3b` from
`https://github.com/servo/servo.git`. The revision, source identity, license,
and source paths are recorded in `third_party/sources.lock`.

The parent repository does not vendor the large Servo checkout. `nagi fetch`
bootstraps the exact detached revision into the generated
`third_party/servo/` path, writes its generated `REVISION` marker, validates
the checkout commit and clean state, and fetches its locked Cargo sources.
Nagi-specific changes belong to the tracked `third_party/servo-patches/`
boundary. A wrong-revision or dirty existing checkout is rejected without
resetting or replacing it.

Albert remains the Servo embedder. The integration boundary is the Servo
`EventLoopWaker`, `RenderingContext`, `WebView`, and `WebViewDelegate` API.
Servoshell is used as an API/build reference only; it is not the final Nagi
browser application.

The first rendering path is software-only: Servo produces an RGBA image or
buffer, and a Nagi-owned adapter submits it through the capability-checked
Surface API. No host window, X11, Wayland, Chromium, or NetSurf path is a
valid substitute.

## Current state

The pinned source is reproducibly fetchable, but a Nagi-target build and a
guest-rendered first web pixel are not yet accepted. The pinned checkout's public
`SoftwareRenderingContext` currently depends on its Surfman/OpenGL software
adapter, so the Nagi path requires an explicit adapter/Softpipe integration
before the guest acceptance can pass.
