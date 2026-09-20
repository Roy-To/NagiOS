# ADR 0019: M17 requires a real guest software-rendering backend

## Status

Accepted as the M17 blocking record on 2026-09-19. M17 is `BLOCKED`, not
`PASS`: the Servo fetch/patch boundary is implemented, but the first guest web
pixel has not been produced.

## Context

M17 requires Servo to render a bundled local page inside Nagi user space and
deliver the resulting pixels through the capability-checked Nagi Surface and
display-present path. The implementation must not use X11, Wayland, a host
browser, host screenshots, a host filesystem, or a fake/no-op GL context.

The pinned Servo revision is
`b820a9679a784877f91b4acc90c2c6e849f18d3b`. Its public
`RenderingContext`/WebRender path requires genuine GL operations and its
current shared paint manifest enables Surfman's `sm-x11` feature. Servo's
`SoftwareRenderingContext` is an OpenGL/Surfman software adapter, not a CPU
HTML-to-RGBA renderer. Nagi has no pinned Mesa source or Softpipe build path
yet.

## Evidence

- `cargo check --manifest-path third_party/servo/Cargo.toml -p servo --target targets/x86_64-unknown-nagi-user.json --no-default-features`
  fails because the custom target has no installed `core`/`std`.
- The same check with `-Zbuild-std=std,panic_abort` reaches standard-library
  compilation but fails because the host MSVC `link.exe` is unavailable.
- A temporary `lld-link.exe`/Windows SDK attempt reaches link resolution but
  fails on missing MSVC CRT/runtime symbols including `mainCRTStartup`,
  `memcpy`, `__CxxFrameHandler3`, `_tls_index`, and `_CxxThrowException`.
- Read-only source inspection shows Servo paint currently requests Surfman
  `sm-x11`; the Nagi repository has no pinned Mesa/Softpipe source entry.
- The existing `user/nagi-servo` crate provides bounded Surface copy, input,
  and wake primitives, but it cannot satisfy Servo's GL-backed
  `RenderingContext` contract by itself.

## Decision

Keep M17 blocked until all of the following are implemented and reproducible:

1. A pinned Nagi-compatible Mesa Softpipe or equivalent guest software-GL
   source/build entry, including source hash, license, and Nagi patch boundary.
2. A Nagi Servo/Surfman/WebRender adapter selected for `target_os = "nagi"`
   without X11 or Wayland features.
3. A target build with the required patched `std`/relibc and linker/runtime
   support.
4. A real Servo WebView frame readback into `NagiSurface`, followed by a QEMU
   serial marker and nonzero checksum proving `First Web Pixel on Nagi`.

The fetch implementation in `5672893` remains useful and is retained: it
applies sorted tracked patches, records generated-checkout fingerprints, and
refuses unsafe or stale generated state without overwriting it.

## Remediation continuation (2026-09-19)

The `BLOCKED` status is retained as the historical acceptance state, not as a
stop instruction. The actionable blockers were reclassified and work
continues inside M17:

- Mesa 24.3.0 is now pinned at
  `f1f246cfda65eff82fba3be1caf2d23bdeda60cc`, with a tracked Nagi platform
  and static Softpipe patch boundary;
- Surfman is pinned at
  `205778f497327c573929c7b471194390e15f331d`, with a Nagi static EGL,
  surfaceless adapter that excludes X11 and Wayland target dependencies;
- Servo's local libc 0.2.189 source and workspace boundary are tracked, and
  the Nagi Albert adapter calls Servo's real software `RenderingContext` and
  `WebView::paint` path;
- the current next experiment is the Ubuntu target build: generate relibc C
  headers, compile Mesa/Softpipe, aggregate its target-owned static archives,
  build Servo with patched std, and run QEMU.

The local Windows MSVC linker/CRT absence is recorded as a host verification
limitation only. It is not an external product blocker and does not permit
host rendering or synthetic pixel evidence.

## Remediation continuation (2026-09-20)

CI run `35501349699` verified the pinned Tokio adapter and stopped at
`getrandom 0.4.3`, whose default backend rejects `target_os = "nagi"`. This
is an internal target prerequisite. The remediation adds the real QEMU
VirtIO-RNG device to Nagi's existing low-level device boundary, exposes a
bounded `SYS_RANDOM_GET` syscall with user-range validation, and selects
getrandom's custom backend from `libnagi`. No host RNG, RDRAND fallback,
fixed seed, Unix device, or unsupported-success result is used. The target
build and guest entropy behavior remain to be verified before the M17 gate.

## Remediation continuation (2026-09-20, follow-up)

CI run `35502681902` compiled the target dependencies, Servo bootstrap, Mesa
Softpipe, package/UEFI, kernel, `mio`, `socket2`, `tokio`, and the new
`getrandom` backend. It then stopped in `freetype-sys 0.23.0`: the pinned
WebRender 0.70 glyph rasterizer selects FreeType through its legacy Unix
condition, which is separate from Servo's `bundled_freetype` feature and
therefore called cross-compiling host `pkg-config`.

This is an internal Nagi build prerequisite, not a Windows MSVC limitation.
The Nagi Albert target dependency now explicitly enables the existing pinned
`freetype-sys` `bundled` feature. That keeps FreeType as a real target-owned
source build and does not replace font rasterization with a stub or host
library. The next CI target build must verify the bundled C build and continue
to the next unresolved Servo/runtime boundary.

## Remediation continuation (2026-09-20, IPC follow-up)

CI run `35503537693` verified the bundled FreeType build and reached the Servo
user-init compile. It then stopped in `ipc-channel 0.23.0`: the crate only
selects its Unix backend for Linux/BSD/illumos and cfgs every backend out for
`target_os = "nagi"`.

M17 does not enable Servo's multiprocess feature, so the Nagi adapter selects
ipc-channel's existing `force-inprocess` backend. That backend is a real
crossbeam-backed same-process/thread transport; it does not route IPC through
the host or return synthetic success. The next target build must verify this
selection and expose the next Servo/runtime prerequisite.

## Remediation continuation (2026-09-20, allocator follow-up)

CI run `35504269079` verified the in-process `ipc-channel` backend and reached
Servo's allocator compile. It then stopped because `servo-allocator` calls
`libc::malloc_usable_size`, which was absent from the pinned Nagi libc ABI.

The Nagi POSIX heap already records each requested payload beside its
allocation and validates that metadata when releasing blocks. It now exposes
that recorded payload through `nagi_posix_malloc_usable_size`; the pinned
Servo-libc patch declares `malloc_usable_size`, and relibc forwards the libc
call to the Nagi POSIX runtime. This preserves real Servo allocator
introspection without disabling the allocator or consulting a host allocator.
The next target build must verify this ABI and continue to the next boundary.

## Remediation continuation (2026-09-20, patch-hunk follow-up)

CI run `35505340415` reached the target user-init compile after the allocator
repair, then reported an unclosed `cfg_if!` delimiter in the generated
`third_party/libc-servo/src/unix/nagi.rs`. The tracked Nagi libc patch's new-file
hunk contained 1,412 added lines but declared 1,411, so patch application
silently omitted the final closing delimiter. The hunk header is corrected to
the actual pinned patch content. This keeps generated third-party sources
reproducible and does not edit the generated checkout directly.

The same pinned-source audit identified the next compile-required cfg gap:
Servo's x86_64 target dependency conditions include Nagi in the Linux-like
`gaol` sandbox path, but gaol has no Nagi platform implementation. The tracked
`0004-nagi-single-process-no-gaol.patch` excludes Nagi from the gaol dependency,
Linux profile, and multiprocess spawn branches, allowing M17's existing
single-process embedder to use its explicit unsupported path. It does not add
host process spawning or claim a Linux sandbox on Nagi; target CI must verify
the resulting dependency graph and expose any remaining runtime boundary.

## Remediation continuation (2026-09-20, target C ABI follow-up)

CI run `35505820899` reached `aws-lc-sys` after the Servo libc and gaol cfg
repairs. Its `cc-rs` build used host `cc` and did not receive Nagi's generated
relibc headers. Under strict C11 feature visibility, the host pthread header
did not expose `pthread_rwlock_t` or `PTHREAD_RWLOCK_INITIALIZER`. Adding a
host feature macro alone would be an ABI error: the host rwlock layout is not
the four-byte rwlock object implemented by Nagi relibc.

The target C boundary now routes `cc-rs` through the tracked
`tools/nagi-target-cc.sh` wrapper. It selects freestanding x86_64 ELF,
Clang's resource headers, and the generated Nagi relibc headers while excluding
host standard include directories. The Mesa bootstrap runs a focused
`pthread_rwlock_t` size/initializer syntax check before compiling Softpipe.
This is a real target ABI repair, not a host stub or a rendering shortcut.
Target CI must verify the aws-lc objects, final link, and then the real QEMU
first-web-pixel gate.

## Remediation continuation (2026-09-20, target C header overlay follow-up)

CI run `35507108032` passed the generated rwlock ABI check, Mesa/Softpipe,
package/UEFI, M16 package, kernel, and the `aws-lc-sys` C compilation after
the wrapper repair. It then stopped in bundled `libz-sys 1.1.29`: the gzip
sources use `O_RDONLY`, `O_WRONLY`, `O_CREAT`, `O_TRUNC`, and `O_APPEND`, while
the generated relibc fcntl header did not expose the Nagi target constants in
that direct C build.

The tracked `tools/mesa/nagi-headers/fcntl.h` already provides those exact
Nagi ABI values and includes the generated header through `include_next`. The
target C wrapper and the Mesa preflight now put that overlay before generated
relibc headers. No host fcntl header or host zlib library is introduced.
Target CI must verify bundled zlib compilation and continue to the next
Servo/runtime prerequisite.

## Remediation continuation (2026-09-20, FreeType bundled include follow-up)

CI run `35507706711` verified the fcntl overlay through bundled zlib and
reached `freetype-sys 0.23.0`. Its bundled libpng C compile then failed to
find `zlib.h`: the pinned build script passed the relative
`libz-sys/src/zlib` path, but Nagi's freestanding target wrapper correctly
excludes host include directories and that relative path is not valid from the
Cargo build invocation.

The exact `freetype-sys 0.23.0` registry source is now included in the same
source-lock, generated-checkout, patch-fingerprint, and Nagi patch boundary as
the other target prerequisites. The tracked patch uses `DEP_Z_INCLUDE`, which
is emitted by the pinned `libz-sys` dependency after it selects its real
target-owned zlib build. It does not add a host zlib path or replace the
FreeType/libpng build. The next target build must verify this C boundary and
continue to the next Servo/runtime prerequisite.

## Remediation continuation (2026-09-20, aws-lc C ABI follow-up)

CI run `35508629491` verified the pinned FreeType/libz repair and reached the
real `aws-lc-sys` target C build. The next internal boundary was Nagi's
relibc headers: `stdatomic.h` preserved the `_Atomic` qualifier in temporary
objects passed to Clang's generic `__atomic_*` builtins, and target cbindgen
selected no `struct termios` definition because the redox-compatible layout
was not enabled for `target_os = "nagi"`.

The relibc header implementation now uses unqualified temporary value types
for the generic atomic operations while keeping the atomic pointer itself,
and selects the existing redox-compatible `termios` layout for Nagi header
generation. Mesa bootstrap performs a target C syntax/ABI preflight for both
interfaces before Softpipe compilation. No host atomic library or host
termios header is introduced; target CI must verify aws-lc and continue to
the next Servo/runtime prerequisite.

## Remediation continuation (2026-09-20, cbindgen and Clang C11 follow-up)

CI run `35509724031` exposed two details in the new preflight itself. cbindgen
requires an explicit `target_os = "nagi"` entry in its defines table before
it emits the target `struct termios`; without it, only incompatible function
prototypes were emitted. Clang also rejects its generic `__atomic_*` builtins
when the address is a C11 `_Atomic` object, so relibc now selects Clang's
native `__c11_atomic_*` operations for the target C ABI and retains the
generic path for non-Clang consumers. The next target run must verify the
preflight, aws-lc, and the following user-init dependency boundary.

CI run `35510072330` verified the Clang C11 atomic repair. Its remaining
preflight failure showed that cbindgen emitted a `__nagi__` guard while the
established target wrapper defines `__NAGI__`. The source-lock-preserved
header generation boundary now maps `target_os = "nagi"` to the existing
uppercase guard; no host header is involved. The next target run must verify
the complete termios header and proceed to aws-lc.

## Remediation continuation (2026-09-20, hyper-util Unix connector follow-up)

CI run `35510401081` passed the generated termios and C11 atomic preflight,
Mesa Softpipe, all package/kernel prerequisites, and the real `aws-lc-sys`
C build. User-init then reached `hyper-util 0.1.20`; its legacy HTTP connector
has an unconditional Unix `Connection` implementation whenever Rust reports
`cfg(unix)`, but the pinned Nagi tokio adapter correctly excludes Unix-domain
socket types because Nagi has no such implementation in this vertical slice.
The exact registry source is now pinned under the existing source-lock,
generated-checkout, fingerprint, and patch boundary. Its single tracked patch
excludes only that unused Unix connector for `target_os = "nagi"`; no host
socket implementation is introduced. The next target run must verify this
boundary and continue user-init compilation.

CI run `35511522322` was not a source/build result: target, Ubuntu, and
Windows jobs all failed during Actions startup with zero executed steps, and
rerun attempt 2 reproduced the same condition. It is retained as CI
infrastructure history; the hyper-util patch remains unverified until a
normal target job executes it.

CI run `35511637360` reproduced the same all-job startup failure for commit
`274a11f`; attempts 2 and 3 also ended without executing a step. The target
spec independently confirms that Nagi has both `target_family = "unix"` and
`target_os = "nagi"`, which is the exact cfg combination handled by the
tracked hyper-util patch. No target compile result is claimed until Actions
executes a normal job.

## Remediation continuation (2026-09-20, WebDriver feature boundary)

Public snapshot CI run `35520298442` verified the executable-mode repair,
Servo bootstrap, Mesa Softpipe archive, M16 package, kernel, Ubuntu host, and
Windows launcher. Target user-init then failed while compiling `warp 0.4.3`:
the Servo workspace's default `webdriver` feature enabled the server runtime
for the embedded `script` dependency, and Warp selected Unix listener types
under `cfg(unix)`. Nagi deliberately reports the Unix target family while
excluding Unix-domain sockets and Unix signal APIs, so adding those APIs would
be an architecture regression.

The tracked Servo patch boundary now sets the workspace `webdriver`
dependency to `default-features = false` and enables `features = ["server"]`
only for Servo's standalone `webdriver_server` package. Script retains the
real WebDriver protocol types; the embedded Nagi graph no longer requests the
unused server runtime. This is a source-pinned feature-boundary repair, not a
Warp stub or host rendering shortcut. The next public target run must verify
the graph and continue to UEFI and the real first-web-pixel acceptance.

## Remediation continuation (2026-09-20, locked graph follow-up)

Public snapshot CI run `35521317614` verified the pinned Servo bootstrap on
all three jobs, then stopped before compilation of the host and target
workspaces because the committed `Cargo.lock` still described the old
default-feature graph. Ubuntu and Windows reported that the lock file needed
an update under `--locked`; the target dependency preflight rejected the same
stale graph. Cargo regenerated the lock from the pinned Servo manifests after
the feature-boundary patch, removing `warp`, its server-only transitive
packages, and the `webdriver` server-only `tokio` edge. The regenerated lock
passes locked offline metadata and the target dependency graph preflight
without `warp` or `servo-webdriver-server`. No server API, host fallback, or
rendering shortcut was added. The next public run must verify the lock in CI
and continue through target build, UEFI, and real QEMU first-web-pixel
acceptance.

## Remediation continuation (2026-09-20, Surfman parent-workspace binding)

Public snapshot CI run `35521923686` passed the locked graph boundary, Mesa
Softpipe archive, package/kernel prerequisites, and Servo bootstrap, then
reached the real Nagi user-init build. It failed while compiling
`libloading 0.8.9`: the Nagi parent workspace resolved registry Surfman, so
Surfman's Unix-wide Wayland dependency pulled `dlib` and dynamic loader code
into the Nagi target. This was a source-binding defect, not evidence that Nagi
needs host display APIs. The generated pinned Surfman checkout already carries
the tracked Nagi patch that excludes Wayland/X11/dlopen for `target_os =
"nagi"` and selects the static Mesa surfaceless backend.

The Nagi root workspace now binds `surfman` through its `[patch.crates-io]`
table, and the lock records the generated path package. The target graph
preflight additionally rejects `libloading`, `dlib`, and `wayland-sys` so a
future source-binding regression stops before the full target build. Local
locked metadata and target graph checks pass with the generated checkout;
real CI target build, UEFI, and QEMU first-web-pixel evidence remain required.

## Remediation continuation (2026-09-20, Nagi tempfile filesystem backend)

Public snapshot CI run `35522589749` passed the Surfman parent-workspace
binding and reached the Nagi user-init target build. The next failure came
from `tempfile 3.27.0`, which selected its Unix `rustix` backend solely
because Nagi intentionally reports `target_family = "unix"`. `rustix` then
compiled host-oriented filesystem APIs against Nagi libc and failed on 43
missing declarations, including `statfs`, `dup3`, fcntl locking/fallocate
constants, and related types. Adding fake libc symbols or weakening the target
ABI would be incorrect.

The pinned `tempfile` source is now materialized through the existing
registry-source lock and patch-fingerprint mechanism. Its Nagi-specific file
backend uses Nagi's real std/VFS operations for create, unlink, clone/reopen,
rename, and hard-link persistence, while the upstream rustix backend remains
selected on supported Unix targets. The Nagi root workspace binds this source,
and target CI rejects `rustix` in the Nagi dependency graph before compilation.
The next public run must verify the backend, target build, UEFI, and real QEMU
first-web-pixel acceptance.

## Remediation continuation (2026-09-20, fresh-cache registry bootstrap)

Public snapshot CI run `35523783329` failed in the bootstrap step on all three
jobs before reaching the Servo target build. The fresh Public repository did
not have a Cargo registry source cache, and `nagi-bootstrap` treated the cache
as a prerequisite instead of fetching the exact source named by
`third_party/sources.lock`. The concrete errors were `cannot read Cargo
registry source cache .../registry/src: ... No such file or directory` on both
Ubuntu and Windows.

The registry bootstrap now creates a repository-external temporary Cargo
manifest with the exact `=version` from the source specification and asks
Cargo to fetch it into the configured Cargo home. It then uses the existing
checksum, ordered patch, source lock, and checkout fingerprint checks before
installing the generated checkout. This repairs fresh CI initialization while
preserving the pinned source boundary; it does not substitute an unpinned
latest dependency or a host rendering path.

Run `35524127951` then reached this fallback on every job and exposed a second
bootstrap-only defect: Cargo rejected the generated manifest with `no targets
specified in the manifest`. The temporary manifest now includes an empty lib
target, used only to let Cargo generate/fetch its lockfile. It does not alter
the Nagi workspace or the generated pinned source validation boundary.

## Remediation continuation (2026-09-20, mozjs_sys Nagi target adapter)

Run `35524229892` passed the clean-runner bootstrap and all prerequisite stages,
then reached `mozjs_sys v153.0.0-2` during the real Nagi user-init build. Its
source build invoked Mozilla's GNU `config.sub` with
`x86_64-unknown-nagi-user`, which failed because that script interprets the
Rust target's `user` component as an unknown operating system. This was a
target adapter gap, not evidence that the JS engine could be replaced or
executed on the host.

The pinned registry source now has a Nagi-owned patch that gives Mozilla's
configure layer the explicit freestanding `x86_64-unknown-nagi` triplet while
retaining the Nagi target compiler, generated relibc headers, and final guest
link. The source lock, bootstrap, patch fingerprint, and root Cargo path
binding are tracked. The next run must verify the real SpiderMonkey cross-build
and expose the next compile or link boundary.

## Remediation continuation (2026-09-20, configure triplet correction)

Public snapshot CI run `35525315524` reached the real `mozjs_sys` build, but
the configure-only `x86_64-unknown-elf` fallback was rejected by the pinned
Mozilla `config.sub` as `OS 'elf' not recognized`. This was a defect in the
Nagi-owned adapter patch, not a reason to replace the target build with a host
build. The patch now uses the script's accepted freestanding
`x86_64-unknown-none` form. The Nagi Rust target, compiler wrapper, generated
relibc headers, and guest link remain unchanged. UEFI and real QEMU pixel
acceptance were skipped by the failed user-init prerequisite and remain open.

## Remediation continuation (2026-09-20, native Nagi configure OS)

Public snapshot CI run `35526109052` reached `mozjs_sys` after the previous
triplet correction, but Mozilla's configure `split_triplet()` rejected
`x86_64-unknown-none` with `Unknown OS: none`. The next adapter uses an
explicit `x86_64-unknown-nagi` configure triplet and adds Nagi to the pinned
configure OS/kernel enums and preprocessor checks. It also suppresses the
generic `libm` linkage that is not a Nagi runtime dependency. This is a real
Nagi target adapter, not a Linux/WASI label or host fallback. UEFI and real
QEMU pixel acceptance remain open until user-init, UEFI, and the acceptance
wrapper all pass.

## Remediation continuation (2026-09-20, target LLVM prerequisite)

Public snapshot CI run `35527150705` passed the native Nagi configure adapter:
Mozilla recognized `x86_64-unknown-nagi`, detected the Nagi kernel through
`__NAGI__`, and selected the real Nagi compiler wrapper. It then rejected the
runner's actual Clang `18.1.3` because this pinned SpiderMonkey source requires
Clang/LLVM 19 or newer. The remediation pins the CI dependency to the Ubuntu
24.04 `clang-19`/`lld-19` packages and selects them explicitly. The mozjs
adapter also routes Nagi C/C++ preprocessing through the Nagi freestanding
wrapper and avoids requesting a host `stdc++` runtime for `nagi-user`. No host
rendering or synthetic pixel path is introduced. UEFI and real QEMU pixel
acceptance remain open.

## Remediation continuation (2026-09-20, Nagi archiver binding)

Public snapshot CI run `35528078834` passed the pinned Mesa Softpipe archive,
UEFI dependency, M16 package, and Nagi kernel stages. The real `mozjs_sys`
configure then reached Clang 19 and native Nagi detection but failed at its
archiver probe because `makefile.cargo` inherited
`AR=x86_64-unknown-nagi-user-ar`, a target-prefixed GNU executable that is not
part of the Nagi toolchain. The next adapter patch binds `AR` to the pinned
`llvm-ar` used by the Nagi Mesa path and records a regression assertion for
that contract. This changes only tool selection; it does not add a host
runtime, host rendering, or synthetic pixel path. UEFI and real QEMU pixel
acceptance remain open.

## Remediation continuation (2026-09-20, Nagi TimeStamp platform shim)

Public snapshot CI run `35528990869` passed the archiver boundary and all
prior target prerequisites, then stopped in Mozilla's `timestamp.mozbuild`
with `No TimeStamp implementation on this platform` for Nagi. The Nagi
platform already exposes a real monotonic clock through the kernel timer,
`libnagi`/`nagi-posix`, and generated relibc headers. The next ordered adapter
patch therefore selects Mozilla's existing POSIX `TimeStamp` source for
`OS_TARGET == "Nagi"`; it does not provide a host clock or loop-counter
substitute. UEFI and real QEMU pixel acceptance remain open.

## Remediation continuation (2026-09-20, target C++ standard headers)

Public snapshot CI run `35529920163` passed the Nagi TimeStamp platform shim
and reached Mozilla's next real compile boundary. Its freestanding wrapper
correctly rejected host system includes, but no C++ standard header root had
been supplied, so the first required `<cstddef>` header was unavailable. The
next repair installs the pinned Ubuntu noble `libc++-19-dev` headers and passes
their explicit `/usr/include/c++/v1` root through `NAGI_CXX_HEADERS`. This is
compile-time header provisioning only; Nagi relibc remains the C header and
runtime boundary, and no host C++ runtime link is introduced. UEFI and real
QEMU pixel acceptance remain open.

## Remediation continuation (2026-09-20, libc++ and Nagi header ordering)

Run `35530635419` confirmed that the pinned libc++ headers were installed and
the explicit `NAGI_CXX_HEADERS` hook resolved the initial `<cstddef>` absence.
The next real failure showed that Mesa's intentionally minimal
`tools/mesa/nagi-headers/type_traits` was shadowing libc++ and that libc++
`include_next` probes could not reach the Nagi C header boundary in the prior
order. The follow-up wrapper repair keeps libc++ first and moves the Nagi
Mesa/relibc compatibility paths behind it with `-idirafter` only for the C++
header-enabled path. This remains a target header-order repair, not a host
runtime or rendering fallback. UEFI and real QEMU pixel acceptance remain
open.

## Remediation continuation (2026-09-20, libc++ thread backend)

Public snapshot CI run `35531324244` (head `7210f01`) passed the previous
libc++ header-order boundary and entered the real MozJS C++ compile. The
target then stopped in libc++ `__config` with `No thread API`: Nagi's custom
target triple is not one of libc++'s platform auto-detection cases. This was
not a missing host library. The Nagi runtime already exports the POSIX pthread
ABI through `nagi-posix`, including the symbols required by the target
compatibility boundary. The ordered `0005-nagi-libcxx-thread-api.patch` now
selects libc++'s pthread backend for Nagi in the mozjs build flags. It does
not add a host pthread/C++ runtime, disable threading, or fake a rendering
result. The target build must be rerun; UEFI and real QEMU first-web-pixel
acceptance remain open.

## Remediation continuation (2026-09-20, libc++ rune table)

Public snapshot CI run `35532340846` (head `eb37984`) passed the explicit
libc++ pthread backend selection and reached libc++ locale headers. The next
real target compile failure was `unknown rune table for this platform`.
Nagi's current 0.1 target runtime does not provide a host locale database,
and importing one would violate the freestanding guest boundary. The ordered
`0006-nagi-libcxx-rune-table.patch` therefore selects libc++'s portable default
rune table for Nagi. This supplies the header-level ctype masks required by
MozJS without host locale state or synthetic rendering behavior. The target
build must be rerun; UEFI and real QEMU first-web-pixel acceptance remain
open.

## Remediation continuation (2026-09-20, libc++ localization boundary)

Public snapshot CI run `35533256072` (head `b1d9ff5`) passed the portable
rune-table selection and reached libc++'s optional localization layer. The
next real target compile failure was the absence of Nagi `_l` locale functions
such as `strtoll_l` and `strtod_l`. Nagi must not import a host locale
database for this boundary; MozJS already has its pinned ICU implementation
for Unicode/locale behavior. The ordered `0007-nagi-libcxx-no-localization.patch`
therefore selects libc++'s supported no-localization configuration for Nagi.
It does not replace ICU, add host locale state, or fake rendering. The target
build must be rerun; UEFI and real QEMU first-web-pixel acceptance remain
open.

## Historical continuation: run #20 exposed an over-broad workaround

Public snapshot CI run `35534135637` (head `5965075`) passed the previous
missing `_l` declarations only because `0007` disabled libc++ localization,
then failed earlier in `streambuf` with missing `streamsize` and incomplete
`std::ios_base`. This proves that the broad no-localization switch cannot be
the M17 solution. The next repair keeps libc++ localization enabled and adds
the target-owned Nagi relibc numeric conversion ABI: real `strto*` parsing,
C/POSIX `_l` wrappers, and generated-header declarations. Ordered patch
`0008` explicitly removes the diagnostic define after `0007`; it does not
import host locale state or add a rendering fallback. UEFI and real QEMU
first-web-pixel acceptance remain unexecuted.

## Remediation continuation (2026-09-21, target font and mmap boundary)

Public snapshot CI run `35535949462` (head `cd3ef01`) verified that the
Nagi-owned numeric `strto*`/`_l` ABI and restored libc++ localization moved the
real target compile past the previous `streambuf` failure. It then exposed two
independent Nagi target gaps: Servo's shared font identifier had no platform
module for `target_os = "nagi"`, and the Nagi C++ header overlay did not expose
the already-defined Nagi `PROT_NONE` and `MAP_FIXED` mmap values.

The ordered Servo `0006` patch enables its existing pinned FreeType backend
for Nagi, uses the real Nagi VFS/mmap font-data path, and adds a Nagi-owned
system-font registry boundary that reports no system fonts until a Nagi font
package service exists. It does not use host fontconfig, DirectWrite, CoreText,
or guessed host paths. The Mesa header overlay now exports `PROT_NONE = 0` and
`MAP_FIXED = 0x10`, matching `third_party/libc/src/unix/nagi.rs`; no runtime
flag value is invented. The local target check reaches the known Windows
`link.exe` limitation, while the patch forward/reverse checks and CLI source
contract check pass. Target CI must verify the FreeType C build, continue to
the target link, and then execute the real UEFI/QEMU first-web-pixel gate.

## Remediation continuation (2026-09-21, Nagi pthread naming ABI)

Public snapshot CI run `35538673589` (head `9da3875`) passed the Ubuntu and
Windows jobs, Mesa Softpipe, Servo bootstrap, target feature boundary, M16
package artifact, and Nagi kernel target build. The next real user-init
compile reached MozJS's pinned POSIX thread backend and stopped because the
generated Nagi `pthread.h` did not declare `pthread_setname_np` or
`pthread_getname_np`.

The Nagi relibc pthread boundary now stores a bounded 16-byte thread name in
the guest Pthread object using atomic bytes, exports both real name APIs, and
lets cbindgen expose them to MozJS. It does not call host thread APIs or
return synthetic success. The next target run must verify the generated C
header and MozJS build, then continue to UEFI and the real QEMU pixel gate.

## Remediation continuation (2026-09-21, MozJS allocator header boundary)

Public snapshot CI run `35539581256` (head `fa79566`) verified the pthread
naming ABI and reached the pinned MozJS allocator compile. It then stopped in
`mozalloc.cpp:126` because the Nagi freestanding `stdlib.h` does not implicitly
declare the non-POSIX `malloc_usable_size` function from `malloc.h`.

The ordered MozJS patch `0009` includes the pinned Nagi relibc `malloc.h` only
under `__NAGI__`. This exposes the existing real guest allocator introspection
ABI to MozJS without consulting a host allocator, disabling accounting, or
changing the first-web-pixel acceptance. The next target build must verify the
patch and continue toward the UEFI and real QEMU gate.

## Remediation continuation (2026-09-21, condition-variable clock boundary)

Public snapshot CI run `35540890122` (head `b76d88d`) verified the allocator
header repair and reached MozJS's `ConditionVariable_posix.cpp`. Its target
configuration selected the macOS/Android-only
`pthread_cond_timedwait_relative_np`, which is not part of Nagi's guest ABI.

The next ordered MozJS patch selects the existing absolute timed-wait path for
Nagi and uses `CLOCK_REALTIME`, which is supported by the real Nagi relibc
condition-variable and clock interfaces. It does not add a host pthread API or
replace synchronization with a stub. The next target build must verify this
boundary and continue toward UEFI and the real QEMU gate.

## Exit criteria

Reopen M17 from this ADR after the guest rendering dependency is available.
Run the target build, focused adapter tests, and the QEMU acceptance wrapper.
Only then change the status to `PASS`; otherwise retain `BLOCKED` with updated
command output and the next concrete experiment.
