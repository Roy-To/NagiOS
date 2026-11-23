#!/bin/sh
set -eu

: "${NAGI_ZIG:?Set NAGI_ZIG to the Zig executable path}"
: "${ZIG_GLOBAL_CACHE_DIR:?Set a Job-local ZIG_GLOBAL_CACHE_DIR}"
: "${ZIG_LOCAL_CACHE_DIR:?Set a Job-local ZIG_LOCAL_CACHE_DIR}"

zig_bin_dir=$(CDPATH= cd "$(dirname "$NAGI_ZIG")" && pwd -P)
export ZIG_LIB_DIR="${NAGI_ZIG_LIB_DIR:-$zig_bin_dir/lib}"
linux_target="${NAGI_ZIG_TARGET:-x86_64-linux-gnu.2.17.0}"

exec "$NAGI_ZIG" cc -target "$linux_target" "$@"
