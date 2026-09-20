# Nagi mozjs_sys patches

These ordered patches adapt the pinned `mozjs_sys 153.0.0-2` source build to
Nagi's freestanding target toolchain. The Mozilla configure triplet is only a
build-system identifier; the Rust target, compiler wrapper, relibc headers,
and final guest link remain Nagi-owned. The configure-only triplet is the
Nagi-owned `x86_64-unknown-nagi` form; the ordered adapter patch teaches the
pinned Mozilla configure layer to recognize it without relabeling Nagi as
Linux/WASI or using a host runtime.

The ordered patches also route Nagi C/C++ preprocessing through the existing
freestanding compiler wrapper and suppress mozjs_sys's default host
`stdc++` link request for `nagi-user`. They also bind Mozilla's archiver lookup
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

The generated source is materialized from `third_party/sources.lock` and is
never edited in the Cargo cache. Each patch is checked and applied before the
source fingerprint is recorded.
