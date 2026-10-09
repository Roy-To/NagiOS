#!/usr/bin/env bash
# Host measurement for the M25 local TTS provider.
#
#   tests/m25-tts/measure.sh [OUT_DIR]
#
# Every case declares its expected outcome (ok | reject). Outputs of a case
# (WAV, timing, stdout) are deleted before it runs, so nothing stale is ever
# read. Only cases that were expected to succeed AND succeeded are reported
# as measurements; expected rejections are reported separately; anything else
# is UNEXPECTED and makes the script exit 1.
#
# If WHISPER_CLI and WHISPER_MODEL are set, each successful WAV is
# transcribed with whisper.cpp as an independent intelligibility check.
#
# Host evidence only: not Nagi/QEMU, no AudioService playback.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="${1:-$ROOT/out/tts-eval}"
CACHE="${NAGI_TTS_CACHE:-$ROOT/out/cache/tts}"
JOBS="${JOBS:-2}"
mkdir -p "$OUT"

"$ROOT/tools/tts/fetch.sh" "$CACHE" >/dev/null || { echo "FAIL fetch"; exit 1; }
VOICE="$CACHE/tohoku-f01-neutral.htsvoice"
DICT="$CACHE/naist-jdic"

cargo build --release --locked -j"$JOBS" \
  --manifest-path "$ROOT/crates/nagi-tts-provider/Cargo.toml" --examples >/dev/null 2>&1 \
  || { echo "FAIL build"; exit 1; }
BIN_DIR="$ROOT/crates/nagi-tts-provider/target/release/examples"

echo "== host"
echo "date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "commit=$(git -C "$ROOT" rev-parse HEAD 2>/dev/null)"
echo "uname=$(uname -srm)"
echo "cpu=$(lscpu 2>/dev/null | sed -n 's/^Model name: *//p' | head -1) cpus=$(nproc)"
echo "mem_total_kib=$(sed -n 's/^MemTotal: *\([0-9]*\).*/\1/p' /proc/meminfo)"
echo "loadavg_before=$(cut -d' ' -f1-3 /proc/loadavg)"
echo "rustc=$(rustc --version)"
echo "profile=release (codegen-units=1), single thread"
echo "voice_sha256=$(sha256sum "$VOICE" | cut -d' ' -f1)"

status=0

finish_case() { # name expected rc
  local name="$1" expected="$2" rc="$3"
  sed 's/^/  /' "$OUT/$name.stdout"
  if [ "$expected" = ok ] && [ "$rc" -eq 0 ]; then
    echo "  OUTCOME=SUCCESS"
    grep -E "Maximum resident set size|User time|System time|Elapsed \(wall" \
      "$OUT/$name.time" | sed 's/^\t*/  /'
    if [ -n "${WHISPER_CLI:-}" ] && [ -n "${WHISPER_MODEL:-}" ] && [ -f "$OUT/$name.wav" ]; then
      echo "  whisper_transcript=$("$WHISPER_CLI" -m "$WHISPER_MODEL" -l ja -t "$JOBS" -nt \
        -f "$OUT/$name.wav" 2>/dev/null | tr -d '\n' | sed 's/^ *//')"
    fi
  elif [ "$expected" = reject ] && [ "$rc" -eq 3 ] && [ ! -f "$OUT/$name.wav" ]; then
    echo "  OUTCOME=EXPECTED_REJECT (bounded provider error, no WAV, not a measurement)"
  else
    echo "  OUTCOME=UNEXPECTED expected=$expected exit=$rc"
    head -5 "$OUT/$name.time" | sed 's/^/  stderr: /'
    status=1
  fi
}

synth_case() { # name expected format texts...
  local name="$1" expected="$2" format="$3"; shift 3
  echo "== synth $name ($format, expect $expected)"
  rm -f "$OUT/$name.wav" "$OUT/$name.time" "$OUT/$name.stdout"
  /usr/bin/time -v "$BIN_DIR/synthesize" "$VOICE" "$DICT" "$OUT/$name.wav" "$format" "$@" \
    >"$OUT/$name.stdout" 2>"$OUT/$name.time"
  finish_case "$name" "$expected" $?
}

worst_case() { # expected format kind
  local expected="$1" format="$2" kind="$3"
  local name="worst_${format}_${kind//:/_}"
  echo "== worst $kind ($format, expect $expected)"
  rm -f "$OUT/$name.time" "$OUT/$name.stdout"
  /usr/bin/time -v "$BIN_DIR/worst_case" "$VOICE" "$DICT" "$format" "$kind" \
    >"$OUT/$name.stdout" 2>"$OUT/$name.time"
  local rc=$?
  sed 's/^/  /' "$OUT/$name.stdout"
  if { [ "$expected" = ok ] && [ $rc -eq 0 ]; } || { [ "$expected" = reject ] && [ $rc -eq 3 ]; }; then
    echo "  OUTCOME=$([ $rc -eq 0 ] && echo SUCCESS || echo EXPECTED_REJECT)"
    grep -E "Maximum resident set size|User time" "$OUT/$name.time" | sed 's/^\t*/  /'
  else
    echo "  OUTCOME=UNEXPECTED expected=$expected exit=$rc"
    status=1
  fi
}

synth_case commands ok stereo48k \
  "こんにちは、ナギです。" "アルバートを開いて" "今日の天気は晴れです。" "ファイルを保存しました。"
synth_case commands_mono16k ok mono16k "こんにちは、ナギです。" "アルバートを開いて"
synth_case numbers ok stereo48k "会議は午後三時から、参加者は12人です。"
synth_case near_cap ok stereo48k "日本語の音声合成が、ローカルで動作しています。"
# Predicted longer than 5.46 s of stereo 48 kHz: rejected at begin, no audio.
synth_case over_cap reject stereo48k "日本語の音声合成が、ローカルで正しく動作しているかを確認します。"

# Worst-case bounded (<= 1 KiB) inputs: time-to-first-chunk, cancel cost,
# peak memory, and whether the 1 MiB budget rejects them up front.
for format in stereo48k mono16k; do
  for kind in sentences kanji digits latin mixed; do
    worst_case reject "$format" "$kind"
  done
done
worst_case reject stereo48k hiragana
worst_case ok mono16k hiragana
worst_case ok stereo48k punctuation
worst_case ok stereo48k fit:2
worst_case reject stereo48k fit:3
worst_case ok mono16k fit:12
worst_case reject mono16k fit:13

echo "loadavg_after=$(cut -d' ' -f1-3 /proc/loadavg)"
echo "RESULT=$([ $status -eq 0 ] && echo PASS || echo FAIL)"
exit $status
