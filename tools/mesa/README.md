# M17 Mesa build boundary

`nagi-x86_64-user.meson.cross` is the tracked Meson cross configuration for
the Nagi user-space Mesa/Softpipe archive build. It deliberately builds static
archives for `system = 'nagi'`; it does not link a host executable or select
X11, Wayland, WGL, or a host OpenGL implementation.

Clang receives `x86_64-unknown-elf` because
`x86_64-unknown-nagi-user` is a Rust target JSON identity, not a portable LLVM
target triple. This is the freestanding ELF code-generation ABI; the Nagi
system identity remains `system = 'nagi'`, and Cargo owns the final Nagi ELF
link.

The intended configuration is:

```text
meson setup out/mesa-build third_party/mesa \
  --cross-file tools/mesa/nagi-x86_64-user.meson.cross \
  -Dgallium-drivers=softpipe -Dvulkan-drivers= \
  -Dplatforms=nagi -Degl-native-platform=surfaceless \
  -Dglx=disabled -Dllvm=disabled -Dshared-glapi=enabled \
  -Dosmesa=false -Dopengl=true \
  -Dzlib=disabled -Dzstd=disabled -Dshader-cache=disabled \
  -Dexpat=disabled -Dxmlconfig=disabled \
  -Dbuild-tests=false -Denable-glcpp-tests=false -Dbuild-aco-tests=false \
  '-Dc_args=--target=x86_64-unknown-elf -ffreestanding -fno-stack-protector -fno-builtin -mcmodel=large -I<generated-relibc-headers>' \
  '-Dcpp_args=--target=x86_64-unknown-elf -ffreestanding -fno-stack-protector -fno-builtin -mcmodel=large -I<generated-relibc-headers>' \
  -Dprefix=<absolute-staging-prefix>
```

`third_party/relibc/Makefile headers` generates the C ABI headers from the
same Nagi target source used by the Rust user runtime. The tracked
`tools/mesa/nagi-headers/time.h` wrapper then selects the Nagi clock IDs (1 and
4) after including that generated header, and `stdint.h` supplies the C99
integer-constant macros missing from freestanding target clang. The `fcntl.h`
wrapper likewise supplies Nagi's descriptor and open/create flag values when
relibc's target-specific fcntl module is not selected by cbindgen. The tracked
`tools/mesa/nagi-headers/sys/mman.h` wrapper does the same for Nagi's mmap
protection and mapping flags. These wrappers preserve the real Nagi POSIX ABI
when cbindgen cannot resolve the target-specific re-export. The target build
must
provide `cbindgen`, `flex`, `bison`, LLVM's `ar`/`ranlib`/`nm`/`objcopy`/`strip`,
and the pinned Python generators listed in `tools/mesa/requirements.txt`; it
must not point Mesa at the host system headers. The helper pins relibc's Cargo
invocation to
`nightly-2025-08-01` (override with `NAGI_RUST_TOOLCHAIN` only when the whole
target toolchain is intentionally changed) and requires cbindgen 0.28.0. The
target user build also routes `cc-rs` C helpers through
`tools/nagi-target-cc.sh`. That wrapper selects the same freestanding ELF
triple, the tracked Nagi C header overlay, and generated relibc headers, and
rejects a missing `pthread.h`; it
prevents a host pthread layout or host C runtime from entering a Nagi object.
The pre-Mesa `nagi-pthread-header-check.c` syntax check locks the generated
four-byte rwlock ABI to the relibc implementation.
The companion `nagi-c11-header-check.c` syntax/size check locks the generated
atomic and `struct termios` interfaces used by real target C dependencies such
as aws-lc.
The tracked `nagi-headers/type_traits` header is intentionally limited to the
`std::underlying_type_t` trait used by Mesa's selected enum-operator helpers;
it does not claim to provide a general C++ standard library. The
`platforms=nagi` setting is enabled by the
tracked patch in `third_party/mesa-patches/0001-nagi-platform-static-softpipe.patch`.

The adapter must still expose only the Nagi framebuffer handoff after the
archive is linked. After `ninja`, `tools/mesa/build.sh` combines the target
static archives into `out/m17-mesa/libnagi_mesa.a`; `nagi-albert` links that
archive only for `target_os = "nagi"`. This build metadata is not itself a
rendering implementation.
