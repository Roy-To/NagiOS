#!/usr/bin/env bash
# Download the pinned multilingual-e5-small inputs at an immutable revision and
# verify size + SHA-256 of every file. No credentials, no paid services.
#
# Usage: tools/embedding/fetch.sh [--with-reference]
# Cache:  ${NAGI_EMBEDDING_CACHE:-$HOME/.cache/nagi-embedding}/<revision>/
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
manifest="$here/manifest.toml"
with_reference=0
[[ "${1:-}" == "--with-reference" ]] && with_reference=1

python3 - "$manifest" "$with_reference" <<'PY'
import hashlib, os, sys, tomllib, urllib.request, shutil

manifest_path, with_reference = sys.argv[1], sys.argv[2] == "1"
with open(manifest_path, "rb") as f:
    m = tomllib.load(f)
model = m["model"]
repo, rev = model["repository"], model["revision"]
if len(rev) != 40 or any(c not in "0123456789abcdef" for c in rev):
    sys.exit(f"fetch: revision is not an immutable commit hash: {rev}")
cache = os.environ.get("NAGI_EMBEDDING_CACHE") or os.path.expanduser("~/.cache/nagi-embedding")
dest_root = os.path.join(cache, rev)
files = list(model["files"])
if with_reference:
    files.append(m["reference"])

def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()

for entry in files:
    name, size, sha = entry["name"], entry["size_bytes"], entry["sha256"]
    dest = os.path.join(dest_root, name)
    if os.path.exists(dest) and os.path.getsize(dest) == size and digest(dest) == sha:
        print(f"fetch: ok (cached) {name}")
        continue
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    url = f"{repo}/resolve/{rev}/{name}"
    tmp = dest + ".part"
    print(f"fetch: downloading {url}")
    with urllib.request.urlopen(url, timeout=120) as r, open(tmp, "wb") as out:
        shutil.copyfileobj(r, out, 1 << 20)
    got_size, got_sha = os.path.getsize(tmp), digest(tmp)
    if got_size != size or got_sha != sha:
        os.remove(tmp)
        sys.exit(f"fetch: integrity mismatch for {name}: size {got_size} sha256 {got_sha}")
    os.replace(tmp, dest)
    print(f"fetch: ok {name} {sha}")
print(dest_root)
PY
