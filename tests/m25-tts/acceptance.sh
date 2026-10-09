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
#   missing-assets  the real suite with NAGI_TTS_VOICE/NAGI_TTS_DICT unset
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

# The acceptance mode must fail closed without artifacts.
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
