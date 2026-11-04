#!/usr/bin/env bash
# Host acceptance runner for the M25 local TTS provider.
#
#   tests/m25-tts/acceptance.sh [LOG_DIR]
#
# Steps (each logged in full to LOG_DIR/<step>.log, summary in summary.txt):
#   fetch           tools/tts/fetch.sh (pinned, checksummed artifacts)
#   fmt             cargo fmt --check
#   clippy          cargo clippy --all-targets -D warnings (engine enabled)
#   clippy-core     same with --no-default-features (no_std core only)
#   unit            cargo test: model-free tests; real-engine tests must be
#                   reported as ignored, never as passed
#   real            cargo test --test real_engine -- --ignored
#                   (real Japanese synthesis with the pinned artifacts)
#   real-adversarial  cargo test --test real_adversarial -- --ignored
#                   (corrupted voice/dictionary bytes, lifecycle after errors;
#                   exactly 3 tests must pass; test names and the result line
#                   are echoed to stdout so CI logs show them)
#   real-hostile    cargo test --test real_hostile_model -- --ignored
#                   (hostile duration PDFs / STREAM_WIN rows in the pinned
#                   voice; exactly 1 test must pass; echoed like above)
#   missing-assets  each real suite with NAGI_TTS_VOICE/NAGI_TTS_DICT unset
#                   must FAIL (guards against a silent skip)
#
# Any failing step makes the script exit 1. Missing artifacts are a FAIL.
# This is host acceptance only: it does not run on Nagi and does not exercise
# AudioService playback.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LOG_DIR="${1:-$ROOT/out/tts-acceptance/$(date -u +%Y%m%dT%H%M%SZ)}"
CACHE="${NAGI_TTS_CACHE:-$ROOT/out/cache/tts}"
MANIFEST="$ROOT/crates/nagi-tts-provider/Cargo.toml"
JOBS="${JOBS:-2}"
mkdir -p "$LOG_DIR"
SUMMARY="$LOG_DIR/summary.txt"
: >"$SUMMARY"
status=0

record() { # step outcome detail
  printf '%-15s %-5s %s\n' "$1" "$2" "$3" | tee -a "$SUMMARY"
  [ "$2" = PASS ] || status=1
}

run_step() { # step command...
  local step="$1"; shift
  local log="$LOG_DIR/$step.log"
  { echo "\$ $*"; "$@"; } >"$log" 2>&1
  local rc=$?
  if [ $rc -eq 0 ]; then record "$step" PASS "exit 0"; else record "$step" FAIL "exit $rc"; fi
  return $rc
}

{
  echo "date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "commit=$(git -C "$ROOT" rev-parse HEAD 2>/dev/null)"
  echo "dirty=$(git -C "$ROOT" status --porcelain -- crates/nagi-tts-provider tools/tts tests/m25-tts | wc -l)"
  echo "host=$(uname -srm) cpus=$(nproc)"
  echo "rustc=$(rustc --version)"
} >"$LOG_DIR/environment.txt"

run_step fetch "$ROOT/tools/tts/fetch.sh" "$CACHE"
run_step fmt cargo fmt --manifest-path "$MANIFEST" -- --check
run_step clippy cargo clippy --manifest-path "$MANIFEST" -j"$JOBS" --all-targets --locked -- -D warnings
run_step clippy-core cargo clippy --manifest-path "$MANIFEST" -j"$JOBS" --no-default-features --all-targets --locked -- -D warnings

# Model-free tests: the artifacts are deliberately not exported here.
if run_step unit env -u NAGI_TTS_VOICE -u NAGI_TTS_DICT \
    cargo test --manifest-path "$MANIFEST" -j"$JOBS" --locked; then
  if grep -q "test result: ok. 0 passed; 0 failed; [1-9][0-9]* ignored" "$LOG_DIR/unit.log"; then
    record unit-ignored PASS "real-engine tests reported as ignored, not passed"
  else
    record unit-ignored FAIL "real-engine tests were not reported as ignored"
  fi
  # Each real-artifact binary must report all of its tests as ignored.
  for expect in "real_adversarial:3" "real_hostile_model:1"; do
    bin="${expect%%:*}"; count="${expect##*:}"
    if awk -v bin="tests/$bin.rs" '
          index($0, "Running " bin) { on = 1; next }
          on && /^test result:/ { print; exit }' "$LOG_DIR/unit.log" \
        | grep -q "test result: ok\. 0 passed; 0 failed; $count ignored"; then
      record "unit-ign-$bin" PASS "$count ignored without artifacts"
    else
      record "unit-ign-$bin" FAIL "expected $count ignored without artifacts"
    fi
  done
fi

export NAGI_TTS_VOICE="$CACHE/tohoku-f01-neutral.htsvoice"
export NAGI_TTS_DICT="$CACHE/naist-jdic"
if run_step real cargo test --manifest-path "$MANIFEST" -j"$JOBS" --locked \
    --test real_engine -- --ignored --test-threads=2; then
  if grep -Eq "test result: ok\. [1-9][0-9]* passed; 0 failed; 0 ignored" "$LOG_DIR/real.log"; then
    record real-count PASS "$(grep -Eo '[0-9]+ passed' "$LOG_DIR/real.log" | tail -1) with real artifacts"
  else
    record real-count FAIL "real suite did not report passes"
  fi
fi

# Additional real-artifact suites. Their test lines and result line are
# echoed so the CI log itself shows which tests executed.
real_suite() { # step test-binary expected-passes
  local step="$1" bin="$2" expect="$3"
  if run_step "$step" cargo test --manifest-path "$MANIFEST" -j"$JOBS" --locked \
      --test "$bin" -- --ignored --test-threads=1; then
    :
  fi
  echo "---- $bin (from $LOG_DIR/$step.log) ----"
  grep -E '^(running [0-9]+ tests?|test [A-Za-z0-9_:]+ \.\.\. |test result:)' "$LOG_DIR/$step.log"
  if grep -q "test result: ok\. $expect passed; 0 failed; 0 ignored" "$LOG_DIR/$step.log"; then
    record "$step-count" PASS "$expect passed with real artifacts"
  else
    record "$step-count" FAIL "expected exactly $expect passed"
  fi
}
real_suite real-adversarial real_adversarial 3
real_suite real-hostile real_hostile_model 1

# The acceptance mode must fail closed without artifacts.
for bin in real_adversarial real_hostile_model; do
  log="$LOG_DIR/missing-assets-$bin.log"
  env -u NAGI_TTS_VOICE -u NAGI_TTS_DICT cargo test --manifest-path "$MANIFEST" -j"$JOBS" --locked \
    --test "$bin" -- --ignored >"$log" 2>&1
  rc=$?
  if [ $rc -ne 0 ] && grep -q "FAIL: NAGI_TTS_VOICE is not set" "$log" \
      && grep -q "test result: FAILED\. 0 passed;" "$log"; then
    record "missing-$bin" PASS "fails (exit $rc) without artifacts"
  else
    record "missing-$bin" FAIL "did not fail without artifacts (exit $rc)"
  fi
done
log="$LOG_DIR/missing-assets.log"
env -u NAGI_TTS_VOICE -u NAGI_TTS_DICT cargo test --manifest-path "$MANIFEST" -j"$JOBS" --locked \
  --test real_engine -- --ignored synthesizes_audible_japanese_stereo_48k >"$log" 2>&1
rc=$?
if [ $rc -ne 0 ] && grep -q "FAIL: NAGI_TTS_VOICE is not set" "$log"; then
  record missing-assets PASS "real suite fails (exit $rc) without artifacts"
else
  record missing-assets FAIL "real suite did not fail without artifacts (exit $rc)"
fi

echo "logs: $LOG_DIR"
exit $status
