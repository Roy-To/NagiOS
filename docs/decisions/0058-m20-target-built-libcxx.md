# ADR 0058: Target-built LLVM libc++ for guest llama.cpp

Status: accepted
Date: 2026-10-05
Milestones: M20 (AI Runtime / Granite)

## Context

Guest Granite inference links the pinned llama.cpp/ggml archives into
`nagi-init`. They were compiled against Homebrew's LLVM 19 libc++ *headers*,
but nothing provided the libc++ *library*: the final link left 75 unresolved
standard-library symbols (strings, streams, locale, filesystem, regex,
random device). A small Nagi adapter (`tools/llama/nagi-libcpp-llama.cpp`)
had been defining individual libc++ functions by hand, which does not scale
to the whole library. Linking a host libc++ archive is not valid: it is built
for the host C library and ABI. AGENTS.md requires third-party sources at
pinned revisions with reproducible builds.

## Decision

- Pin the official LLVM 19.1.7 release source tarball (version matching the
  pinned LLVM 19 compiler) in `third_party/sources.lock` by URL and SHA-256.
  `nagi-cli` (`llvm_libcxx.rs`) downloads it on demand, refuses any other
  bytes, and extracts only the subtrees the `runtimes` build reads into
  `out/cache/llvm-libcxx-19.1.7/` behind a digest marker. It is fetched by
  the commands that need it, not by every `./nagi fetch`, because the
  tarball is about 135 MiB.
- `tools/libcxx/build-nagi-target.sh` generates relibc headers from this
  tree, then builds a static `libc++.a` for `x86_64-unknown-nagi-user` with
  the Nagi target compiler wrapper: no exceptions, no RTTI, no ABI library
  (`LIBCXX_CXX_ABI=none`), pthread threading, filesystem, localization, wide
  characters and random device enabled, time-zone database disabled, no
  `pthread`/`rt`/`atomic` dependent-library pragmas. Freestanding Clang's
  `<limits.h>` does not chain to the C library, so relibc's `<limits.h>` is
  preincluded while libc++ is compiled. No upstream source is modified.
- libc++'s C dependencies are real relibc implementations: the Nagi
  C/POSIX locale layer (`newlocale`/`uselocale`, ASCII classification and
  `*_l` variants, UTF-8 multibyte conversion, `strftime_l`, `wcstod`/
  `strtold`), and `mbstate_t` now has real storage so the C and C++ views of
  it agree. Only the C/POSIX locale exists; named locales fail as POSIX
  allows.
- `tools/mesa/nagi-cxx-runtime.cpp` remains the Itanium ABI boundary
  (allocation, guards, `atexit`, type-info). Built with
  `NAGI_CXX_RUNTIME_WITH_LIBCXX`, it omits every definition libc++ provides,
  so the two never overlap.
- `./nagi m20-granite-inference` builds libc++ first, compiles llama.cpp
  against the configured libc++ headers and the regenerated relibc headers,
  and passes `NAGI_LIBCXX_ARCHIVE` to the `nagi-init` build, which links
  `libc++.a` in the llama archive group and drops the hand-written
  `nagi-libcpp-llama.cpp` definitions. The link smoke keeps its adapter.

## Consequences

- The guest C++ standard library is built from pinned upstream source for
  the Nagi target; no host library or host inference is involved.
- The libc++ build output lives under `out/m20-libcxx/` and is incremental.
  Builds log to the acceptance evidence directory.
- Other C++ consumers (Mesa, Servo's C++ dependencies) still use the
  existing runtime shims; moving them to this archive is separate work.
