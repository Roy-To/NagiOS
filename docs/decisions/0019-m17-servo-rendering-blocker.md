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

## Target-link continuation after CI run #127 (2026-09-23)

Public CI run `35860649042` (`7fa87c9`) passed the pinned Servo bootstrap,
Nagi Mesa Softpipe archive, package, and kernel stages. The real target link
then exposed three missing Nagi-owned relibc entry points: `vfprintf`,
`regcomp`, and `regexec`. This is an implementation blocker inside the M17
vertical slice, not a host-toolchain or external-asset blocker. The repair
keeps the descriptor-backed `vfprintf` path and adds a bounded target-owned
POSIX regex implementation; no host libc or fake rendering path is used.
Target link, UEFI, and real QEMU first-web-pixel evidence remain pending until
the repaired target job succeeds.

## Target-link continuation after CI run #128 (2026-09-23)

Public CI run `35864684193` (`6f552bc`) reached the real target link after the
previous relibc repair. The remaining Nagi-owned formatting ABI was
`vasprintf`, `asprintf`, and `__vsnprintf_chk`. The next repair keeps the
descriptor/allocator boundary in Nagi relibc, uses `va_copy`-equivalent
`VaList::with_copy` for sized formatting, and honors the fortified object-size
bound. UEFI and real QEMU first-web-pixel evidence remain pending.

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

## Remediation continuation (2026-09-22, target integer and GNU container ABI)

Public snapshot CI run `35663893779` (head `1250b41`) passed target bootstrap,
dependency-boundary validation, Mesa, package, kernel, and compilation stages,
then failed at final target linking. The exact undefined symbols were `abs`,
`__dynamic_cast`, and GNU `_Rb_tree_increment(_Rb_tree_node_base*)`; UEFI and
real QEMU were skipped.

The next M17 repair adds target-owned integer `abs`, a bounded
single-inheritance `__dynamic_cast` implementation using the Itanium type-info
objects already owned by Nagi, and the actual GNU red-black-tree in-order
successor over its stable node prefix. Unsupported multiple/virtual RTTI
relationships fail closed rather than returning an invented object pointer.
No host C++ runtime, host container implementation, or rendering fallback is
introduced. M17 remains `BLOCKED` until target linking, the UEFI loader, real
QEMU, and the real Servo first-web-pixel gate pass.

## Remediation continuation (2026-09-22, target client socket ABI)

Public snapshot CI run `35666372443` (head `b1efeeb`) passed target bootstrap,
dependency-boundary validation, Mesa, package, kernel, and compilation stages,
then failed at final target linking. The exact undefined symbols were
`getpeername`, `bind`, and `listen`; UEFI and real QEMU were skipped.

The next M17 repair connects `getpeername` to the real peer address retained by
Nagi's smoltcp-backed client TCP descriptor. Nagi 0.1 does not yet expose a
server-listener service, so `bind` and `listen` return the real `ENOSYS`
boundary rather than claiming a fabricated listener or importing host sockets.
M17 remains `BLOCKED` until target linking, the UEFI loader, real QEMU, and the
real Servo first-web-pixel gate pass.

## Exit criteria

## Remediation continuation (2026-09-22, target root-VFS directory ABI)

Public snapshot CI run `35668983049` (head `e5925d9`) passed target bootstrap,
dependency-boundary validation, Mesa, package, kernel, and compilation stages,
then failed at final target linking. The exact undefined symbols were `rmdir`,
`mkdir`, and `opendir`; UEFI and real QEMU were skipped.

The next M17 repair adds target-owned `mkdir` and `rmdir` operations over the
existing Nagi root VFS and a bounded `opendir`/`readdir`/`closedir` adapter that
enumerates the real root directory snapshot. Non-root directory namespaces
remain explicitly rejected by the Nagi POSIX boundary. This does not import
host filesystem behavior or claim a synthetic directory. M17 remains
`BLOCKED` until target linking, the UEFI loader, real QEMU, and the real Servo
first-web-pixel gate pass.

Reopen M17 from this ADR after the guest rendering dependency is available.
Run the target build, focused adapter tests, and the QEMU acceptance wrapper.
Only then change the status to `PASS`; otherwise retain `BLOCKED` with updated
command output and the next concrete experiment.

## Remediation continuation (2026-09-22, target context and reentrant directory ABI)

Public snapshot CI run `35671405076` (head `c1395a0`) passed target bootstrap,
dependency-boundary validation, Mesa, package, kernel, and compilation stages,
then failed at final target linking. The exact undefined symbols were
`setjmp`, `longjmp`, and `readdir_r`; UEFI and real QEMU were skipped.

The next M17 repair adds the real x86-64 callee-saved register, stack, and
return-address context ABI for `setjmp`/`longjmp` in the Nagi-owned relibc
backend. It also adds the reentrant directory copy operation over the existing
root-VFS snapshot, with invalid pointers rejected at the Nagi POSIX boundary.
No host exception, signal, filesystem, or rendering implementation is used.
M17 remains `BLOCKED` until target linking, the UEFI loader, real QEMU, and the
real Servo first-web-pixel gate pass.

## Validation continuation (2026-09-22, Mesa archive retry)

Public snapshot CI run `35673531417` (head `51deb09`) failed at the pinned Mesa
Softpipe archive step with exit code 2 and no compiler annotation; the target
build and all later M17 gates were skipped. The identical pinned Mesa path
passed in run `35671405076`, so this run does not establish a new Nagi source
blocker or acceptance result. Re-run the unchanged implementation path before
classifying the target ABI repair; M17 remains `BLOCKED` until target linking,
UEFI, real QEMU, and the real Servo first-web-pixel gate pass.

## Validation continuation (2026-09-22, assembler syntax correction)

Public snapshot CI run `35673934729` (head `a2be48a`) failed at the same pinned
Mesa Softpipe archive step before target linking. Local object emission then
reproduced the hidden compiler failure: the Nagi relibc `global_asm!` context
switch used AT&T syntax while Rust/LLVM expected Intel syntax. The assembly is
now corrected, and local object emission plus symbol inspection succeeds. This
was an implementation defect, not M17 acceptance evidence; rerun the complete
target path. M17 remains `BLOCKED` until target linking, UEFI, real QEMU, and
the real Servo first-web-pixel gate pass.

## Remediation continuation (2026-09-22, Mesa archive roots and C++/math ABI)

Public CI run `35674424033` (#78, head `73b20bd`) passed the target Mesa
archive and reached final linking. The exact diagnostics were the
`__cxxabiv1::__vmi_class_type_info` vtable and Mesa state-tracker roots
`_mesa_glthread_finish` and `st_context_flush`. The next repair supplied the
Nagi-owned freestanding RTTI object and selective raw linker roots; it did not
force the entire Mesa archive into the image.

Run `35676523724` (#79, head `af5533f`) confirmed the roots but rejected
`-Wl,--start-group`/`--end-group` because Cargo passes these values directly to
rust-lld. Run `35678180409` (#80, head `798e527`) then reached the next real
target ABI boundary: `__cxa_atexit`, `tanf`, and `log2`. Run
`35680421808` (#82, head `039166e`) reproduced those exact final-link
diagnostics after the selective Mesa archive correction.

The next repair adds a bounded Nagi C++ destructor registry wired into the real
POSIX exit boundary, plus target-owned `tanf` and `log2` implementations over
the existing freestanding math core. No host libc++abi, host libm, or symbol-
only success path is introduced. M17 remains `BLOCKED` until target linking,
UEFI, real QEMU, and real first-web-pixel evidence pass.

## Remediation continuation (2026-09-22, socket, directory, and thread ABI)

Public CI run `35682596273` (#83, head `0588c8b`) passed target bootstrap,
dependency validation, Mesa Softpipe, package, kernel, and compilation stages,
then reached final target linking. The exact undefined symbols were
`getsockname`, `dirfd`, and `pthread_detach`; UEFI and real QEMU were skipped.

The next repair adds real smoltcp local-endpoint reporting for `getsockname`,
represents the only supported root directory namespace through its actual
`AT_FDCWD` identity for `dirfd`, and implements bounded detached-thread state
with deferred stack reclamation after a replacement child is accepted. It does
not use host sockets/filesystem/threads or return synthetic success for an
unsupported target operation. M17 remains `BLOCKED` until target linking,
UEFI, real QEMU, and real first-web-pixel evidence pass.

## Remediation continuation (2026-09-22, target freestanding math ABI)

Public snapshot CI run `35686392969` (#85, head `de1796d`) passed target
bootstrap, dependency-boundary validation, Mesa Softpipe, package, kernel, and
target compilation stages, then failed at final target linking. The exact
undefined symbols were `log`, `tanhf`, and `logf`; UEFI and real QEMU were
skipped.

The next M17 repair adds target-owned freestanding `log` and `logf` entrypoints
over Nagi's existing mantissa/exponent logarithm reduction, and stable `tanh`
and `tanhf` entrypoints over the existing Nagi exponential implementation.
The implementation remains independent of host libm and does not turn symbol
presence into a fake success path. The source contract records all four
entrypoints. M17 remains `BLOCKED` until target linking, the UEFI loader, real
QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-22, GNU string and unwind ABI)

Public snapshot CI run `35688435791` (#86, head `496550a`) passed target
bootstrap, dependency-boundary validation, Mesa Softpipe, package, kernel, and
target compilation stages, then failed at final target linking. The exact
undefined symbols were `_Unwind_Resume` and GNU C++11 `basic_string` methods
`_M_append` and `find`; UEFI and real QEMU were skipped.

The next M17 repair adds concrete GNU C++11 string append/find operations over
the existing Nagi allocator-backed string layout. It also provides the
exception-disabled target's `_Unwind_Resume` boundary, which terminates through
the real Nagi abort path if an incompatible object enters an exception resume
path. No host C++ standard library, host unwinder, or symbol-only success path
is introduced. M17 remains `BLOCKED` until target linking, the UEFI loader,
real QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-22, substring and FILE ABI)

Public snapshot CI run `35692225348` (#88, head `5625a18`) passed target
bootstrap, dependency-boundary validation, Mesa Softpipe, package, kernel, and
target compilation stages, then failed at final target linking. The exact
undefined symbols were `fprintf`, `strstr`, and `fopen`; UEFI and real QEMU
were skipped.

The next M17 repair adds target-owned `strstr` over guest memory. It also maps
`fopen` modes to Nagi POSIX/VFS descriptors and sends bounded `fprintf` output
through the existing Nagi formatter and FILE write boundary. The implementation
does not import host stdio or claim output without a real Nagi descriptor. M17
remains `BLOCKED` until target linking, the UEFI loader, real QEMU, and real
Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, C++ allocation and Mesa glthread link ABI)

Public snapshot CI run `35797964401` (#94, head `9f14d21`) passed target
bootstrap, dependency-boundary validation, Mesa Softpipe, package, and kernel
stages, then failed at final target linking. The exact undefined symbols were
`std::__throw_bad_alloc()`, `__cxa_end_catch`, and `_mesa_glthread_finish`;
UEFI and real QEMU were skipped.

The next M17 repair adds fail-closed target-owned C++ ABI entrypoints for bad
allocation and exception-end paths. It also replaces the direct archive root
for `_mesa_glthread_finish` with a Nagi-owned link-only data anchor that points
to the real pinned Mesa implementation, so rust-lld extracts the actual
glthread object without providing a fake rendering function. No host
libc++abi, host graphics implementation, or synthetic pixel path is added.
M17 remains `BLOCKED` until target linking, the UEFI loader, real QEMU, and
real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, exceptions-disabled C++ ABI)

Public snapshot CI run `35793927424` (#92, head `b53d990`) passed target
bootstrap, dependency-boundary validation, Mesa Softpipe, package, and kernel
stages, then failed at final target linking. The exact undefined symbols were
`std::__throw_bad_array_new_length()`, `__cxa_begin_catch`, and
`__cxa_rethrow`; UEFI and real QEMU were skipped.

The next M17 repair adds target-owned fail-closed entrypoints for these
exceptions-disabled C++ ABI paths. If reached, they terminate through Nagi's
real abort boundary instead of importing host libc++abi or an unwinder, and
M17 remains `BLOCKED` until target linking, the UEFI loader, real QEMU, and
real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, errno, scanning, and duplication ABI)

Public snapshot CI run `35791007289` (#91, head `1723c49`) passed target
bootstrap, dependency-boundary validation, Mesa Softpipe, package, and kernel
stages, then failed at final target linking. The exact undefined symbols were
`__errno_location`, `sscanf`, and `strdup`; UEFI and real QEMU were skipped.

The next M17 repair exposes the existing Nagi errno slot, implements bounded
guest-memory `sscanf` support for the integer/string formats used by pinned
Mesa, and duplicates strings through the Nagi allocator. No host libc parser,
host errno storage, or host allocator is introduced. M17 remains `BLOCKED`
until target linking, the UEFI loader, real QEMU, and real Servo first-web-pixel
evidence pass.

## Remediation continuation (2026-09-23, Meson core target layout)

Public snapshot CI run `35809154991` (#100, head `409f15d`) passed Servo
bootstrap but failed during the pinned Mesa Softpipe archive build while
selecting the explicit Mesa core target. Target link, UEFI, and real QEMU were
not reached.

The next M17 repair accepts every Ninja target name containing `libmesa.a`,
including Meson `.p` output-layout forms, before repeating the target-owned
archive extraction. No host archive, whole-archive shortcut, or fake Mesa
rendering function is introduced. M17 remains `BLOCKED` until target linking,
the UEFI loader, real QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-22, base-2 math and descriptor stdio)

Public snapshot CI run `35690360685` (#87, head `5f90040`) passed target
bootstrap, dependency-boundary validation, Mesa Softpipe, package, kernel, and
target compilation stages, then failed at final target linking. The exact
undefined symbols were `exp2f`, `log2f`, and `fread`; UEFI and real QEMU were
skipped.

The next M17 repair adds target-owned `exp2`/`exp2f` and `log2f` over the
existing freestanding Nagi exponent/logarithm core. It also adds `fread` for
Nagi descriptor-backed FILE streams by forwarding reads to the real
`nagi_posix_read` boundary; Nagi memory output streams remain explicitly
non-readable and return `EBADF`. No host stdio, host filesystem, or synthetic
read result is introduced. M17 remains `BLOCKED` until target linking, the
UEFI loader, real QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, static symbol, once, and diagnostic ABI)

Public snapshot CI run `35787930674` (#90, head `04a47e6`) passed target
bootstrap, dependency-boundary validation, Mesa Softpipe, package, and kernel
stages, then failed at final target linking. The exact undefined symbols were
`dlsym`, `pthread_once`, and `perror`; UEFI and real QEMU were skipped.

The next M17 repair adds a fail-closed `dlsym` because the Nagi user target is
statically linked and has no dynamic loader namespace, a four-byte guest
atomic `pthread_once`, and a `perror` implementation that writes diagnostics
through Nagi descriptor 2. No host dynamic loader, host pthread, or host
stderr is introduced. M17 remains `BLOCKED` until target linking, the UEFI
loader, real QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, FILE cursor and bounded string ABI)

Public snapshot CI run `35694703468` (#89, head `41de72a`) passed target
bootstrap, dependency-boundary validation, Mesa Softpipe, package, kernel, and
target compilation stages, then failed at final target linking. The exact
undefined symbols were `fseek`, `ftell`, and `strncpy`; UEFI and real QEMU
were skipped.

The next M17 repair adds descriptor-backed `fseek` and `ftell` through
`nagi_posix_lseek`, plus POSIX bounded `strncpy` over guest memory. The
implementation does not import host stdio or host string routines, and M17
remains `BLOCKED` until target linking, the UEFI loader, real QEMU, and real
Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, pure-virtual and POSIX UID ABI)

Public snapshot CI run `35795642028` (#93, head `43ee78e`) passed target
bootstrap, dependency-boundary validation, Mesa Softpipe, package, and kernel
stages, then failed at final target linking. The exact undefined symbols were
`__cxa_pure_virtual`, `geteuid`, and `getuid`; UEFI and real QEMU were skipped.

The next M17 repair adds the target-owned Itanium pure-virtual entrypoint, which
terminates through Nagi's real abort boundary if an invalid virtual dispatch is
reached. It also exposes the initial capability-scoped root as the explicit
POSIX compatibility uid 0 view through nagi-posix and relibc; authorization
continues to use capabilities and does not derive authority from the numeric
uid. No host libc++abi, host identity, or host filesystem is introduced. M17
remains `BLOCKED` until target linking, the UEFI loader, real QEMU, and real
Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, Mesa glthread extraction and POSIX output ABI)

Public snapshot CI run `35801744073` (#95, head `d9db493`) passed target
bootstrap, dependency-boundary validation, Mesa Softpipe, package, and kernel
stages, then failed at final target linking. The exact undefined symbols were
`_mesa_glthread_finish`, `printf`, and `getegid`; UEFI and real QEMU were
skipped.

The next M17 repair locates the real pinned Mesa object defining
`_mesa_glthread_finish` with `llvm-nm`, places that object in a dedicated
target archive, and links it before the aggregate archive. It also implements
`printf` through Nagi stdout descriptor 1 and maps the capability-scoped root
to the explicit POSIX gid 0 compatibility view. No Mesa rendering stub, host
stdout, host identity, or host filesystem is introduced. M17 remains
`BLOCKED` until target linking, the UEFI loader, real QEMU, and real Servo
first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, string and ctype ABI)

Public snapshot CI run `35833047798` (#115, head `2a6c59e`) passed the pinned
Servo bootstrap, dependency boundary, Mesa Softpipe archive, package, kernel,
and target compilation stages, including the prior ctype/timezone repair. It
reached final target linking with the remaining undefined symbols `strcasecmp`,
`isalnum`, and `strcspn`; UEFI and real QEMU were skipped.

The next M17 repair adds locale-independent ASCII `strcasecmp` and `isalnum`,
plus guest-memory `strcspn`. No host locale, host string routines, or host
filesystem is imported. M17 remains `BLOCKED` until target linking, the UEFI
loader, real QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, generated Mesa archive naming)

Public snapshot CI run `35805855427` (#98, head `f004387`) passed Servo
bootstrap but failed during the pinned Mesa Softpipe archive build because the
generated output did not contain the fixed `libmesa.a` filename. Target link,
UEFI, and real QEMU were not reached.

The next M17 repair removes that filename assumption and searches every
generated target archive with `llvm-nm --defined-only` for the real
`_mesa_glthread_finish` definition before extracting its exact member. No host
archive, whole-archive shortcut, or fake Mesa rendering function is introduced.
M17 remains `BLOCKED` until target linking, the UEFI loader, real QEMU, and real
Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, explicit Mesa core target build)

Public snapshot CI run `35806844547` (#99, head `61f9247`) passed Servo
bootstrap but failed during the pinned Mesa Softpipe archive build because the
default generated archive set contained no definition of
`_mesa_glthread_finish`; target link, UEFI, and real QEMU were not reached.

The next M17 repair discovers the `libmesa.a` target from Ninja's target graph
and builds it explicitly before scanning the generated archives. This keeps
the real pinned Mesa glthread implementation in the target-owned archive set
without whole-archive extraction or a rendering stub. M17 remains `BLOCKED`
until target linking, the UEFI loader, real QEMU, and real Servo first-web-pixel
evidence pass.

## Remediation continuation (2026-09-23, Mesa archive-member discovery)

Public snapshot CI run `35804263561` (#96, head `96d830d`) passed Servo
bootstrap but failed during the pinned Mesa Softpipe archive build while
discovering the real `_mesa_glthread_finish` object. Target link, UEFI, and
real QEMU were not reached.

The next M17 repair keeps the extraction target-owned and reproducible: search
the pinned Mesa archives with `llvm-nm`, identify the exact member, extract it
with the pinned LLVM archiver, and place that real object in the dedicated Nagi
roots archive before the aggregate archive. No whole-archive shortcut or fake
Mesa rendering function is introduced. M17 remains `BLOCKED` until target
linking, the UEFI loader, real QEMU, and real Servo first-web-pixel evidence
pass.

## Remediation continuation (2026-09-23, Ninja archive target matching)

Public snapshot CI run `35809747498` (#101, head `4c4c92e`) passed Servo
bootstrap but still failed during the pinned Mesa Softpipe archive build: the
target matcher could select a generated object target below Meson's
`libmesa.a.p` directory, so the real `libmesa.a` archive was never built by
the explicit command. The archive scan consequently found no definition of
`_mesa_glthread_finish`. The next repair matches only a Ninja target whose
final path component is `libmesa.a`; the existing symbol-aware extraction then
operates on the real generated archive. No host archive, whole-archive
shortcut, or fake Mesa rendering function is introduced. M17 remains
`BLOCKED` until target linking, the UEFI loader, real QEMU, and real Servo
first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, direct Mesa core archive selection)

Public snapshot CI run `35805304582` (#97, head `934f230`) passed Servo
bootstrap but failed again during the pinned Mesa Softpipe archive build while
discovering the real `_mesa_glthread_finish` member. Target link, UEFI, and
real QEMU were not reached.

The next M17 repair selects the Meson-produced `libmesa.a` directly, extracts
the exact member with the pinned LLVM archiver, and emits explicit annotations
for missing archive/member state. Malformed archive members are skipped only
for discovery; no rendering implementation is bypassed. M17 remains
`BLOCKED` until target linking, the UEFI loader, real QEMU, and real Servo
first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, Mesa generated SPIR-V header)

Public snapshot CI run `35813251531` (#104, head `3f3bcde`) reached the real
`src/mesa/libmesa.a` compilation and exposed a Nagi Mesa dependency-graph
defect: `glspirv.c` could not find generated
`compiler/spirv/spirv_info.h`. The next repair adds that generated header as
a source of the existing `idep_vtn` dependency through numbered Mesa patch
`0019`. It does not add a host header or replace SPIR-V compilation. M17
remains `BLOCKED` until target linking, the UEFI loader, real QEMU, and real
Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, Mesa core compiler diagnostics)

Public snapshot CI run `35812210395` (#103, head `e6cbde1`) selected the real
`src/mesa/libmesa.a` target and entered its 256-object compile, but the Mesa
core target failed before archive creation. The public annotation retained
only the warning tail and `ninja: build stopped`, so the next repair
prioritizes compiler `error:` and `fatal error:` lines in the bounded
annotation. This preserves the real pinned Mesa build and does not replace
the core archive or rendering path. M17 remains `BLOCKED` until target
linking, the UEFI loader, real QEMU, and real Servo first-web-pixel evidence
pass.

## Remediation continuation (2026-09-23, Mesa core target diagnostics)

Public snapshot CI run `35810511071` (#102, head `4694c6c`) passed Servo
bootstrap but failed in the Mesa Softpipe step after the stricter
`libmesa.a` target matching was applied. The public job summary exposed only
`Process completed with exit code 1`, so it did not yet distinguish a real
Mesa core compilation failure from a Ninja target-resolution failure. The
next repair preserves the real target build and emits the selected target plus
the captured Ninja stderr tail as a bounded CI annotation. M17 remains
`BLOCKED` until target linking, the UEFI loader, real QEMU, and real Servo
first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, hidden Mesa symbol scan)

Public snapshot CI run `35813827384` (#105, head `9bb8bf0`) passed the
generated SPIR-V header repair and produced the real `libmesa.a` archive, but
the archive scan did not find `_mesa_glthread_finish`; the candidate list
included `libmesa.a`. The scan used `llvm-nm -g`, which can exclude
hidden-visibility Mesa symbols. The next repair scans all defined symbols
while preserving exact member extraction. No fake rendering function or host
archive is introduced. M17 remains `BLOCKED` until target linking, the UEFI
loader, real QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, Mesa object-level symbol scan)

Public snapshot CI run `35814387308` (#106, head `61556cd`) produced the real
`libmesa.a`, but neither archive-level `llvm-nm` nor archive-member extraction
exposed `_mesa_glthread_finish`. The next repair preserves archive discovery
and additionally scans the `.o` files emitted by that same Meson target,
selecting only the real defining object for the dedicated roots archive. No
fake rendering function or whole-archive shortcut is introduced. M17 remains
`BLOCKED` until target linking, the UEFI loader, real QEMU, and real Servo
first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, Gallium archive and lrintf ABI)

Public snapshot CI run `35815283540` (#107, head `54ed6d3`) passed Servo
bootstrap and the pinned Mesa Softpipe archive, then reached final Nagi
user-init target linking. The remaining undefined symbols were `lrintf`,
`u_surface_default_template`, and `pp_init`. The latter two are real Mesa
Gallium auxiliary objects whose `libgallium.a` target was not part of the
default Nagi graph; `lrintf` was a missing target-owned relibc C ABI export.
The next repair explicitly builds `libgallium.a` and adds target-owned
`lrintf`. M17 remains `BLOCKED` until target linking, the UEFI loader, real
QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, GLSL archive and math predicates)

Public snapshot CI run `35818125262` (#108, head `228551c`) passed Mesa
Softpipe and the prior `libgallium.a`/`lrintf` repair, then reached target
linking with the remaining undefined symbols
`link_util_parse_program_resource_name`, `isnan`, and `__isnanf`. The first is
from Mesa's real GLSL linker archive, whose `libglsl.a` target was not yet
explicitly built; the latter two are missing target math predicates. The next
repair explicitly builds `libglsl.a` and adds target-owned `isnan`/`__isnanf`.
M17 remains `BLOCKED` until target linking, the UEFI loader, real QEMU, and
real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, rounding and sprintf ABI)

Public snapshot CI run `35820755188` (#109, head `6296567`) passed the Mesa
GLSL archive and math-predicate repairs, then reached target linking with the
remaining undefined symbols `lroundf`, `llround`, and `sprintf`. The next
repair adds target-owned nearest-away-from-zero rounding and an
unbounded-format C ABI entrypoint over Nagi's existing formatter. No host
libm or host stdio is introduced. M17 remains `BLOCKED` until target linking,
the UEFI loader, real QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, target stdio stream ABI)

Public snapshot CI run `35823065041` (#110, head `9073c44`) passed the pinned
Servo bootstrap, dependency boundary, Mesa Softpipe archive, package, kernel,
and target compilation stages, then reached final target linking. The exact
remaining undefined symbols were `feof`, `fgets`, and `stdout`; UEFI and real
QEMU were not reached.

The next M17 repair completes the target-owned descriptor-backed stdio slice:
`stdout` now points to a real Nagi descriptor-1 `FILE` object, `fgets` reads
line bytes through the Nagi POSIX/VFS descriptor boundary, and `feof` reports
the stream's recorded end-of-file state. Read errors and EOF are tracked on
the Nagi stream object; no host stdio, host filesystem, or synthetic stream is
introduced. M17 remains `BLOCKED` until target linking, the UEFI loader, real
QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, VFS access and unsupported IPC ABI)

Public snapshot CI run `35827739822` (#113, head `240e501`) passed the pinned
Servo bootstrap, dependency boundary, Mesa Softpipe archive, package, kernel,
and target compilation stages, including the prior numeric/stdout repair. It
reached final target linking with the remaining undefined symbols `access`,
`setvbuf`, and `shmget`; UEFI and real QEMU were skipped.

The next M17 repair implements `access` through Nagi's real VFS open/close
boundary, exposes the target's explicit unbuffered `setvbuf` contract, and
adds a truthful fail-closed `shmget` for optional SysV/X11/DRI objects that
are outside the M17 surfaceless Softpipe path. No host filesystem, host stdio
buffer, shared-memory handle, or synthetic success result is introduced. M17
remains `BLOCKED` until target linking, the UEFI loader, real QEMU, and real
Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, numeric and stdout ABI)

Public snapshot CI run `35825148240` (#112, head `c58ecda`) passed the pinned
Servo bootstrap, dependency boundary, Mesa Softpipe archive, package, kernel,
and target compilation stages. It also passed the prior `feof`/`fgets`/`stdout`
repair and reached final target linking, where the exact remaining undefined
symbols were `lround`, `atof`, and `puts`; UEFI and real QEMU were skipped.

The next M17 repair adds target-owned `lround` over the existing Nagi rounding
core, `atof` over the real target `strtod` parser, and `puts` over the real
descriptor-1 stdout stream. No host libm, host stdio, or synthetic output is
introduced. M17 remains `BLOCKED` until target linking, the UEFI loader, real
QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, ctype and UTC timezone ABI)

Public snapshot CI run `35830580426` (#114, head `acca2e9`) passed the pinned
Servo bootstrap, dependency boundary, Mesa Softpipe archive, package, kernel,
and target compilation stages, including the prior VFS/stdio/IPC repair. It
reached final target linking with the remaining undefined symbols `isdigit`,
`tzset`, and `timezone`; UEFI and real QEMU were skipped.

The next M17 repair adds locale-independent target ctype `isdigit` and the
Nagi UTC timezone contract through `tzset` and the POSIX `timezone` global.
No host locale table or host timezone database is imported. M17 remains
`BLOCKED` until target linking, the UEFI loader, real QEMU, and real Servo
first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, unsupported SysV shared-memory ABI)

Public snapshot CI run `35835203783` (#116, head `bea57d7`) passed the pinned
Servo bootstrap, dependency boundary, Mesa Softpipe archive, package, kernel,
and target compilation stages. It reached final target linking with the exact
remaining undefined symbols `shmat`, `shmctl`, and `shmdt`; UEFI and real QEMU
were skipped.

The next M17 repair adds target-owned fail-closed entries for those SysV
shared-memory symbols. Each entry returns `ENOSYS` and never returns a host
pointer or claims a guest mapping that Nagi 0.1 does not provide. This keeps
the optional X11/DRI shared-memory ABI explicit while preserving the real
Servo-to-Nagi surfaceless Softpipe boundary. M17 remains `BLOCKED` until
target linking, the UEFI loader, real QEMU, and real Servo first-web-pixel
evidence pass.

## Remediation continuation (2026-09-23, target time, errno, and random ABI)

Public snapshot CI run `35838340415` (#117, head `32c1313`) passed the pinned
Servo bootstrap, dependency boundary, Mesa Softpipe archive, package, kernel,
and target compilation stages. It reached final target linking with the exact
remaining undefined symbols `strerror`, `time`, and `srand`; UEFI and real
QEMU were skipped.

The next M17 repair adds Nagi-owned errno text, forwards `time` to the real
guest `clock_gettime(CLOCK_REALTIME)` ABI, and supplies target-local seeded
`rand`/`srand` state. These paths do not import a host clock, host libc error
table, or host random source. M17 remains `BLOCKED` until target linking, the
UEFI loader, real QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, FILE, ctype, and VFS sync ABI)

Public snapshot CI run `35841735587` (#118, head `4ebf10d`) passed the pinned
Servo bootstrap, dependency boundary, Mesa Softpipe archive, package, kernel,
and target compilation stages. It reached final target linking with the exact
remaining undefined symbols `fputs`, `isspace`, and `sync`; UEFI and real
QEMU were skipped.

The next M17 repair adds `fputs` over the target-owned descriptor or guest
memory FILE boundary, locale-independent ASCII `isspace`, and the Nagi VFS
write-barrier contract for `sync`. Nagi descriptor writes are committed by
the service boundary before returning, so no host stdio or host filesystem
flush is substituted. M17 remains `BLOCKED` until target linking, the UEFI
loader, real QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, Mesa static loader/winsys targets)

Public snapshot CI run `35844150510` (#119, head `cbb114e`) passed the pinned
Servo bootstrap, dependency boundary, Mesa Softpipe archive, package, kernel,
and target compilation stages. It reached final target linking with the exact
remaining undefined symbols `fputc`, `sw_screen_create_vk`, and
`null_sw_create`; UEFI and real QEMU were skipped.

The next M17 repair adds target FILE `fputc` and explicitly builds Mesa's
pinned `libpipe_loader_static.a` and `libws_null.a` targets. Both upstream
targets are `build_by_default=false`; selecting and aggregating those real
objects preserves the Servo-to-Nagi Softpipe path without a fake renderer or
host fallback. M17 remains `BLOCKED` until target linking, the UEFI loader,
real QEMU, and real Servo first-web-pixel evidence pass.

## Remediation continuation (2026-09-23, Mesa archive extraction and `strspn`)

Public snapshot CI run `35851514326` (#120, head `87cc024`) passed the pinned
Servo bootstrap, dependency boundary, Mesa Softpipe archive, package, kernel,
and target compilation stages. It reached final target linking with
`sw_screen_create_vk`, `wrapper_sw_winsys_wrap_pipe_screen`, `null_sw_create`,
and `strspn` still unresolved. Building the real `build_by_default=false`
Mesa targets was insufficient because the single aggregate archive scan did
not extract providers that occur after their users. The next repair seeds
those real Softpipe loader/winsys symbols through the Nagi target link and
adds guest-memory `strspn` to the Nagi relibc ABI. UEFI and real QEMU
first-web-pixel acceptance were not reached. M17 remains `BLOCKED`; M18
remains `NOT STARTED`.

## Remediation continuation (2026-09-23, Mesa helper target registration)

Public CI run `35858894366` (#126, head `4d90f2b`) passed Servo bootstrap and
then failed in the Mesa Softpipe step because the Meson graph had no
`libpipe_loader_nagi_roots.a` output target. The new helper definition was
correct, but `src/gallium/targets/pipe-loader` is normally configured only
for clover/tests, both disabled by the M17 configuration. The next repair
adds `with_platform_nagi` to that existing subdirectory condition, preserving
the pinned Mesa source and patch boundary. Target build, UEFI, and real QEMU
first-web-pixel acceptance were not reached. M17 remains `BLOCKED`; M18
remains `NOT STARTED`.

## Remediation continuation (2026-09-23, Mesa helper header scope)

The latest target CI run for commit `208387f` registered the Nagi helper
archive but failed compiling its real Mesa `sw_helper.h` source. Clang
reported conflicting `pipe_screen_config` types because the new translation
unit did not include Mesa's defining `pipe/p_screen.h` before the helper
header, causing C prototype-scope tags. The next repair adds that standard
Mesa header before `sw_helper.h`; no rendering or ABI stub is introduced.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

## Remediation continuation (2026-09-23, Servo bootstrap diagnostics)

Public CI run `35857472389` (#122, head `63bb9b8`) failed during the pinned
Servo bootstrap before dependency-boundary, Mesa, target build, UEFI, or real
QEMU acceptance. The public check exposed only exit code 4, so no source or
linker conclusion is drawn from this run. The next repair preserves the
failure in `out/logs/m17-bootstrap.log` and emits a bounded first-error
annotation, allowing the exact pinned-source, patch-order, fingerprint, or
Servo Cargo-fetch failure to be corrected from evidence. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

## Remediation continuation (2026-09-23, Mesa patch hunk integrity)

Public CI run `35858275718` (#123, head `68f07cb`) confirmed the bootstrap
diagnostic: the new Mesa static-helper patch was rejected as a corrupt patch
at line 70 because its added hunk counts did not match the actual additions.
No Mesa, target build, UEFI, or real QEMU acceptance ran. The patch hunk
counts are now corrected and the added hunk was checked against the generated
Mesa source without altering that checkout. M17 remains `BLOCKED`; M18
remains `NOT STARTED`.

## Remediation continuation (2026-09-23, static helper roots and `strndup`)

Public CI run `35854076101` (#121, head `3d0286b`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive, package, kernel, and target
compilation. The link-root repair resolved `null_sw_create` and `strspn`, but
the final target link still reported `sw_screen_create_vk`,
`wrapper_sw_winsys_wrap_pipe_screen`, and `strndup`. UEFI and real QEMU
first-web-pixel acceptance were skipped. The next repair adds a pinned Mesa
Nagi static helper target that compiles the real `sw_helper.h` Softpipe
implementation, explicitly materializes upstream `libwsw.a`, and adds
target-owned guest allocator `strndup`. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

## Target-link continuation after CI run #129 (2026-09-23)

Public CI run `35868771255` (`cfbeb5c`) passed the repaired formatting ABI and
again reached the real target link. The remaining Nagi-owned symbols were
`atexit`, `ldexp`, and `__isfinite`. The next repair provides target-owned
exit-handler storage and freestanding IEEE math/predicate entry points; it
does not link a host CRT or libm. UEFI and real QEMU first-web-pixel evidence
remain pending.

## Target-link continuation after CI run #130 (2026-09-23)

Public CI run `35871531770` (`4c726ce`) passed the repaired exit/math ABI and
again reached the real target link. The remaining target-owned C++/compiler
runtime symbols were `std::_Rb_tree_insert_and_rebalance`,
`std::_Rb_tree_decrement`, and `__popcountdi2`. The next repair adds real GNU
red-black tree insertion/predecessor operations and a target popcount to the
Nagi-owned C++ runtime; it does not import host libstdc++ or compiler-rt.
UEFI and real QEMU first-web-pixel evidence remain pending.

## Target-link continuation after CI run #134 (2026-09-24)

Public CI run `35887795498` (#134, head `4665387`) resolved `strnlen`, `div`,
and GNU basic_string `_M_replace`, then reached the next real target-link set:
`syslog`, `openlog`, and
`std::__detail::_Prime_rehash_policy::_M_need_rehash(...)`. UEFI and real
QEMU first-web-pixel acceptance were skipped. The next repair adds
descriptor-backed target syslog/openlog handling and a Nagi-owned GNU prime
rehash policy ABI; it does not import a host syslog daemon, host libc, or host
C++ runtime. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

## Target-link continuation after CI run #137 (2026-09-24)

Public CI run `35899807167` (#137, head `7e1cd53`) resolved `fabsl`, GNU
`__throw_out_of_range_fmt`, and basic_string `_M_replace_aux(...)`, then
reached the next real target-link set:
`JS::NewArrayBufferWithContents(JSContext*, unsigned long,
std::unique_ptr<void, JS::FreePolicy>)`. UEFI and real QEMU first-web-pixel
acceptance were skipped. The next repair adds target-only MozJS patch `0014`,
which restores the real UniquePtr ownership wrapper by delegating to the
existing four-argument ArrayBuffer API; it does not create a synthetic JS
object or import a host runtime. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

## Target-link continuation after CI run #138 (2026-09-24)

Public CI run `35904323947` (#138, head `7bc9e6a`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. The real target user-init link still reported MozJS
`JS::NewArrayBufferWithContents(JSContext*, unsigned long,
std::unique_ptr<void, JS::FreePolicy>)`, GNU basic_string
`_M_construct(unsigned long, char)`, and `sincosf`; UEFI and real QEMU
first-web-pixel acceptance were skipped. The next repair adds an exact
selective jsglue extraction anchor for the tracked target-only ownership
wrapper, real allocator-backed GNU string construction, and the Nagi sin/cos
math implementation's `sincosf` ABI. No host libc, host C++ runtime,
synthetic JS object, or synthetic rendering path is introduced. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

## Target-link continuation after CI run #139 (2026-09-24)

Public CI run `35909004970` (#139, head `44afc1e`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. The target user-init link resolved the MozJS ArrayBuffer wrapper,
GNU basic_string `_M_construct(unsigned long, char)`, and `sincosf`, then
reported `__isnormal`, `__isnormalf`, and `frexp`. UEFI and real QEMU
first-web-pixel acceptance were skipped. The next repair adds Nagi-owned
IEEE-bit-level normal predicates and frexp/frexpf decomposition; no host libm,
host C++ runtime, synthetic rendering, or weakened acceptance is introduced.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

## Target-link continuation after CI run #136 (2026-09-24)

Public CI run `35896205811` (#136, head `0a31126`) resolved fortified
`__memset_chk`, `__memmove_chk`, and GNU basic_string `resize(unsigned long,
char)`, then reached the next real target-link set: `fabsl`, GNU
`__throw_out_of_range_fmt`, and basic_string `_M_replace_aux(unsigned long,
unsigned long, unsigned long, char)`. UEFI and real QEMU first-web-pixel
acceptance were skipped. The next repair adds an x86-64 long-double ABI
implementation, a fail-closed GNU throw entrypoint, and allocator-backed
character replacement; it does not import a host libc or C++ runtime. M17
remains `BLOCKED`; M18 remains `NOT STARTED`.

## Target-link continuation after CI run #135 (2026-09-24)

Public CI run `35892368804` (#135, head `8e039d2`) resolved `syslog`,
`openlog`, and GNU `_Prime_rehash_policy::_M_need_rehash`, then reached the
next real target-link set: fortified `__memset_chk`, `__memmove_chk`, and GNU
basic_string `resize(unsigned long, char)`. UEFI and real QEMU first-web-pixel
acceptance were skipped. The next repair adds bounded guest-memory fortified
operations and allocator-backed GNU string resize; it does not import a host
libc or C++ runtime. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

## Target-link continuation after CI run #133 (2026-09-24)

Public CI run `35884059558` (`659a76a`) resolved the const GNU tree iterator,
erase/rebalance, and basic_string `_M_create` symbols, then reached the next
real target-link set: `strnlen`, `div`, and GNU basic_string `_M_replace`. The
next repair adds bounded guest-memory `strnlen`, the C `div_t` ABI, and real
Nagi allocator-backed string replacement. No host libc, host C++ runtime, or
synthetic link-only definition is imported. UEFI and real QEMU first-web-pixel
evidence remain pending.

## Target-link continuation after CI run #131 (2026-09-24)

Public CI run `35876355907` (`7a698f3`) passed Servo bootstrap, Mesa
Softpipe archive construction, package, and kernel compilation, then reached
the real target link. The remaining symbols were `__fprintf_chk`,
`__vfprintf_chk`, and `std::_Rb_tree_insert_and_rebalance(...)`; the latter
was declared with an incorrect `_ZSt27` length instead of the real `_ZSt29`
ABI spelling. The next repair adds Nagi-owned fortified stdio entrypoints that
reuse the bounded target `vfprintf` path and corrects the GNU tree symbol. No
host stdio, host C++ runtime, or compiler-rt is imported. UEFI and real QEMU
first-web-pixel evidence remain pending.

## Target-link continuation after CI run #132 (2026-09-24)

Public CI run `35879561669` (`79f2bf1`) resolved the fortified stdio symbols
and the correctly mangled GNU tree insertion symbol, then reached the next
real target-link set: const `_Rb_tree_increment`,
`_Rb_tree_rebalance_for_erase`, and GNU basic_string `_M_create`. The next
repair implements the real const iterator operations, GNU deletion
rebalancing/header maintenance, and Nagi allocator-backed string capacity
creation. No host C++ runtime or synthetic link-only definition is used.
UEFI and real QEMU first-web-pixel evidence remain pending.

## Target-link continuation after CI run #140 (2026-09-24)

Public CI run `35911899646` (#140, head `3546db3`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. The target user-init link resolved `__isnormal`, `__isnormalf`,
and `frexp`, then exposed the real MozJS static-archive ordering boundary:
`JS::NewArrayBufferWithContents(...)`,
`JS::RestoreMicroTaskQueue(...)`, and `__gxx_personality_v0`. UEFI and real
QEMU first-web-pixel acceptance were skipped. The next repair adds the
target-only MozJS archive-order patch `0015`, which retains the real jsglue
object and rescans `js_static`, plus a fail-closed Nagi C++ personality ABI.
It does not link host C++ or fabricate a provider; M17 remains `BLOCKED` and
M18 remains `NOT STARTED`.

## Target-link continuation after CI run #141 (2026-09-24)

Public CI run `35916232106` (#141, head `4b3c5f8`) passed the target bootstrap,
dependency, Mesa Softpipe, package, and kernel stages. `Build Nagi user init`
stopped before link resolution because rustc rejected the duplicate
`static:+whole-archive=jsglue` modifier with `overriding linking modifiers from
command line is not supported`. The next repair keeps the pinned source and
replaces that syntax with ordered raw lld archive state flags. UEFI and real
QEMU first-web-pixel acceptance remain pending; M17 remains `BLOCKED` and M18
remains `NOT STARTED`.

## Target-link continuation after CI run #142 (2026-09-24)

Public CI run `35919768358` (#142, head `0526f17`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. The ordered raw lld archive state resolved the prior MozJS
ArrayBuffer/microtask provider and personality failures. The target user-init
link then exposed `scalbn`, `__cxa_bad_typeid`, and
`std::__1::mutex::lock()`. UEFI and real QEMU first-web-pixel acceptance were
skipped. The next repair adds target-owned scaling, libc++ mutex ABI routing
to relibc pthreads, and fail-closed typeid handling. M17 remains `BLOCKED` and
M18 remains `NOT STARTED`.

## Target-link continuation after CI run #143 (2026-09-24)

Public CI run `35923751011` (#143, head `d3564a0`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. The target user-init link resolved `scalbn`,
`__cxa_bad_typeid`, and `std::__1::mutex::lock()`, then exposed the real
libc++ condition-variable and mutex-destruction boundary:
`std::__1::condition_variable::notify_all()`,
`std::__1::condition_variable::wait(unique_lock<mutex>&)`, and
`std::__1::mutex::~mutex()`. UEFI and real QEMU first-web-pixel acceptance
were skipped. The next repair routes these operations to relibc pthreads with
real ownership checks. M17 remains `BLOCKED` and M18 remains `NOT STARTED`.

## Target-link continuation after CI run #145 (2026-09-24)

Public CI run `35927852465` (#145, head `157958d`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. The target user-init build still failed after the target link
stage; UEFI and real QEMU first-web-pixel acceptance were not reached. The
public annotation exposed only the failed step, so the next experiment is
explicitly bounded: seed only the real relibc pthread mutex and condition
variable providers referenced by the Nagi-owned libc++ bridge. This is a link
ordering repair, not a host synchronization fallback or an acceptance
shortcut. M17 remains `BLOCKED` and M18 remains `NOT STARTED`.

## Target-link continuation after CI run #146 (2026-09-24)

Public CI run `35930495046` (#146, head `f90d12c`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. The target user-init custom build command still failed after the
target-link stage; its public annotation did not include the compiler/linker
symbol detail, so UEFI and real QEMU first-web-pixel acceptance were not
reached. The pthread provider seed was insufficient. The next bounded repair
adds the real Itanium deleting-destructor (`D0`) entrypoints for libc++ mutex
and condition-variable objects, with relibc destruction followed by the Nagi
allocator release. M17 remains `BLOCKED` and M18 remains `NOT STARTED`.

## Target-link continuation after CI run #147 (2026-09-24)

Public CI run `35933099876` (#147, head `216d909`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. The target user-init custom build command still failed after the
target-link stage; the D0 destructor repair was insufficient, and UEFI/QEMU
were not reached. The public annotation again exposed only the generic custom
build error. The target build diagnostic parser is extended to include
clang/runtime/linker/undefined-symbol details in the next annotation. M17
remains `BLOCKED` and M18 remains `NOT STARTED`.

## Target-link continuation after CI run #148 (2026-09-24)

Public CI run `35934736445` (#148, head `8a98bf8`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The target user-init compile then failed at
`tools/mesa/nagi-cxx-runtime.cpp:851:29`: the real libc++ mutex destructor
bridge referenced `pthread_mutex_destroy` without a declaration in the
freestanding translation unit. The diagnostic parser exposed the exact
compiler error. UEFI and real QEMU first-web-pixel acceptance were not
reached. The next bounded repair adds the missing declaration; it does not
change the runtime implementation or M17 acceptance. M17 remains `BLOCKED`
and M18 remains `NOT STARTED`.

## Target-link continuation after CI run #149 (2026-09-24)

Public CI run `35937071116` (#149, head `f5aebed`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The target user-init compile passed the missing
`pthread_mutex_destroy` declaration repair, then the real link exposed
`lrint`, `llrint`, and
`std::__1::__call_once(unsigned long volatile&, void*, void (*)(void*))`.
UEFI and real QEMU first-web-pixel acceptance were not reached. The next
bounded repair adds target-owned relibc `lrint/llrint` exports and the exact
libc++ `__call_once` ABI entrypoint, implemented with guest pthread
mutex/condition-variable synchronization. M17 remains `BLOCKED` and M18
remains `NOT STARTED`.

## Target-link continuation after CI run #150 (2026-09-24)

Public CI run `35939582983` (#150, head `fb6cdf0`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The target user-init link passed the `lrint`, `llrint`,
and libc++ `__call_once` repairs, then exposed missing target providers
`localtime_r`, `tzname`, and `setlocale`. UEFI and real QEMU first-web-pixel
acceptance were not reached. The next bounded repair adds a guest-clock UTC
`struct tm` conversion, C/POSIX locale handling, and guest UTC timezone
globals in Nagi relibc. It does not consult host time/locale state or change
M17 acceptance. M17 remains `BLOCKED` and M18 remains `NOT STARTED`.

## Target-link continuation after CI run #151 (2026-09-24)

Public CI run `35942115871` (#151, head `d56c79f`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The target user-init link passed the target time/locale
repair, then exposed `_Unwind_GetCFA`, `_Unwind_FindEnclosingFunction`, and
`strncat`. UEFI and real QEMU first-web-pixel acceptance were not reached.
The next bounded repair keeps the no-unwinder boundary fail-closed and adds a
guest-memory `strncat` implementation; it does not import host libunwind or
host libc. M17 remains `BLOCKED` and M18 remains `NOT STARTED`.

## Target-link continuation after CI run #152 (2026-09-24)

Public CI run `35944501706` (#152, head `12b0e40`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The real target user-init link then exposed
`std::bad_alloc::bad_alloc()`, `std::bad_alloc::what() const`, and `islower`
after approximately twenty-two minutes. UEFI and real QEMU first-web-pixel
acceptance were not reached. The next bounded repair supplies the matching
unversioned libc++ `std::exception`/`std::bad_alloc` Itanium ABI in the
Nagi-owned freestanding runtime and a target-owned C-locale `islower` with a
selective archive seed. No host C++/libc/locale implementation or synthetic
rendering path is used. M17 remains `BLOCKED` and M18 remains `NOT STARTED`.

## Target-link continuation after CI run #153 (2026-09-24)

Public CI run `35947092812` (#153, head `0a390c5`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The real target user-init link passed the `bad_alloc` and
`islower` repairs, then exposed
`std::__1::__next_prime(unsigned long)`,
`std::__1::locale::use_facet(std::__1::locale::id&) const`, and `nearbyint`
after approximately nineteen minutes. UEFI and real QEMU first-web-pixel
acceptance were not reached. The next bounded repair supplies the exact
libc++ hash-prime ABI, keeps unsupported locale-facet access fail-closed at
the Nagi abort boundary instead of returning a fabricated facet, and adds
target-owned IEEE `nearbyint` with a selective archive seed. No host
C++/libc/locale implementation or synthetic rendering is used. M17 remains
`BLOCKED` and M18 remains `NOT STARTED`.

## Target-link continuation after CI run #154 (2026-09-24)

Public CI run `35949658392` (#154, head `e78baac`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The real target user-init link passed the #153
`__next_prime`, locale-facet, and `nearbyint` repairs, then exposed
`pthread_getattr_np`, `pthread_attr_getstack`, and `nearbyintf` after
approximately twenty minutes. UEFI and real QEMU first-web-pixel acceptance
were not reached. The next bounded repair adds the Nagi-owned guest-stack
attribute bridge, backed by the fixed initial process stack and the actual
bounded native pthread stack, plus target-owned IEEE `nearbyintf` and
selective archive seeds. Local Windows `cargo check -p nagi-posix` remains
blocked by missing MSVC `link.exe`; formatting and diff checks pass. No host
pthread/libm implementation or synthetic rendering is used. M17 remains
`BLOCKED` and M18 remains `NOT STARTED`.

## Target-link continuation after CI run #155 (2026-09-24)

Public CI run `35952148203` (#155, head `c87f349`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The real target user-init link passed the #154
pthread-stack and `nearbyintf` repairs, then exposed `mktime`, `gmtime_r`,
and `readlink` after approximately twenty-one minutes. UEFI and real QEMU
first-web-pixel acceptance were not reached. The next bounded repair adds
target-owned UTC `mktime`/`gmtime_r` conversion and a fail-closed Nagi
`readlink` ABI for the currently unsupported Tier-B symlink operation, with
selective archive seeds. It does not import host time/filesystem/path state
or create synthetic rendering. M17 remains `BLOCKED` and M18 remains
`NOT STARTED`.

## Target-link continuation after CI run #156 (2026-09-24)

Public CI run `35954492666` (#156, head `fcdd0baf5fa4b37a934736464de4f84c872a6dea`)
passed Servo bootstrap, dependency validation, Mesa Softpipe archive
construction, package, and kernel compilation. The real user-init link failed
after about twenty-two minutes. rust-lld printed 20 distinct undefined
symbols, then stopped with `too many errors emitted`; its diagnostic explicitly
recommended `--error-limit=0`. The workflow annotation additionally truncated
the list to three entries. The visible symbols included POSIX file/runtime
entries, libc++ thread/sort/string entries, four SpiderMonkey APIs, and
`dlopen`/`dlerror`; the complete set is not yet known, so no runtime fix is
chosen from this partial list.

The first 20 are classified as follows:

- **Nagi relibc / POSIX ABI:** `remove`, `madvise`, `getrusage`, `fsync`.
- **Nagi C++ runtime / libc++:** `std::__1::this_thread::sleep_for`,
  `std::__1::__sort` instantiations for `signed char`, `int`, `long`, `short`,
  `unsigned short`, `unsigned char`, `unsigned int`, and `unsigned long`, plus
  `std::__1::basic_string::append(unsigned long, char)`.
- **SpiderMonkey / MozJS:** `JS::RestoreMicroTaskQueue`,
  `JS::InitAsyncTaskCallbacks`, `JS::Dispatchable::Run`, and
  `JS::NewArrayBufferWithContents`.
- **Dynamic loader / unsupported target facility:** `dlopen`, `dlerror`.
- **Mesa / other:** no entries appeared in the first 20. This is not evidence
  that the truncated remainder has no symbols in those categories.

The next bounded change is diagnostic-only: add `--error-limit=0` to the M17
`nagi-init` link, deduplicate all rust-lld undefined-symbol records, publish
the complete list in the GitHub job summary and chunked annotations, and scan
generated target archives/objects with `llvm-nm` for exact candidate
definitions. A definition match is evidence to inspect archive ordering or
extraction, not proof that the provider is linkable. No host runtime, fake ABI
entrypoint, acceptance change, or rendering shortcut is introduced. M17
remains `BLOCKED`; M18 remains `NOT STARTED`.
