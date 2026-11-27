#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
builder="$script_dir/build_nagi_init_elf.sh"
label_base="preflight-$$"
temp_root=${TMPDIR:-/tmp}

assert_rejected_before_target() {
  label=$1
  expected=$2
  shift 2
  target="$script_dir/target-nagi-init-rust198-$label"
  log="$temp_root/nagi-init-build-negative-$$.log"
  rm -f "$log"
  if "$@" "$builder" "$label" >"$log" 2>&1; then
    echo "expected preflight rejection for $label" >&2
    cat "$log" >&2
    rm -f "$log"
    exit 1
  fi
  if [ -e "$target" ] || [ -L "$target" ]; then
    echo "preflight created an output target for $label" >&2
    rm -f "$log"
    exit 1
  fi
  if ! /usr/bin/grep -F "$expected" "$log" >/dev/null; then
    echo "preflight rejected $label for an unexpected reason" >&2
    cat "$log" >&2
    rm -f "$log"
    exit 1
  fi
  rm -f "$log"
  echo "negative=$label rejected_before_target=PASS"
}

assert_rejected_before_target "$label_base-encoded" \
  "refusing caller CARGO_ENCODED_RUSTFLAGS" \
  /usr/bin/env CARGO_ENCODED_RUSTFLAGS='--cfg wrong_build'
assert_rejected_before_target "$label_base-linker" \
  "refusing conflicting CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER" \
  /usr/bin/env CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=/usr/bin/false
assert_rejected_before_target "$label_base-clang" \
  "refusing conflicting NAGI_TARGET_CLANG" \
  /usr/bin/env NAGI_TARGET_CLANG=/usr/bin/false
assert_rejected_before_target "$label_base-profile" \
  "refusing caller CARGO_PROFILE_DEV_*" \
  /usr/bin/env CARGO_PROFILE_DEV_OPT_LEVEL=3
assert_rejected_before_target "$label_base-build-dir" \
  "refusing conflicting CARGO_BUILD_BUILD_DIR" \
  /usr/bin/env CARGO_BUILD_BUILD_DIR="$temp_root/nagi-init-build-negative-$$-outside"

bad_label="$label_base/escape"
if "$builder" "$bad_label" >"$temp_root/nagi-init-build-negative-$$.log" 2>&1; then
  echo "expected invalid target label to be rejected" >&2
  rm -f "$temp_root/nagi-init-build-negative-$$.log"
  exit 1
fi
if [ -e "$script_dir/target-nagi-init-rust198-$label_base" ] ||
   [ -L "$script_dir/target-nagi-init-rust198-$label_base" ]; then
  echo "invalid label created an output target" >&2
  rm -f "$temp_root/nagi-init-build-negative-$$.log"
  exit 1
fi
rm -f "$temp_root/nagi-init-build-negative-$$.log"
echo "negative=path_label rejected_before_target=PASS"

plan_label="$label_base-plan"
plan=$("$builder" "$plan_label" --plan)
printf '%s\n' "$plan" | /usr/bin/grep -F -- '--features linux-service-action-bridge,m10-desktop,linux-dev-data-selection' >/dev/null
printf '%s\n' "$plan" | /usr/bin/grep -F 'linux_dev_preview_smoke,linux_desktop_dev_smoke' >/dev/null
printf '%s\n' "$plan" | /usr/bin/grep -F 'profile=debug' >/dev/null
printf '%s\n' "$plan" | /usr/bin/grep -F "build_dir=$script_dir/target-nagi-init-rust198-$plan_label" >/dev/null
if [ -e "$script_dir/target-nagi-init-rust198-$plan_label" ] ||
   [ -L "$script_dir/target-nagi-init-rust198-$plan_label" ]; then
  echo "plan mode created an output target" >&2
  exit 1
fi
echo "positive=plan_isolated_debug_linux_dev_preview=PASS"
