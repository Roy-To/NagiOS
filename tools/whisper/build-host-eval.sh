#!/usr/bin/env bash
# Builds the pinned whisper.cpp revision with the Nagi patches for the HOST
# evaluation harness (tests/m25-whisper). Host builds are for measurement only
# and never stand in for the Nagi-target build (tools/whisper/build-nagi-target.sh).
#
# Inputs (env):
#   NAGI_WHISPER_PINNED  clean checkout of sources.lock [sources.whisper_cpp]
#                        (default out/cache/whisper.cpp-pinned)
#   NAGI_WHISPER_HOST_SOURCE  scratch patched copy (default out/cache/whisper-cpp-host)
#   NAGI_WHISPER_HOST_BUILD   build dir (default out/whisper-host-build)
#   NAGI_BUILD_JOBS      parallel jobs (default 1)
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
pinned=${NAGI_WHISPER_PINNED:-"$repo_root/out/cache/whisper.cpp-pinned"}
source_root=${NAGI_WHISPER_HOST_SOURCE:-"$repo_root/out/cache/whisper-cpp-host"}
build_dir=${NAGI_WHISPER_HOST_BUILD:-"$repo_root/out/whisper-host-build"}
jobs=${NAGI_BUILD_JOBS:-1}

revision=$(awk '/^\[sources.whisper_cpp\]/{s=1;next} /^\[/{s=0} s&&/^revision/{gsub(/"/,"",$3);print $3}' \
    "$repo_root/third_party/sources.lock")
if [[ -z "$revision" ]]; then
    echo "host whisper build: no [sources.whisper_cpp] revision in sources.lock" >&2
    exit 2
fi
if [[ "$(git -C "$pinned" rev-parse HEAD 2>/dev/null)" != "$revision" ]]; then
    echo "host whisper build: $pinned is not at pinned revision $revision" >&2
    exit 2
fi
if [[ -n "$(git -C "$pinned" status --porcelain)" ]]; then
    echo "host whisper build: pinned checkout $pinned has local changes" >&2
    exit 2
fi

patches=("$repo_root"/third_party/whisper-cpp-patches/*.patch)
fingerprint=$( (echo "$revision"; cat "${patches[@]}") | sha256sum | cut -d' ' -f1)
marker="$source_root/.nagi-host-patch"
if [[ ! -f "$marker" || "$(cat "$marker")" != "$fingerprint" ]]; then
    rm -rf "$source_root"
    git clone --quiet --no-checkout "$pinned" "$source_root"
    git -C "$source_root" checkout --quiet "$revision"
    for patch in "${patches[@]}"; do
        git -C "$source_root" apply --whitespace=nowarn "$patch"
    done
    echo "$fingerprint" > "$marker"
fi

cmake -S "$source_root" -B "$build_dir" \
    -DCMAKE_BUILD_TYPE=Release \
    -DBUILD_SHARED_LIBS=OFF \
    -DWHISPER_BUILD_EXAMPLES=OFF \
    -DWHISPER_BUILD_SERVER=OFF \
    -DWHISPER_BUILD_TESTS=OFF \
    -DWHISPER_CURL=OFF \
    -DGGML_BACKEND_DL=OFF \
    -DGGML_CPU=ON \
    -DGGML_NATIVE=OFF \
    -DGGML_OPENMP=OFF \
    -DGGML_CUDA=OFF \
    -DGGML_METAL=OFF \
    -DGGML_VULKAN=OFF \
    -DGGML_BUILD_TESTS=OFF
nice -n 19 cmake --build "$build_dir" --parallel "$jobs" --target whisper
