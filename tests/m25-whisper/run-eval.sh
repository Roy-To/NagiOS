#!/usr/bin/env bash
# Host evaluation of the M25 Whisper provider on unseen Japanese speech.
# Produces HOST measurements only (never a guest result).
#
# Prerequisites:
#   tools/whisper/build-host-eval.sh            (pinned+patched host whisper.cpp)
#   tools/whisper/fetch-fleurs-ja-eval.py       (CC BY 4.0 audio, hash-verified)
#   locked model third_party/models.lock [models.whisper_small_multilingual]
#     at $NAGI_WHISPER_MODEL (default out/cache/whisper-models/ggml-small.bin)
# Optional: NAGI_M25_WHISPER_FIXTURE_SHA256 = SHA-256 of the guest acceptance
#   fixture PCM; any evaluation clip with that hash is refused.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
model=${NAGI_WHISPER_MODEL:-"$repo_root/out/cache/whisper-models/ggml-small.bin"}
audio_dir=${NAGI_M25_EVAL_AUDIO:-"$repo_root/out/m25-whisper-eval/fleurs"}
output=$(realpath -m "${1:-"$repo_root/out/m25-whisper-eval/host-eval.json"}")
model=$(realpath -m "$model")
audio_dir=$(realpath -m "$audio_dir")
export NAGI_WHISPER_HOST_BUILD=${NAGI_WHISPER_HOST_BUILD:-"$repo_root/out/whisper-host-build"}
export NAGI_WHISPER_HOST_SOURCE=${NAGI_WHISPER_HOST_SOURCE:-"$repo_root/out/cache/whisper-cpp-host"}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$repo_root/out/m25-whisper-tests-target"}

read -r model_bytes model_sha < <(python3 - "$repo_root/third_party/models.lock" <<'PY'
import sys, tomllib
lock = tomllib.load(open(sys.argv[1], "rb"))["models"]["whisper_small_multilingual"]
print(lock["size_bytes"], lock["sha256"])
PY
)
if [[ "$(stat -c %s "$model" 2>/dev/null || stat -f %z "$model")" != "$model_bytes" ]] \
    || [[ "$(sha256sum "$model" | cut -d' ' -f1)" != "$model_sha" ]]; then
    echo "run-eval: $model does not match models.lock" >&2
    exit 2
fi

table="$(dirname "$output")/eval-set.tsv"
mkdir -p "$(dirname "$output")"
python3 - "$repo_root/tests/m25-whisper/eval/fleurs-ja-validation.json" "$audio_dir" "$table" <<'PY'
import hashlib, json, os, pathlib, sys
manifest, audio, table = json.load(open(sys.argv[1], encoding="utf-8")), pathlib.Path(sys.argv[2]), sys.argv[3]
fixture = os.environ.get("NAGI_M25_WHISPER_FIXTURE_SHA256", "").lower()
lines = []
for clip in manifest["clips"]:
    pcm = audio / f"{clip['id']}.pcm"
    digest = hashlib.sha256(pcm.read_bytes()).hexdigest()
    if digest != clip["pcm_s16le_sha256"]:
        sys.exit(f"{pcm}: SHA-256 mismatch; re-run tools/whisper/fetch-fleurs-ja-eval.py")
    if fixture and digest == fixture:
        sys.exit(f"{clip['id']} is the guest acceptance fixture; fixtures never count as evaluation")
    lines.append(f"{clip['id']}\t{pcm}\t{clip['reference']}\n")
open(table, "w", encoding="utf-8").writelines(lines)
PY

cd "$repo_root/tests/m25-whisper"
nice -n 19 cargo build --locked --offline -j1 --release --features real-engine --bin m25-whisper-eval
nice -n 19 "$CARGO_TARGET_DIR/release/m25-whisper-eval" "$table" "$model" "$model_bytes" "$output"
