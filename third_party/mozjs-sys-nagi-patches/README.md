# Nagi mozjs_sys patches

These ordered patches adapt the pinned `mozjs_sys 153.0.0-2` source build to
Nagi's freestanding target toolchain. The Mozilla configure triplet is only a
build-system identifier; the Rust target, compiler wrapper, relibc headers,
and final guest link remain Nagi-owned. The configure-only triplet is the
Nagi-owned `x86_64-unknown-nagi` form; the ordered adapter patch teaches the
pinned Mozilla configure layer to recognize it without relabeling Nagi as
Linux/WASI or using a host runtime.

The ordered patches also route Nagi C/C++ preprocessing through the existing
freestanding compiler wrapper and suppress both mozjs_sys's explicit and
`cc-rs`-inferred host `stdc++` link requests for `nagi-user`. They also bind
Mozilla's archiver lookup
to the pinned `llvm-ar` already used by the Nagi Mesa toolchain, rather than
inventing a target-prefixed GNU binutils executable. Nagi's C++ sources
therefore remain subject to the Nagi headers/toolchain and do not acquire a
host C++ runtime. The platform adapter selects Mozilla's existing POSIX
`TimeStamp` implementation for Nagi; its `clock_gettime(CLOCK_MONOTONIC)`
calls resolve through the generated relibc/Nagi PAL boundary. The target
compiler wrapper can also consume the pinned Ubuntu libc++ headers through an
explicit `NAGI_CXX_HEADERS` path; this is a compile-time header input only and
does not link a host C++ runtime. When that path is enabled, the wrapper keeps
the real libc++ headers before the Mesa compatibility headers so libc++
`include_next` resolves through the Nagi relibc boundary rather than through a
Mesa-only C++ shim. Because Nagi's target triple is custom, the Servo build
script also selects libc++'s pthread thread backend explicitly; those pthread
symbols resolve to the existing Nagi POSIX runtime boundary.
Nagi also selects libc++'s portable default rune table because the current
target runtime does not provide a host locale database; this is header-level
ctype support and does not import host locale state. Patch `0007` was an
initial diagnostic attempt to disable libc++ localization, but that broad
switch also removes standard streambuf types. Patch `0008` therefore restores
localization after the Nagi-owned relibc backend supplies the C/POSIX numeric
`strto*_l` ABI declared by the generated target headers. The final build keeps
libc++'s normal numeric facets without importing host locale state.

Patch `0009` makes the Nagi target's real `malloc_usable_size` declaration
visible to MozJS's allocator bridge. The freestanding Nagi `stdlib.h` does not
implicitly include the non-POSIX `malloc.h` header, so the target-only patch
includes that pinned relibc header under `__NAGI__`; it does not add a host
allocator or replace allocator accounting.

Patch `0010` selects MozJS's existing absolute condition-variable timeout path
for Nagi and uses the real relibc `CLOCK_REALTIME` clock. Nagi already provides
`pthread_cond_timedwait`, `pthread_condattr_setclock`, and `clock_gettime`; it
does not need the macOS/Android-only `pthread_cond_timedwait_relative_np`
extension. This keeps Servo/MozJS synchronization on the guest pthread ABI.

Patch `0011` selects MozJS's existing no-op mmap fault-handler boundary for
Nagi. Nagi's first-pixel vertical slice does not expose Unix signal delivery,
and its guest file mappings are owned by the Nagi POSIX memory facade rather
than a host mmap that can deliver `SIGBUS`. The patch therefore keeps the
`MmapAccessScope` macros source-compatible while excluding the Unix
`sigaction`/`siglongjmp` implementation; it does not add a fake signal API or
redirect faults to the host.

Patch `0012` applies the same boundary explicitly to MozJS's bindgen phase.
bindgen invokes libclang separately from the C++ compiler wrapper, so wrapper-
internal include arguments and the Rust-only `-user` target suffix are not
inherited automatically. The patch maps bindgen to the canonical compile-only
`x86_64-unknown-elf` spelling and supplies the pinned libc++, generated relibc,
and Nagi Mesa header roots. It does not add host headers, a host C++ runtime,
or a synthetic target ABI; the resulting bindings still compile against the
Nagi-owned guest interfaces.

Patch `0013` extends MozJS's existing system-allocator size bridge to the Nagi
platform. The pinned `jsglue.cpp` otherwise rejects Nagi at its platform
conditional even though the real relibc `malloc.h` ABI already provides
`malloc_usable_size`. The patch selects that guest header and existing ABI
under `__NAGI__`; it does not call a host allocator or replace allocator
accounting with a constant.

Patch `0014` restores the real `JS::NewArrayBufferWithContents` ownership
wrapper for the Nagi target when the pinned target archive omits the inline
UniquePtr forwarding object. It delegates to SpiderMonkey's four-argument
API and releases the guest allocation only after successful ArrayBuffer
creation; failed creation retains the UniquePtr deleter. It is target-only
and does not return a synthetic object or leak contents.

The final Nagi MozJS link is assembled by user/nagi-init/build.rs. The
transitive mozjs_sys build-script search paths are retained, but its native
archive link arguments are not present on the final nagi-init link. M17
therefore places the real js_static, jsapi, and jsglue archives in a selective
ELF linker group at the final binary boundary. The group rescans archive
members to resolve real SpiderMonkey providers without forcing every object
into the image or importing a host runtime.

The generated source is materialized from `third_party/sources.lock` and is
never edited in the Cargo cache. Each patch is checked and applied before the
source fingerprint is recorded.
