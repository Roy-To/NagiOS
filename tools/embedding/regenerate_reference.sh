#!/usr/bin/env bash
# Regenerate the verification-only reference data for the M24 provider in a
# fresh, hash-pinned Python environment and compare it with the committed copies.
#
#   tests/m24-embedding/parity_reference.json  (onnxruntime + tokenizers vectors)
#   tests/m24-embedding/tokenizer_parity.json  (tokenizers token ids)
#
# Inputs: pinned upstream files incl. onnx/model.onnx (fetch.sh --with-reference).
# Python: CPython 3.12 with packages from requirements-reference.txt installed
#         with --require-hashes --only-binary :all: (no source builds).
# Exit status is non-zero if installation, generation, or the comparison fails.
#
# Usage: tools/embedding/regenerate_reference.sh [--write]
#   --write  replace the committed files with the regenerated ones (otherwise
#            the regenerated files stay in the work directory and are compared).
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
write=0
[[ "${1:-}" == "--write" ]] && write=1
python=${PYTHON:-python3.12}
cache=${NAGI_EMBEDDING_CACHE:-$HOME/.cache/nagi-embedding}
work=${NAGI_M24_REGEN_DIR:-$(mktemp -d)}
log_dir=${NAGI_M24_EVIDENCE_DIR:-$cache/evidence}
mkdir -p "$log_dir" "$work"
log="$log_dir/m24-reference-regeneration-$(date -u +%Y%m%dT%H%M%SZ).log"
exec > >(tee "$log") 2>&1

echo "regen: repo commit $(git -C "$repo" rev-parse HEAD)"
echo "regen: host $(uname -sm); python $("$python" -c 'import sys;print(sys.version.split()[0])')"
echo "regen: requirements sha256 $(sha256sum "$here/requirements-reference.txt" | cut -d' ' -f1)"
input=$("$here/fetch.sh" --with-reference | tail -n 1)

"$python" -m venv "$work/venv"
"$work/venv/bin/python" -m pip install --quiet --disable-pip-version-check --no-cache-dir \
    --require-hashes --only-binary :all: -r "$here/requirements-reference.txt"
"$work/venv/bin/python" -m pip check
echo "regen: installed (pip freeze):"
"$work/venv/bin/python" -m pip freeze --all | sed 's/^/  /'

"$work/venv/bin/python" "$here/reference_e5.py" --input "$input" \
    --corpus "$repo/tests/m24-embedding/parity_corpus.json" \
    --output "$work/parity_reference.json"
"$work/venv/bin/python" "$here/tokenizer_parity.py" --input "$input" --repo "$repo" \
    --output "$work/tokenizer_parity.json"

status=0
for name in parity_reference.json tokenizer_parity.json; do
    new=$(sha256sum "$work/$name" | cut -d' ' -f1)
    old=$(sha256sum "$repo/tests/m24-embedding/$name" | cut -d' ' -f1)
    if [[ "$new" == "$old" ]]; then
        echo "regen: $name byte-identical to committed copy (sha256 $new)"
    else
        echo "regen: $name DIFFERS from committed copy (new $new, committed $old)"
        status=1
    fi
    [[ $write == 1 ]] && cp "$work/$name" "$repo/tests/m24-embedding/$name"
done
if [[ $status == 0 ]]; then
    echo "regen: PASS (reference data reproduced from pinned inputs and hash-pinned packages)"
else
    echo "regen: FAIL (reference data not reproduced; inspect $work)"
fi
exit $status
