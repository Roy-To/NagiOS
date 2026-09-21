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

## Remediation continuation (2026-09-21, mmap signal boundary)

Public snapshot CI run `35542223324` (head `d56cc42`) verified the condition-
variable repair and reached MozJS's `MmapFaultHandler.cpp`. The pinned source
selected the Unix `sigaction` implementation and failed because the generated
Nagi signal header does not expose `SA_SIGINFO`, `SA_NODEFER`, or `SA_ONSTACK`.
This is consistent with the existing M17 architecture boundary: Nagi's
vertical slice does not expose Unix signal delivery, and its guest file mapping
facade is not a host mmap that delivers `SIGBUS`.

Ordered MozJS patch `0011` now selects the source's existing no-op
`MmapAccessScope` macro boundary for `__NAGI__` and excludes only the Unix
signal-handler implementation. It does not add unsupported signal constants,
call a host signal API, or claim memory-fault recovery. The target build must
verify this boundary before UEFI and real QEMU first-web-pixel acceptance can
execute.

## Remediation continuation (2026-09-21, bindgen target boundary)

Public snapshot CI run `35543941691` (head `9c01eae`) verified patch `0011` and
reached MozJS's bindgen phase. Clang rejected the Rust-only target spelling
`x86_64-unknown-nagi-user` with `version 'user' in target triple ... is
invalid`; the same invocation also could not find libc++ `<functional>` because
bindgen did not inherit the include arguments implemented inside
`tools/nagi-target-cc.sh`.

The Nagi-owned `mozjs-sys` build script now configures bindgen explicitly for
the freestanding compile boundary: canonical `x86_64-unknown-elf` target,
pinned libc++ headers, generated relibc headers, Mesa's Nagi overlay, and the
existing libc++ feature defines. This is a compile-time target mapping and
header boundary; it does not import host headers or a host C++ runtime. The
target build must verify bindgen and continue toward target link, UEFI, and
real QEMU first-web-pixel acceptance.

## Remediation continuation (2026-09-21, ordered-hunk placement repair)

Public snapshot CI run `35545417125` (head `21cf313`) passed the target
bootstrap and Mesa/kernel prerequisites, then failed compiling `mozjs_sys`:
`cannot find function configure_nagi_bindgen in this scope`. The cause was not
a missing Nagi API. Patch `0012` used line-number-only insertions; after the
earlier ordered patches changed `build.rs` line offsets, the helper landed
inside `link_static_lib_binaries` and the call landed inside the compiler
argument loop. Patch `0012` now uses stable source context for the bindgen
call and top-level helper placement. Applying the ordered build-script patches
to the pinned source was rechecked successfully. This repair changes only
patch reproducibility; the target build, UEFI, and real QEMU first-pixel gate
remain required.

## Remediation continuation (2026-09-21, bindgen-call placement repair)

The next public snapshot CI run `35546742142` (head `e832a26`) confirmed that
the helper itself was top-level, but failed with Rust syntax errors because the
line-number-only call hunk landed inside `builder.clang_arg(`, before its
`arg` expression. The ordered patch now uses source context for the completed
compiler-argument loop and the following WASI branch. Reapplying the ordered
patches reproduces a complete call after the loop and before WASI handling;
the target build, UEFI, and real QEMU first-pixel gate remain required.

## Remediation continuation (2026-09-21, bindgen include-order repair)

Public snapshot CI run `35547606552` (head `b3e8bb5`) passed the ordered-patch
placement boundary and reached the real bindgen invocation. Clang then failed
in pinned libc++ `<cstddef>` and `<cstdint>` because the bindgen arguments put
the clang resource directory before `/usr/include/c++/v1`; libc++'s
`include_next` could not resolve builtin `<stddef.h>` and `<stdint.h>`. The
existing Nagi target compiler wrapper establishes the correct freestanding
order, so patch `0012` now applies libc++ headers first, clang resource
headers second, and relibc/Mesa compatibility headers after them. UEFI and
real QEMU first-pixel acceptance remain required.

## Remediation continuation (2026-09-21, jsglue allocator platform boundary)

Public snapshot CI run `35548995494` (head `42a4597`) passed the corrected
bindgen header order and reached the pinned `src/jsglue.cpp`. Its system
allocator bridge rejected Nagi at two `unsupported platform` conditionals,
although Nagi already exposes the real relibc `malloc.h` and
`malloc_usable_size` ABI used by the earlier MozJS allocator patch. Ordered
patch `0013` selects that existing guest ABI under `__NAGI__` for the jsglue
size reporter. It does not call a host allocator or synthesize allocator
sizes. UEFI and real QEMU first-pixel acceptance remain required.

## Remediation continuation (2026-09-21, Navigator platform target branch)

Public snapshot CI run `35550551510` (head `c13d463`) passed the ordered
bindgen header boundary and the real jsglue allocator bridge, then reached
Servo Rust compilation. The pinned `script::dom::navigatorinfo` module had
`Platform()` implementations for Windows, Linux/BSD, macOS, and iOS, but no
implementation for Nagi. Both the window and worker Navigator bindings
therefore failed with `cannot find function, tuple struct or tuple variant
Platform in module navigatorinfo`.

Ordered Servo patch `0007-nagi-navigator-platform.patch` adds the explicit
`target_os = "nagi"` branch and returns the target-owned Web API platform
identifier `Nagi`. This is a platform-information compatibility boundary; it
does not render content, consult a host platform, or change the first-web-pixel
acceptance. The next target run must verify the patched Servo Rust graph and
continue to target link, UEFI, and real QEMU first-web-pixel evidence.

## Remediation continuation (2026-09-21, navigator patch hunk count)

Public snapshot CI run `35558265319` (head `c655aad`) localized the first
target compiler diagnostic to `navigatorinfo.rs:84:3`, reporting an unclosed
delimiter. Replaying the ordered patch against the pinned clean Servo source
showed that `0007-nagi-navigator-platform.patch` declared `+60,10` while its
hunk contained eleven resulting lines. `git apply --check` accepted the
malformed count, but the applied file omitted the Nagi function's closing `}`.
The patch now declares `+60,11`, and the Servo patch-boundary test asserts the
corrected hunk count and final closing line. This is a reproducibility repair
inside the tracked Nagi patch boundary; it does not weaken the target build or
guest rendering acceptance. The next target run must verify Servo compilation
and continue to target link, UEFI, and real QEMU first-web-pixel evidence.

## Remediation continuation (2026-09-21, Servo event-loop return contract)

Public snapshot CI run `35560131276` (head `5ab6b52`) passed the corrected
navigator patch and reached the Nagi-owned Albert embedder. The compile error
at `user/nagi-albert/src/lib.rs:124:16` was `error[E0600]: cannot apply unary
operator ! to type ()`. The pinned Servo `Servo::spin_event_loop` API is a
unit-returning heartbeat; the Servo embedder owns shutdown handling rather than
returning a boolean to the Nagi adapter. The adapter now calls the real
heartbeat directly and retains the existing signal/yield scheduling boundary.
This is an API-contract correction, not a host fallback or synthetic rendering
change. The next target run must verify the adapter and continue to target link,
UEFI, and real QEMU first-web-pixel evidence.

## Remediation continuation (2026-09-21, target linker diagnostic boundary)

Public snapshot CI run `35562090985` (head `a19cb94`) passed the corrected
navigator patch and Albert event-loop contract, then failed after compiling the
Nagi user-init graph with `error: linking with rust-lld failed: exit status: 1`.
The unauthenticated public job-log endpoint exposes the first linker error but
not the linker body or undefined-symbol list. The CI wrapper now extracts the
first `undefined symbol`, `undefined reference`, `rust-lld`, or `ld.lld` detail
from the captured target log and includes it in the next diagnostic annotation.
This is observability work only; it does not change linker behavior, add a host
library, weaken the Nagi target ABI, or alter the first-web-pixel gate. The next
target run must identify and repair the actual missing target-owned link input,
then continue to UEFI and real QEMU first-web-pixel evidence.

## Remediation continuation (2026-09-21, cc-rs host C++ runtime boundary)

Public snapshot CI run `35563574740` (head `7a9f1ba`) exposed the exact linker
diagnostic after the target user-init graph compiled:
`rust-lld: error: unable to find library -lstdc++`. The pinned `mozjs_sys`
build script had two independent C++ runtime paths. Its explicit
`link_static_lib_binaries` branch was already patched for Nagi, but its
`cc-rs` `Build::compile()` path retained `cc-rs`'s default `stdc++` request
for unknown non-MSVC targets. This was an internal Nagi integration defect,
not a missing host toolchain.

Ordered MozJS patch `0003` now sets `builder.cpp_link_stdlib(None)` for
`nagi-user` and places the explicit link branch behind the same target guard,
including when `CXXSTDLIB` is present. The patch-boundary test, clean-source
patch check, and patched `build.rs` probe pass locally. No host C++ runtime,
fake library, or synthetic rendering path was added. The next target run must
verify the link boundary and continue to UEFI and real QEMU first-web-pixel
acceptance.

## Remediation continuation (2026-09-21, shared cc-rs C++ boundary)

Public snapshot CI run `35566339373` (head `afcccae`) verified the MozJS
specific repair but still stopped at the same target linker diagnostic:
`rust-lld: error: unable to find library -lstdc++`. Source tracing found the
remaining C++ build scripts in the pinned `fontsan`, `harfbuzz-sys`, and
`glslopt` graph. They all use `cc-rs`'s shared default, so patching each
consumer independently would leave the target boundary incomplete.

The M17 source-lock/bootstrap path now pins `cc 1.4.6` and applies ordered
Nagi patch `0001`, which returns no inferred C++ standard library for
`target.os = "nagi"`. The patch is target-specific and preserves host,
Windows, Apple, BSD, Android, and WASI behavior. It removes no C++ objects,
does not add a host library or stub, and keeps ownership of the Nagi C++ ABI
with the target runtime/toolchain. The local patched-crate compile probe and
bootstrap CLI check pass; the local `cargo run ... fetch` remains unable to
link only because this Windows host lacks `link.exe`. The next public target
run must verify the generated checkout, locked graph, target link, UEFI, and
real QEMU first-web-pixel evidence.

## Remediation continuation (2026-09-21, final target-link duplicate symbols)

Public snapshot CI run `35568601044` (head `dcce137`) passed the pinned
bootstrap, Mesa, package, and kernel stages and reached final Nagi user-init
linking. The preceding shared `cc-rs` host-runtime failure was gone. The
target linker instead reported duplicate `softpipe_launch_grid`,
`softpipe_draw_vbo`, and `abort` symbols.

The Softpipe duplicates came from the Nagi build forcing every member of the
aggregated target-owned Mesa archive with `+whole-archive`, including static
members reachable through another archive path. The link boundary now uses
normal selective archive extraction, preserving the real Mesa/EGL/Softpipe
objects while allowing the linker to select each needed member once. The
`abort` duplicate is a separate ABI ownership collision: relibc provides the
strong target libc implementation while `nagi-posix` also exposed a strong
fallback. The Nagi POSIX fallback is now weak, so it remains available for a
narrow link without competing with relibc. No Mesa object was removed, no
host library or stub was introduced, and the real rendering path is unchanged.
The next target run must verify final link, UEFI, and the real QEMU first-web-
pixel gate.

## Remediation continuation (2026-09-21, target C++ runtime symbols)

Public snapshot CI run `35571409458` (head `8f2a773`) confirmed that the
duplicate Softpipe and `abort` symbols were removed and reached the final
target link. The linker then exposed the next target-owned ABI gap:
`__stack_chk_guard`, `__stack_chk_fail`, and `operator delete(void*)` were
unresolved.

The tracked Nagi-owned `tools/mesa/nagi-cxx-runtime.cpp` is compiled by
`user/nagi-init/build.rs` for `x86_64-unknown-none` without host C++ headers or
runtime libraries. It provides the required allocation and deallocation
operators through the real `nagi_posix_malloc`/`nagi_posix_free` allocator
boundary and routes stack-protector failure through the target abort boundary.
This preserves the real Servo/Mesa link path; it does not provide host
rendering or a synthetic pixel. The next run must verify the final target link,
UEFI, and real QEMU first-web-pixel evidence.

## Remediation continuation (2026-09-21, POSIX network ABI)

Public snapshot CI run `35574033249` (head `1a6ca9e`) verified that the
freestanding target C++ runtime resolved the previous `__stack_chk_*` and
`operator delete(void*)` link gaps. The next target linker diagnostic was the
Nagi POSIX network ABI: undefined `readv`, `shutdown`, and `setsockopt`.

The current repair adds `readv` over the existing Nagi user-space read path,
and maps socket shutdown, `TCP_NODELAY`, and POSIX receive/send timeouts from
the POSIX descriptor runtime to the existing capability-scoped `nagi-net`
SocketApi and smoltcp TCP socket. Unsupported socket options return
`ENOPROTOOPT`; no host socket, fake success, or synthetic rendering path is
used. The abort fallback is weak only for `target_os = "nagi"`, preserving the
target relibc ownership boundary without emitting weak COFF linkage for the
Windows host checks. The next CI run must verify final target linking and then
continue through UEFI and real QEMU first-web-pixel acceptance.

## Remediation continuation (2026-09-21, target thread/C++ ABI)

Public snapshot CI run `35580032533` (head `48f87a3`) first exposed a
target-only `u32`/`usize` comparison in the repaired socket-option ABI. Commit
`0a07fbd` corrected the `option_length` comparison and allowed the next run to
reach final target linking.

Run `35582239552` (head `0a07fbd`) passed the target compile boundary and
reported the next real Nagi ABI gap: undefined `pthread_equal`,
`pthread_setname_np`, and the libc++ nanosecond overload
`std::__1::this_thread::sleep_for(std::__1::chrono::duration<long long,
std::__1::ratio<1l, 1000000000l> > const&)`.

The current repair keeps the existing relibc implementations as the preferred
strong ownership where they are selected, and adds target-only weak fallbacks
in `nagi-posix` for the final-link path. Thread names are copied into bounded
Nagi user-space metadata; they never consult host thread state. The Nagi-owned
C++ runtime now defines the exact libc++ Itanium symbol and delegates its
nanosecond duration to `nagi_posix_sleep_ns`, which uses `GuestClock`. This is
an ABI completion, not a host C++ runtime, fake sleep, or rendering shortcut.
The next run must verify final target linking, UEFI, and real QEMU first-web-
pixel acceptance.

## Remediation continuation (2026-09-21, target relibc C runtime ABI)

Public snapshot CI run `35585927884` (head `4b8cb81`) confirmed that the
thread and libc++ ABI repair reached final target linking. The next diagnostic
was undefined `strcmp`, `atoi`, and `stderr`.

The source audit found that relibc's upstream `string`, `stdlib`, and `stdio`
modules are conditionally excluded when `target_os = "nagi"`. The Nagi target
therefore compiles only `third_party/relibc/src/nagi.rs`, so the existing
upstream strong definitions never enter the target archive. The repair keeps
the ownership in that Nagi-only backend: `strcmp` performs C byte comparison,
`atoi` uses the existing target-owned decimal parser, and `stderr` points to a
Nagi-owned descriptor-2 stream whose `fwrite` path calls the real
`nagi_posix_write_fd` facade. This does not import host libc or provide a
symbol-only success path; it completes a real target C runtime boundary.

Local standalone metadata compilation, `cargo check -p nagi-cli --tests`,
format, and whitespace checks pass. The host test binary remains unable to
link locally because this Windows environment lacks MSVC `link.exe`; that is
separate from target evidence. The next CI run must verify target archive
linkage, then continue through UEFI and real QEMU first-web-pixel acceptance.

## Remediation continuation (2026-09-21, condition/resolver/ioctl ABI)

Public snapshot CI run `35589309822` (head `670dbb8`) confirmed that the
Nagi-only relibc C runtime repair reached the next final link boundary. The
exact new diagnostics were undefined `pthread_cond_timedwait`,
`gai_strerror`, and `ioctl`.

The current repair keeps all three in Nagi-owned boundaries. Condition waits
use the existing Nagi mutex word and a sequence counter in the caller-owned
condition object; timed waits poll the guest `GuestClock`, release and
reacquire the mutex, and return the real `ETIMEDOUT` result at the guest
deadline. `gai_strerror` is a target-owned resolver diagnostic table because
the upstream relibc netdb module is excluded for `target_os = "nagi"`.
`ioctl` forwards through `nagi_posix_ioctl`, which validates descriptor shape
and returns `ENOTTY` for unsupported device-control requests without calling a
host ioctl. No host synchronization, resolver, device, or fake-success path is
introduced. UEFI and real QEMU first-web-pixel acceptance remain required.

The follow-up run `35591875406` (head `697dd08`) passed bootstrap, Mesa,
package, and kernel but stopped before linking with
`error[E0412]: cannot find type AtomicU32` in `user/nagi-posix/src/abi.rs`.
The implementation imported `AtomicUsize` but omitted `AtomicU32`; the
corrective import is now added. Target link and all later acceptance stages
remain unverified.

## Remediation continuation (2026-09-21, target socket/stat ABI)

Public snapshot CI run `35593932419` (head `085d912`) verified the condition,
resolver, and ioctl repair through target compilation and reached final
linking. The exact next undefined symbols were `accept`, `getsockopt`, and
`lstat`.

The target-owned Nagi POSIX ABI now exports all three through the pinned
relibc backend. `accept` validates the descriptor sign and returns the real
`ENOSYS` result because the current `nagi-net` service exposes the M17 client
TCP slice but no listener/accept service; it does not fabricate a connection
or descriptor. `getsockopt` reads the descriptor's actual TCP_NODELAY and
receive/send timeout state and returns it with the target socket ABI. `lstat`
reuses the existing guest VFS open/stat/close path, so metadata does not come
from the host filesystem. The repair preserves the Nagi-owned capability and
host-separation boundaries. Local formatting, CLI contract tests, clippy,
whitespace checks, and standalone relibc metadata compilation pass; the next
public run must verify the target link and then continue through UEFI and real
QEMU first-web-pixel acceptance.

## Remediation continuation (2026-09-21, target clock/math facade)

Public snapshot CI run `35597057716` (head `17bcc29`) passed the target
compile boundary and reached final linking. The exact undefined symbols were
`nagi_posix_lstat`, `gettimeofday`, and `pow`.

The target POSIX layer now exports `nagi_posix_lstat` as the strong Nagi VFS
facade and keeps `lstat` as its weak C wrapper. `gettimeofday` reads the real
guest realtime source through `GuestClock`; it does not consult the Windows
host clock. The target-owned relibc backend now provides `pow` and `powf`
with a bounded IEEE-aware logarithm/exponential implementation because the
upstream relibc math header module is excluded for `target_os = "nagi"`.
This is a real freestanding math implementation, not a host math-library
fallback or a constant symbol stub. Local formatting, CLI contract tests,
clippy, whitespace checks, and standalone relibc metadata compilation pass;
the next public run must verify target link, UEFI, and real QEMU first-web-
pixel acceptance.

## Remediation continuation (2026-09-21, stdio/terminal ABI)

Public snapshot CI run `35600567895` (head `bcc978a`) passed the target
compile boundary and reached final linking. The exact undefined symbols were
`isatty`, `strncmp`, and `snprintf`.

The Nagi POSIX layer now exports `nagi_posix_isatty`, returning terminal
status only for the real guest standard descriptors and setting the Nagi errno
facade for invalid or non-terminal descriptors. The target relibc backend
forwards `isatty`, implements bounded `strncmp`, and owns a real bounded C
formatter for `snprintf`/`vsnprintf`. The formatter consumes the C variadic
arguments required by the format string, supports the string, character,
integer, pointer, and floating-point conversions used by the target ABI,
handles width/precision, reports the untruncated length, and terminates a
bounded destination. It does not call host libc or return a fixed success
value. The M17 contract test now covers all three relibc symbols and the Nagi
terminal facade.

The standalone target-backend metadata compile, format, CLI check, clippy,
and whitespace checks pass. The focused host test binary still cannot link on
this Windows PC because MSVC `link.exe` is unavailable; that is a host review
limitation, not target evidence. The repair is not yet accepted until target
link, UEFI, and real QEMU first-web-pixel evidence are produced.

## Remediation continuation (2026-09-21, root VFS file-operation ABI)

Public snapshot CI run `35604881762` (head `0f2c05c`) passed the target
compile boundary and reached final linking. The exact undefined symbols were
`unlink`, `openat`, and `unlinkat`.

The target POSIX layer now connects these operations to the existing Nagi VFS
file table. `openat(AT_FDCWD, ...)` uses the established Nagi open/create/
truncate path, while `unlink` and `unlinkat` use the VFS's durable remove
operation. Nagi's current storage model is a bounded root-directory VFS, so
dirfd-relative `openat`/`unlinkat` resolution is explicitly rejected with
`ENOTSUP`; unsupported unlink flags are rejected with `EINVAL`. The relibc
backend forwards the real operations and does not call the host filesystem or
return a fixed success value. The target link, UEFI, and real QEMU first-web-
pixel gates remain required.

## Remediation continuation (2026-09-21, trigonometry/directory ABI)

Public snapshot CI run `35608468009` (head `04799f3`) passed the target
compile boundary and reached final linking. The exact undefined symbols were
`cosf`, `sinf`, and `fdopendir`.

The target relibc backend now owns freestanding range-reduced polynomial
implementations for `sin`/`cos` and `sinf`/`cosf`, keeping Servo/Mesa math off
the host library boundary. `fdopendir` is connected to the Nagi POSIX
directory boundary; because the current VFS exposes a bounded root file
model and does not yet expose directory file descriptors, a regular file is
rejected with the real `ENOTDIR` result and invalid descriptors preserve the
runtime errno mapping. No fabricated `DIR` object or host directory access is
introduced. Target link, UEFI, and real QEMU first-web-pixel evidence remain
required.

## Remediation continuation (2026-09-21, target memory/C++/Mesa link)

Public snapshot CI run `35610876131` (head `45ab39c`) passed the target
bootstrap, dependency boundary, Mesa, package, kernel, and compilation stages,
then failed at final target linking. The exact undefined diagnostics were
`__memcpy_chk`, `vtable for __cxxabiv1::__si_class_type_info`, and
`dri2_init_drawable`; UEFI and real QEMU were skipped.

The M17 repair stream now adds a bounds-checked `__memcpy_chk` to the
target-owned `libnagi` memory ABI, passes `-fno-exceptions` and `-fno-rtti` to
the freestanding Mesa C/C++ compilation boundary, and tracks
`0018-nagi-enable-dri2-frontend.patch`. The Mesa patch enables the actual DRI2
frontend source for Nagi's surfaceless static `libdri`, without enabling a DRM
device, host display, dynamic loader, or host runtime. These changes require a
fresh target CI link result; M17 remains `BLOCKED` until target linking, UEFI,
real QEMU, and real first-web-pixel evidence all pass.

## Remediation continuation (2026-09-22, target network/process ABI)

Public snapshot CI run `35615722163` (head `768405a`) passed the target
bootstrap, dependency boundary, Mesa, package, kernel, and compilation stages,
then failed at final target linking. The exact undefined diagnostics were
`__assert_fail`, `getaddrinfo`, and `fork`; UEFI and real QEMU were skipped.

The M17 repair stream now adds target-owned assertion failure termination,
`getaddrinfo`/`freeaddrinfo` using numeric IPv4 parsing or the real Nagi
DNS/POSIX resolver, and a truthful `fork` ABI that reports `ENOSYS` because
Nagi's process creation contract is spawn-oriented rather than fork-based.
No host resolver, host process creation, or fabricated success result is used.
These changes require a fresh target CI link result; M17 remains `BLOCKED`
until target linking, UEFI, real QEMU, and real first-web-pixel evidence all
pass.

## Remediation continuation (2026-09-22, target libc++/stdlib ABI)

Public snapshot CI run `35619764841` (head `f401b4a`) passed the target
bootstrap, dependency boundary, Mesa, package, kernel, and compilation stages,
then failed at final target linking. The exact undefined diagnostics were
`strcat`, `bsearch`, and
`std::__1::__libcpp_verbose_abort(char const*, ...)`; UEFI and real QEMU were
skipped.

The M17 repair stream now adds target-owned `strcat` and comparator-based
`bsearch` implementations to the relibc backend. It also maps libc++'s
freestanding verbose-abort ABI to Nagi's real process abort boundary, keeping
diagnostic failures terminating the guest rather than importing a host
libc++abi. These changes require a fresh target CI link result; M17 remains
`BLOCKED` until target linking, UEFI, real QEMU, and real first-web-pixel
evidence all pass.

## Remediation continuation (2026-09-22, libc++ ABI and process wait/exit)

Public snapshot CI run `35623171268` (head `91c5669`) passed the target
bootstrap, dependency boundary, Mesa, package, kernel, and compilation
stages, then failed at final target linking. The exact diagnostics were
`std::__1::__libcpp_verbose_abort(char const*, ...)`, `waitpid`, and `_exit`;
UEFI and real QEMU were skipped.

The next M17 repair corrects the length field in the pinned libc++ verbose
abort Itanium symbol and connects relibc's `_exit`, `exit`, and `waitpid` to
the Nagi POSIX process boundary. `waitpid(1, ..., 0)` joins the existing real
Nagi spawn slot and returns its encoded exit status; unsupported PIDs and
options fail with errno rather than fabricating process state. Exit calls use
the published Nagi process-exit syscall. No host libc, host process, or
synthetic rendering path is introduced. M17 remains `BLOCKED` until target
link, UEFI, real QEMU, and real first-web-pixel evidence pass.

## Remediation continuation (2026-09-22, descriptor and identity ABI)

Public snapshot CI run `35626833161` (head `d50f224`) passed the target
bootstrap, dependency boundary, Mesa, package, kernel, and compilation
stages, then failed at final target linking. The exact diagnostics were
`vtable for __cxxabiv1::__si_class_type_info`, `dup2`, and `setgid`; UEFI and
real QEMU were skipped.

The next M17 repair connects regular-file `dup2` to Nagi's user-space
descriptor table and exposes `setgid` through the capability-owned POSIX
boundary, returning `ENOSYS` because Nagi 0.1 has no mutable POSIX gid store.
Socket and pipe duplication remains explicitly unsupported until shared
descriptor ownership is implemented; it is not represented by a shallow
host-like copy. M17 remains `BLOCKED` until target link, UEFI, real QEMU, and
real first-web-pixel evidence pass.

## Remediation continuation (2026-09-22, duplicate POSIX symbol ownership)

Public snapshot CI run `35630408478` (head `4127b87`) passed the target
bootstrap, dependency boundary, Mesa, package, kernel, and compilation
stages, then failed at final target linking with a duplicate strong `dup2`
symbol. UEFI and real QEMU were skipped.

The target libc symbol remains owned by relibc, while `nagi-posix` now exposes
only the Nagi-owned `nagi_posix_dup2` facade and descriptor-table
implementation. This preserves one strong C ABI owner and prevents the
adapter from colliding with the relibc target backend. M17 remains `BLOCKED`
until target link, UEFI, real QEMU, and real first-web-pixel evidence pass.

## Remediation continuation (2026-09-22, freestanding C++ guard and C-locale ABI)

Public snapshot CI run `35632734832` (head `f0f4cad`) passed the target
bootstrap, dependency boundary, Mesa, package, kernel, and compilation
stages, then reached final target linking. The exact undefined diagnostics were
`__cxa_guard_acquire`, `std::__1::locale::classic()`, and
`std::__1::ctype<char>::id`; UEFI and real QEMU were skipped.

The next M17 repair adds the Itanium static-initialization guard protocol to
the Nagi-owned freestanding C++ runtime. Acquire uses an atomic owner bit and
waits for a concurrent initializer; release publishes initialization and abort
clears the owner bit for a real retry. It also provides the pinned libc++
classic C-locale identity and zero-initialized `ctype<char>::id` storage in
Nagi-owned target memory. No host locale database, host C++ runtime, or
rendering fallback is introduced. M17 remains `BLOCKED` until target linking,
UEFI, real QEMU, and real first-web-pixel evidence pass.

## Remediation continuation (2026-09-22, C++ runtime compile correction)

Public snapshot CI run `35636554538` (head `9fcd26c`) passed the target
bootstrap, dependency boundary, Mesa, package, kernel, and target compilation
stages, then failed inside `Build Nagi user init` while compiling the new
freestanding C++ runtime. The public annotation exposed only the custom build
command failure, so the source was compiled locally with the available LLVM
clang target configuration. That reproduced the concrete error: the
`alignas` attribute was placed after `extern "C"`; it also exposed a
C-linkage warning for the user-defined reference return.

The next M17 repair moves the locale-id declaration to valid freestanding C++
syntax and spells the classic-locale reference ABI as an equivalent pointer
return, removing the warning without changing the target calling convention.
The local `x86_64-unknown-none` compile now passes. M17 remains `BLOCKED`
until the target link, UEFI, real QEMU, and real first-web-pixel evidence pass.

## Remediation continuation (2026-09-22, target identity and root-directory ABI)

Public snapshot CI run `35639577267` (head `6cc0d23`) passed the target
bootstrap, dependency boundary, Mesa, package, kernel, and compilation
stages, then reached final target linking. The exact undefined diagnostics were
`setuid`, `chroot`, and `chdir`; UEFI and real QEMU were skipped.

The next M17 repair adds relibc-owned `setuid`, `chroot`, and `chdir` symbols
backed by Nagi POSIX adapters. Because M17's process starts in its single
capability-scoped root, `chdir("/")` and `chroot("/")` preserve that actual
namespace as no-ops; alternate namespaces fail closed with `ENOTSUP`, and
mutable POSIX uid changes fail closed with `ENOSYS` because Nagi identity is
capability-scoped rather than a mutable uid store. No host filesystem,
privilege escalation, or fabricated success for unsupported namespaces is
introduced. M17 remains `BLOCKED` until target linking, UEFI, real QEMU, and
real first-web-pixel evidence pass.

## Remediation continuation (2026-09-22, process-group and signal ABI)

Public snapshot CI run `35642763129` (head `5bbb66e`) passed the target
bootstrap, dependency boundary, Mesa, package, kernel, and compilation
stages, then reached final target linking. The exact undefined diagnostics were
`setpgid`, `setsid`, and `signal`; UEFI and real QEMU were skipped.

The next M17 repair adds relibc-owned process-group/session and classic signal
symbols backed by Nagi POSIX adapters. Nagi 0.1's current process model is
spawn-oriented and does not expose Unix process-group/session or signal
delivery APIs, so `setpgid` and `setsid` return `ENOSYS`, while `signal`
returns the ABI-correct `SIG_ERR` pointer and sets errno. This keeps the target
link honest without claiming signal delivery, importing a host signal API, or
fabricating process state. M17 remains `BLOCKED` until target linking, UEFI,
real QEMU, and real first-web-pixel evidence pass.

## Remediation continuation (2026-09-22, string/sort/trigonometry ABI)

Public snapshot CI run `35646089462` (head `3e49cce`) passed the target
bootstrap, dependency boundary, Mesa, package, kernel, and compilation
stages, then reached final target linking. The exact undefined diagnostics were
`memchr`, `qsort`, and `tan`; UEFI and real QEMU were skipped.

The next M17 repair adds target-owned byte search and deterministic in-place
sorting to the relibc backend, because the upstream target-selected string and
stdlib modules are not compiled for Nagi. It also adds `tan` using the existing
freestanding range-reduced sine/cosine implementation. No host allocator,
host libc, or symbol-only success path is introduced. M17 remains `BLOCKED`
until target linking, UEFI, real QEMU, and real first-web-pixel evidence pass.

## Remediation continuation (2026-09-22, target inverse-trigonometric ABI)

Public snapshot CI run `35649189787` (head `997714b`) passed target bootstrap,
dependency-boundary validation, Mesa, package, kernel, and compilation stages,
then failed at final target linking. The exact undefined symbols were `atanf`,
`asinf`, and `atan2f`; UEFI and real QEMU were skipped.

The next M17 repair adds real freestanding inverse-trigonometric math to the
Nagi relibc backend: range-reduced atan/atan2, bounded Newton square root for
asin, domain and NaN handling, and float/double C ABI entry points. It does
not import host libm or claim unsupported rendering behavior. M17 remains
`BLOCKED` until target linking, the UEFI loader, real QEMU, and the real Servo
first-web-pixel gate pass.

## Remediation continuation (2026-09-22, target C++ RTTI/process identity ABI)

Public snapshot CI run `35652406841` (head `a188f9a`) passed target bootstrap,
dependency-boundary validation, Mesa, package, kernel, and compilation stages,
then failed at final target linking. The exact undefined symbols were the
Itanium ABI vtables for `__cxxabiv1::__class_type_info` and
`__cxxabiv1::__si_class_type_info`, plus `getpid`; UEFI and real QEMU were
skipped.

The next M17 repair adds the required virtual slot order to the Nagi-owned
freestanding C++ runtime so the target links without importing libc++abi. It
also exposes the kernel-published root process identity through the Nagi POSIX
facade and relibc `getpid`. M17 remains `BLOCKED` until target linking, the
UEFI loader, real QEMU, and the real Servo first-web-pixel gate pass.

## Remediation continuation (2026-09-22, target exp/rwlock ABI)

Public snapshot CI run `35655720144` (head `e5fbab8`) passed target bootstrap,
dependency-boundary validation, Mesa, package, kernel, and compilation stages,
then failed at final target linking. The exact undefined symbols were `expf`,
`pthread_rwlock_init`, and `pthread_rwlock_rdlock`; UEFI and real QEMU were
skipped.

The next M17 repair adds freestanding exp/expf wrappers over the existing Nagi
range-reduced exponent core. It also implements the target's four-byte opaque
rwlock ABI with an atomic reader count and writer bit, including init, blocking
read acquisition, and the complementary write/unlock operations. It does not
call host libm or host pthreads. M17 remains `BLOCKED` until target linking,
the UEFI loader, real QEMU, and the real Servo first-web-pixel gate pass.

## Remediation continuation (2026-09-22, target hypot and GNU C++ ABI)

Public snapshot CI run `35658269625` (head `18a3442`) passed target bootstrap,
dependency-boundary validation, Mesa, package, kernel, and compilation stages,
then failed at final target linking. The exact undefined symbols were `hypotf`,
GNU `std::__throw_length_error(char const*)`, and GNU
`basic_string::_M_dispose()`; UEFI and real QEMU were skipped.

The next M17 repair adds scaled freestanding `hypot`/`hypotf` over Nagi's
bounded square-root implementation. It also supplies the concrete GNU C++ ABI
length-error entrypoint and an allocator-backed C++11 `basic_string` dispose
implementation. Local target clang output confirms the exact symbols, and the
dispose path releases heap-backed strings through Nagi's allocator while
leaving local-buffer strings intact. No host exception runtime, host
allocator, or rendering fallback is introduced. M17 remains `BLOCKED` until
target linking, the UEFI loader, real QEMU, and the real Servo first-web-pixel
gate pass.

## Remediation continuation (2026-09-22, environment and exec ABI)

Public snapshot CI run `35661181113` (head `93107b7`) passed target bootstrap,
dependency-boundary validation, Mesa, package, kernel, and compilation stages,
then failed at final target linking. The exact undefined symbols were GNU
`std::nothrow`, `environ`, and `execvp`; UEFI and real QEMU were skipped.

The next M17 repair adds a Nagi-owned GNU nothrow object, publishes the real
empty-start environment object used by Nagi user processes, and exposes
`execvp` as a fail-closed `ENOSYS` boundary because Nagi's process model is
spawn-oriented and has no Unix exec-in-place primitive. It does not route
execution through the host or claim a process replacement that did not occur.
M17 remains `BLOCKED` until target linking, the UEFI loader, real QEMU, and the
real Servo first-web-pixel gate pass.

## Exit criteria

Reopen M17 from this ADR after the guest rendering dependency is available.
Run the target build, focused adapter tests, and the QEMU acceptance wrapper.
Only then change the status to `PASS`; otherwise retain `BLOCKED` with updated
command output and the next concrete experiment.
