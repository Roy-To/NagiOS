#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
source_root=${NAGI_LLAMA_SOURCE:-"$repo_root/out/cache/llama-cpp-nagi"}
build_dir=$(mktemp -d)
trap 'rm -rf "$build_dir"' EXIT
"${CXX:-c++}" -std=c++17 -Wall -Wextra -Werror -ffunction-sections -fdata-sections \
    -I"$source_root/include" -I"$source_root/ggml/include" \
    "$repo_root/tests/m23-summary/tokenizer-boundaries.cpp" \
    -Wl,--gc-sections -o "$build_dir/tokenizer-boundaries"
"$build_dir/tokenizer-boundaries"
echo 'M23 summary tokenizer boundary orchestration PASS (no model inference)'
