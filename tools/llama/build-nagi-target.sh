#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
source_root="$repo_root/out/cache/llama-cpp-nagi"
build_dir=${NAGI_LLAMA_BUILD:-"$repo_root/out/m20-llama-target"}
target=${NAGI_TARGET:-x86_64-unknown-nagi-user}

if [[ ! -f "$source_root/CMakeLists.txt" ]]; then
    echo "Nagi llama build: validated patched source checkout is missing: $source_root" >&2
    exit 2
fi

if [[ -z "${NAGI_RELIBC_HEADERS:-}" ]]; then
    export NAGI_RELIBC_HEADERS="$repo_root/out/m17-mesa/relibc-target/$target/include"
fi
if [[ ! -f "$NAGI_RELIBC_HEADERS/pthread.h" ]]; then
    echo "Nagi llama build: generated relibc headers are missing: $NAGI_RELIBC_HEADERS" >&2
    exit 2
fi

if [[ -z "${NAGI_CXX_HEADERS:-}" ]]; then
    if [[ -n "${NAGI_TARGET_CLANG:-}" ]]; then
        compiler_dir=$(cd "$(dirname "$NAGI_TARGET_CLANG")" && pwd)
        candidate="$compiler_dir/../include/c++/v1"
        if [[ -f "$candidate/cstddef" ]]; then
            export NAGI_CXX_HEADERS=$(cd "$candidate" && pwd)
        fi
    fi
fi
if [[ -z "${NAGI_CXX_HEADERS:-}" || ! -f "$NAGI_CXX_HEADERS/cstddef" ]]; then
    echo "Nagi llama build: set NAGI_CXX_HEADERS to the target libc++ include directory" >&2
    exit 2
fi

mkdir -p "$build_dir"
cmake -S "$source_root" -B "$build_dir" \
    -DCMAKE_TOOLCHAIN_FILE="$repo_root/tools/llama/nagi-toolchain.cmake" \
    -DCMAKE_BUILD_TYPE=Release \
    "-DCMAKE_C_FLAGS_RELEASE=-O3 -DNDEBUG -ffunction-sections -fdata-sections" \
    "-DCMAKE_CXX_FLAGS_RELEASE=-O3 -DNDEBUG -ffunction-sections -fdata-sections" \
    -DBUILD_SHARED_LIBS=OFF \
    -DLLAMA_BUILD_APP=OFF \
    -DLLAMA_BUILD_COMMON=OFF \
    -DLLAMA_BUILD_EXAMPLES=OFF \
    -DLLAMA_BUILD_SERVER=OFF \
    -DLLAMA_BUILD_TESTS=OFF \
    -DLLAMA_BUILD_TOOLS=OFF \
    -DLLAMA_BUILD_MTMD=OFF \
    -DLLAMA_BUILD_UI=OFF \
    -DLLAMA_OPENSSL=OFF \
    -DGGML_ACCELERATE=OFF \
    -DGGML_BACKEND_DL=OFF \
    -DGGML_CPU=ON \
    -DGGML_CPU_ALL_VARIANTS=OFF \
    -DGGML_CUDA=OFF \
    -DGGML_METAL=OFF \
    -DGGML_NATIVE=OFF \
    -DGGML_OPENMP=OFF \
    -DGGML_RPC=OFF \
    -DGGML_VULKAN=OFF \
    -DGGML_WEBGPU=OFF \
    -DGGML_BUILD_TESTS=OFF

cmake --build "$build_dir" --parallel "${NAGI_BUILD_JOBS:-4}" --target llama
