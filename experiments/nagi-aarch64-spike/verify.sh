#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")"
cargo fmt -- --check
cargo clippy --release --locked --offline -- -D warnings
./build.sh
python3 -m unittest test_runner -v
python3 run.py --output out/el1
code=0
python3 run.py --virtualization on --output out/el2-negative || code=$?
[ "$code" -eq 1 ]
python3 - <<'PYTHON'
import json
from pathlib import Path
r = json.loads(Path("out/el2-negative/result.json").read_text())
assert r["returncode"] == 1 and not r["timed_out"]
s = Path("out/el2-negative/serial.log").read_bytes()
assert b"SPIKE FAIL exception-level" in s and b"COMPLETE PASS" not in s
print("PASS negative guest: rejects EL2 without success marker")
PYTHON
