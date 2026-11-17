#!/usr/bin/env bash
set -euo pipefail

# Build the pinned LLVM libc++ as a static Nagi user-space archive (ADR 0058).
# Inputs: NAGI_LIBCXX_SOURCE (the verified source subset extracted by
# ./nagi from third_party/sources.lock), NAGI_TARGET_CLANG, NAGI_LLVM_AR and
# NAGI_LLVM_RANLIB. Outputs under NAGI_LIBCXX_OUT:
#   relibc-target/<target>/include  relibc C headers generated from this tree
#   build/include/c++/v1            configured libc++ headers (__config_site)
#   build/lib/libc++.a              the libc++ archive
# libc++ is configured without exceptions, RTTI, or an ABI library: the
# Itanium ABI boundary (allocation, guards, type-info) stays in
# tools/mesa/nagi-cxx-runtime.cpp built with NAGI_CXX_RUNTIME_WITH_LIBCXX.

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
target=${NAGI_TARGET:-x86_64-unknown-nagi-user}
rust_toolchain=${NAGI_RUST_TOOLCHAIN:-nightly-2025-08-01}
output_root=${NAGI_LIBCXX_OUT:-"$repo_root/out/m20-libcxx"}
source_root=${NAGI_LIBCXX_SOURCE:-}

if [[ -z "$source_root" || ! -f "$source_root/runtimes/CMakeLists.txt" || ! -f "$source_root/libcxx/include/cstddef" ]]; then
    echo "Nagi libc++ build: verified pinned LLVM source is missing: ${source_root:-<unset>}" >&2
    exit 2
fi
if [[ ! -f "$source_root/.nagi-libcxx-source" ]]; then
    echo "Nagi libc++ build: $source_root was not extracted by ./nagi (missing verification marker)" >&2
    exit 2
fi
for command_name in cargo make cbindgen cmake ninja; do
    command -v "$command_name" >/dev/null 2>&1 || {
        echo "Nagi libc++ build: required command not found: $command_name" >&2
        exit 2
    }
done
case "$(cbindgen --version)" in
    'cbindgen 0.28.0'*) ;;
    *)
        echo "Nagi libc++ build: cbindgen 0.28.0 is required" >&2
        exit 2
        ;;
esac

relibc_target_dir="$output_root/relibc-target"
relibc_build="$relibc_target_dir/$target"
relibc_headers="$relibc_build/include"
build_dir="$output_root/build"
mkdir -p "$output_root"

# libc++ consumes the C ABI declared by the relibc tree in this checkout,
# including the Nagi locale and wide-character layer, so generate its headers
# here rather than reusing headers from another build.
RUST_TARGET_PATH="$repo_root/targets" \
RUSTUP_TOOLCHAIN="$rust_toolchain" \
CARGO_TARGET_DIR="$relibc_target_dir" \
make -C "$repo_root/third_party/relibc" \
    TARGET="$target" \
    BUILD="$relibc_build" \
    TARGET_HEADERS="$relibc_headers" \
    headers
for header in pthread.h locale.h wchar.h wctype.h; do
    if [[ ! -f "$relibc_headers/$header" ]]; then
        echo "Nagi libc++ build: relibc did not generate $header" >&2
        exit 1
    fi
done

export NAGI_RELIBC_HEADERS="$relibc_headers"
# The target compiler wrapper requires a C++ header directory. Configure with
# the source headers, then compile against the generated build headers only:
# a second libc++ include directory would swallow libc++'s include_next into
# relibc behind already-defined header guards.
export NAGI_CXX_HEADERS="$source_root/libcxx/include"

# Freestanding Clang's <limits.h> does not include_next the C library's, so
# POSIX limits such as PATH_MAX (used by <filesystem>) need relibc's header
# preincluded while libc++ itself is compiled.
cmake -G Ninja -S "$source_root/runtimes" -B "$build_dir" \
    -DCMAKE_TOOLCHAIN_FILE="$repo_root/tools/llama/nagi-toolchain.cmake" \
    -DCMAKE_BUILD_TYPE=Release \
    "-DCMAKE_CXX_FLAGS=-mcmodel=large -ffunction-sections -fdata-sections -include $relibc_headers/limits.h" \
    -DLLVM_ENABLE_RUNTIMES=libcxx \
    -DLLVM_INCLUDE_TESTS=OFF \
    -DLIBCXX_ENABLE_SHARED=OFF \
    -DLIBCXX_ENABLE_STATIC=ON \
    -DLIBCXX_ENABLE_EXCEPTIONS=OFF \
    -DLIBCXX_ENABLE_RTTI=OFF \
    -DLIBCXX_CXX_ABI=none \
    -DLIBCXX_ENABLE_STATIC_ABI_LIBRARY=OFF \
    -DLIBCXX_ENABLE_NEW_DELETE_DEFINITIONS=OFF \
    -DLIBCXX_ENABLE_THREADS=ON \
    -DLIBCXX_HAS_PTHREAD_API=ON \
    -DLIBCXX_HAS_PTHREAD_LIB=OFF \
    -DLIBCXX_HAS_RT_LIB=OFF \
    -DLIBCXX_HAS_ATOMIC_LIB=OFF \
    -DLIBCXX_ENABLE_FILESYSTEM=ON \
    -DLIBCXX_ENABLE_LOCALIZATION=ON \
    -DLIBCXX_ENABLE_WIDE_CHARACTERS=ON \
    -DLIBCXX_ENABLE_RANDOM_DEVICE=ON \
    -DLIBCXX_ENABLE_TIME_ZONE_DATABASE=OFF \
    -DLIBCXX_HARDENING_MODE=none \
    -DLIBCXX_INCLUDE_TESTS=OFF \
    -DLIBCXX_INCLUDE_BENCHMARKS=OFF \
    -DLIBCXX_INSTALL_MODULES=OFF

ninja -C "$build_dir" generate-cxx-headers
export NAGI_CXX_HEADERS="$build_dir/include/c++/v1"
ninja -C "$build_dir" -j "${NAGI_BUILD_JOBS:-4}" cxx_static

if [[ ! -f "$build_dir/lib/libc++.a" || ! -f "$build_dir/include/c++/v1/__config_site" ]]; then
    echo "Nagi libc++ build: archive or configured headers are missing in $build_dir" >&2
    exit 1
fi
echo "Nagi libc++ build: archive $build_dir/lib/libc++.a"
echo "Nagi libc++ build: headers $build_dir/include/c++/v1"
echo "Nagi libc++ build: relibc headers $relibc_headers"
