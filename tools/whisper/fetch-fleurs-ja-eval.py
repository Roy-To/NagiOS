#!/usr/bin/env python3
"""Fetch the M25 Whisper unseen Japanese evaluation clips (FLEURS ja_jp).

Audio is CC BY 4.0 (google/fleurs) and is not committed. This script fetches
each row listed in tests/m25-whisper/eval/fleurs-ja-validation.json from the
Hugging Face datasets-server, verifies the source WAV SHA-256, converts IEEE
float32 mono 16 kHz WAV to little-endian int16 PCM, verifies the PCM SHA-256,
and writes `<id>.pcm` to the output directory (default
out/m25-whisper-eval/fleurs). Any hash mismatch is a hard failure.

Usage: tools/whisper/fetch-fleurs-ja-eval.py [--output DIR]
"""

import argparse
import hashlib
import json
import math
import pathlib
import struct
import sys
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "tests/m25-whisper/eval/fleurs-ja-validation.json"
ROWS_API = "https://datasets-server.huggingface.co/rows"


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "nagi-m25-whisper-eval"})
    with urllib.request.urlopen(request, timeout=120) as response:
        return response.read()


def wav_float32_to_pcm16(wav: bytes) -> bytes:
    if wav[:4] != b"RIFF" or wav[8:12] != b"WAVE":
        raise ValueError("not a RIFF/WAVE file")
    offset, fmt, data = 12, None, None
    while offset + 8 <= len(wav):
        chunk_id = wav[offset : offset + 4]
        size = struct.unpack("<I", wav[offset + 4 : offset + 8])[0]
        body = wav[offset + 8 : offset + 8 + size]
        if chunk_id == b"fmt ":
            fmt = struct.unpack("<HHIIHH", body[:16])
        elif chunk_id == b"data":
            data = body
        offset += 8 + size + (size & 1)
    if fmt != (3, 1, 16000, 64000, 4, 32) or data is None or len(data) % 4:
        raise ValueError(f"unexpected WAV format {fmt}")
    samples = struct.unpack(f"<{len(data) // 4}f", data)
    pcm = (max(-32768, min(32767, math.floor(x * 32768.0 + 0.5))) for x in samples)
    return struct.pack(f"<{len(samples)}h", *pcm)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", default=str(ROOT / "out/m25-whisper-eval/fleurs"))
    args = parser.parse_args()
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    dataset = manifest["dataset"]
    output = pathlib.Path(args.output)
    output.mkdir(parents=True, exist_ok=True)

    wanted = {clip["row_idx"]: clip for clip in manifest["clips"]}
    length = max(wanted) + 1
    query = urllib.parse.urlencode(
        {
            "dataset": "google/fleurs",
            "config": dataset["config"],
            "split": dataset["split"],
            "offset": 0,
            "length": length,
        }
    )
    rows = json.loads(fetch(f"{ROWS_API}?{query}"))["rows"]
    failures = 0
    for entry in rows:
        clip = wanted.get(entry["row_idx"])
        if clip is None:
            continue
        row = entry["row"]
        if row["raw_transcription"] != clip["reference"]:
            print(f"{clip['id']}: reference text changed upstream", file=sys.stderr)
            failures += 1
            continue
        wav = fetch(row["audio"][0]["src"])
        if hashlib.sha256(wav).hexdigest() != clip["source_wav_sha256"]:
            print(f"{clip['id']}: source WAV hash mismatch", file=sys.stderr)
            failures += 1
            continue
        pcm = wav_float32_to_pcm16(wav)
        if hashlib.sha256(pcm).hexdigest() != clip["pcm_s16le_sha256"]:
            print(f"{clip['id']}: converted PCM hash mismatch", file=sys.stderr)
            failures += 1
            continue
        (output / f"{clip['id']}.pcm").write_bytes(pcm)
        print(f"{clip['id']}: ok ({len(pcm) // 2} samples)")
        del wanted[entry["row_idx"]]
    for clip in wanted.values():
        print(f"{clip['id']}: row not returned by datasets-server", file=sys.stderr)
        failures += 1
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
