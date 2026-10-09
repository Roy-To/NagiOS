#!/usr/bin/env bash
# Host measurement for the M25 local TTS provider.
#
#   tests/m25-tts/measure.sh [OUT_DIR]
#
# Fetches the pinned artifacts, builds the release `synthesize` example,
# synthesizes a fixed Japanese sentence set under /usr/bin/time -v, and, if
# WHISPER_CLI and WHISPER_MODEL are set, transcribes the produced WAV files
# with whisper.cpp as an independent intelligibility check.
#
# Output is host evidence only. It is not guest (Nagi/QEMU) evidence and does
# not exercise AudioService playback.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="${1:-$ROOT/out/tts-eval}"
CACHE="${NAGI_TTS_CACHE:-$ROOT/out/cache/tts}"
mkdir -p "$OUT"

"$ROOT/tools/tts/fetch.sh" "$CACHE" >/dev/null
VOICE="$CACHE/tohoku-f01-neutral.htsvoice"
DICT="$CACHE/naist-jdic"

cargo build --release --locked -j"${JOBS:-2}" \
  --manifest-path "$ROOT/crates/nagi-tts-provider/Cargo.toml" --example synthesize >/dev/null 2>&1
BIN="$ROOT/crates/nagi-tts-provider/target/release/examples/synthesize"

echo "== host"
echo "date_jst=$(TZ=Asia/Tokyo date '+%Y-%m-%dT%H:%M%z')"
echo "uname=$(uname -srm)"
echo "cpu=$(lscpu 2>/dev/null | sed -n 's/^Model name: *//p' | head -1) cpus=$(nproc)"
echo "mem_total_kib=$(sed -n 's/^MemTotal: *\([0-9]*\).*/\1/p' /proc/meminfo)"
echo "loadavg_before=$(cut -d' ' -f1-3 /proc/loadavg)"
echo "rustc=$(rustc --version)"
echo "voice_sha256=$(sha256sum "$VOICE" | cut -d' ' -f1)"

run() { # name format texts...
  local name="$1" format="$2"; shift 2
  echo "== $name ($format)"
  if ! /usr/bin/time -v "$BIN" "$VOICE" "$DICT" "$OUT/$name.wav" "$format" "$@" 2>"$OUT/$name.time" \
    | sed 's/^/  /'; then :; fi
  grep -E "panicked|failed:" "$OUT/$name.time" | sed 's/^/  /' || true
  grep -E "Maximum resident set size|User time|System time|Elapsed \(wall|Percent of CPU" \
    "$OUT/$name.time" | sed 's/^\t*/  /'
  if [ -n "${WHISPER_CLI:-}" ] && [ -n "${WHISPER_MODEL:-}" ] && [ -f "$OUT/$name.wav" ]; then
    echo "  whisper_transcript=$("$WHISPER_CLI" -m "$WHISPER_MODEL" -l ja -t "${JOBS:-2}" -nt \
      -f "$OUT/$name.wav" 2>/dev/null | tr -d '\n' | sed 's/^ *//')"
  fi
}

run commands stereo48k \
  "こんにちは、ナギです。" "アルバートを開いて" "今日の天気は晴れです。" "ファイルを保存しました。"
run commands_mono16k mono16k \
  "こんにちは、ナギです。" "アルバートを開いて"
run numbers stereo48k "会議は午後三時から、参加者は12人です。"
run near_cap stereo48k "日本語の音声合成が、ローカルで動作しています。"
# Expected to fail closed: the utterance exceeds the 1 MiB (5.46 s) output cap.
run over_cap stereo48k "日本語の音声合成が、ローカルで正しく動作しているかを確認します。"
echo "loadavg_after=$(cut -d' ' -f1-3 /proc/loadavg)"
