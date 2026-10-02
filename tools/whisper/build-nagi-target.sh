#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
source_root=${NAGI_WHISPER_SOURCE:-"$repo_root/out/cache/whisper-cpp-nagi"}
build_dir=${NAGI_WHISPER_BUILD:-"$repo_root/out/m25-whisper-target"}

if [[ ! -f "$source_root/CMakeLists.txt" ]]; then
    echo "Nagi whisper build: validated patched source checkout is missing: $source_root" >&2
    exit 2
fi
if [[ -z "${NAGI_RELIBC_HEADERS:-}" ]]; then
    export NAGI_RELIBC_HEADERS="$repo_root/out/m17-mesa/relibc-target/x86_64-unknown-nagi-user/include"
fi
if [[ ! -f "$NAGI_RELIBC_HEADERS/pthread.h" ]]; then
    echo "Nagi whisper build: generated relibc headers are missing: $NAGI_RELIBC_HEADERS" >&2
    exit 2
fi
if [[ -z "${NAGI_CXX_HEADERS:-}" && -n "${NAGI_TARGET_CLANG:-}" ]]; then
    compiler_dir=$(cd "$(dirname "$NAGI_TARGET_CLANG")" && pwd)
    candidate="$compiler_dir/../include/c++/v1"
    if [[ -f "$candidate/cstddef" ]]; then
        export NAGI_CXX_HEADERS=$(cd "$candidate" && pwd)
    fi
fi
if [[ -z "${NAGI_CXX_HEADERS:-}" || ! -f "$NAGI_CXX_HEADERS/cstddef" ]]; then
    echo "Nagi whisper build: set NAGI_CXX_HEADERS to the target libc++ include directory" >&2
    exit 2
fi

mkdir -p "$build_dir"
cmake -S "$source_root" -B "$build_dir" \
    -DCMAKE_TOOLCHAIN_FILE="$repo_root/tools/whisper/nagi-toolchain.cmake" \
    -DCMAKE_BUILD_TYPE=Release \
    "-DCMAKE_C_FLAGS_RELEASE=-O3 -DNDEBUG -D__NAGI__ -ffunction-sections -fdata-sections" \
    "-DCMAKE_CXX_FLAGS_RELEASE=-O3 -DNDEBUG -D__NAGI__ -ffunction-sections -fdata-sections" \
    -DBUILD_SHARED_LIBS=OFF \
    -DWHISPER_BUILD_EXAMPLES=OFF \
    -DWHISPER_BUILD_SERVER=OFF \
    -DWHISPER_BUILD_TESTS=OFF \
    -DWHISPER_CURL=OFF \
    -DWHISPER_OPENVINO=OFF \
    -DGGML_ACCELERATE=OFF \
    -DGGML_BACKEND_DL=OFF \
    -DGGML_CPU=ON \
    -DGGML_CPU_ALL_VARIANTS=OFF \
    -DGGML_CUDA=OFF \
    -DGGML_METAL=OFF \
    -DGGML_NATIVE=OFF \
    -DGGML_OPENMP=OFF \
    -DGGML_OPENVINO=OFF \
    -DGGML_RPC=OFF \
    -DGGML_VULKAN=OFF \
    -DGGML_WEBGPU=OFF \
    -DGGML_BUILD_TESTS=OFF

cmake --build "$build_dir" --parallel "${NAGI_BUILD_JOBS:-4}" --target whisper
