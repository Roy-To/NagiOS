#!/usr/bin/env bash
# Reproducible fetch + checksum for the M25 local TTS artifacts.
# Pins: tools/tts/tts-artifacts.lock. Output: ignored cache directory
# (default out/cache/tts relative to the repository root).
#
#   tools/tts/fetch.sh [CACHE_DIR]
#
# Re-running is idempotent: verified files are kept, mismatches are
# deleted and reported as errors (exit 1). Nothing is written to git.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CACHE="${1:-${NAGI_TTS_CACHE:-$ROOT/out/cache/tts}}"
mkdir -p "$CACHE"

VOICE_REV=8e3306021db135c265f5eda5f062dc489707ddf8
VOICE_URL="https://raw.githubusercontent.com/icn-lab/htsvoice-tohoku-f01/$VOICE_REV"
DICT_URL="https://github.com/jpreprocess/jpreprocess/releases/download/v0.15.0/naist-jdic-jpreprocess.tar.gz"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

fetch() { # url file sha256 size
  local url="$1" file="$CACHE/$2" want="$3" size="$4"
  if [ -f "$file" ] && [ "$(sha256 "$file")" = "$want" ]; then
    echo "OK    $2 (cached)"; return 0
  fi
  rm -f "$file.part"
  curl -fsSL --retry 3 -o "$file.part" "$url"
  local got_size got
  got_size=$(wc -c <"$file.part" | tr -d ' ')
  got=$(sha256 "$file.part")
  if [ "$got_size" != "$size" ] || [ "$got" != "$want" ]; then
    rm -f "$file.part"
    echo "FAIL  $2: size $got_size sha256 $got (want $size $want)" >&2
    return 1
  fi
  mv "$file.part" "$file"
  echo "OK    $2 (downloaded)"
}

fetch "$VOICE_URL/tohoku-f01-neutral.htsvoice" tohoku-f01-neutral.htsvoice \
  ded6acb4243b93cc50896199d67b00a7c487662bfcdcd3f9f30a4fb0444b0b46 2154716
fetch "$VOICE_URL/COPYRIGHT.txt" tohoku-f01-COPYRIGHT.txt \
  f7f7a1a696f062704825fcc495d330beeeac8c0ed38d8f5bdd8e90bdc3471c95 1765
fetch "$DICT_URL" naist-jdic-jpreprocess.tar.gz \
  8a930bbc57bf4adcf521d53544c7dc9ab8ab3aa997a591b1b1608dc5539017b8 28668638

rm -rf "$CACHE/naist-jdic.tmp"
mkdir -p "$CACHE/naist-jdic.tmp"
tar -xzf "$CACHE/naist-jdic-jpreprocess.tar.gz" -C "$CACHE/naist-jdic.tmp"
status=0
while read -r want name; do
  got=$(sha256 "$CACHE/naist-jdic.tmp/naist-jdic/$name")
  if [ "$got" != "$want" ]; then echo "FAIL  naist-jdic/$name $got" >&2; status=1; fi
done <<'EOF'
cd3dbb6fa448906590c2e5ef2600378e4bd074c4d1d6fc5c049d4fd8c2d2d556 char_def.bin
26f14fe3affdb0966e1696ae6cd4b7faac91bf85b7ca57a5d8cdc045da58d043 dict.da
7fb308e1909be247779be0a4c81070416635c4ff0ab266d48a5e964baffd345f dict.vals
7ef63438285a3dd50299aa826f3d62d9657a907184e055a179af3e00eae820aa dict.words
67b30a728ab8816def37d9e2775d99dc98bf8eaa6f74a821dd3ee2fd009e8b61 dict.wordsidx
a64b58ac89b2ca21f3ae6c56ca772f8d4488cc636f47b821b4cca9c0c67c95d2 matrix.mtx
8fa56e3d131c9d7cabc3633bb7c2add979469a1e22be90ecd714e81df20a5dd4 metadata.json
1ff6b44ab3145cd8ac731b9cb0500804feae5fa257aee8bc78a65a8dc80bb454 unk.bin
EOF
if [ "$status" -ne 0 ]; then rm -rf "$CACHE/naist-jdic.tmp"; exit 1; fi
rm -rf "$CACHE/naist-jdic"
mv "$CACHE/naist-jdic.tmp/naist-jdic" "$CACHE/naist-jdic"
rmdir "$CACHE/naist-jdic.tmp"
echo "OK    naist-jdic/ (8 files verified)"
echo "NAGI_TTS_VOICE=$CACHE/tohoku-f01-neutral.htsvoice"
echo "NAGI_TTS_DICT=$CACHE/naist-jdic"
