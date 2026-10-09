#!/usr/bin/env python3
"""Read-only source preservation, schema and ownership audit for this checkpoint."""
import fnmatch
import hashlib
import json
from pathlib import Path
import subprocess

import jsonschema
import yaml

ROOT = Path(__file__).resolve().parents[3]
SOURCES = {
    "writer": "8cc5c5da04f622be95c79152bf709edb61272c59",
    "sheets": "b6e6efe13018cae4f3de188bb387e0584994609f",
}


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT)


BASE = git("merge-base", "origin/main", "HEAD").decode().strip()

registry = json.loads((ROOT / ".dev/workstreams.json").read_text())
original = json.loads(git("show", f"{BASE}:.dev/workstreams.json"))
assert registry["workstreams"][:-1] == original["workstreams"]
row = registry["workstreams"][-1]
assert row["id"] == "writer-sheets-core-adoption"
jsonschema.Draft202012Validator(
    json.loads((ROOT / ".dev/schemas/workstreams.schema.json").read_text())
).validate(registry)
schema = json.loads((ROOT / ".dev/schemas/workstream-state.schema.json").read_text())
states = list((ROOT / ".dev/workstreams").glob("*/state.json"))
for path in states:
    jsonschema.Draft202012Validator(schema).validate(json.loads(path.read_text()))

immutable = []
for core, ref in SOURCES.items():
    owner = "writer-core-01" if core == "writer" else "sheets-calc-01"
    prefixes = [f"crates/nagi-{core}-core", f"tests/{core}-core", f".dev/workstreams/{owner}"]
    names = git("ls-tree", "-r", "--name-only", ref, "--", *prefixes).decode().splitlines()
    mutable = {
        "crates/nagi-writer-core/Cargo.toml",
        "crates/nagi-writer-core/Cargo.lock",
        "crates/nagi-writer-core/README.md",
        "tests/writer-core/verify.sh",
    }
    for name in names:
        if name in mutable:
            continue
        data = (ROOT / name).read_bytes()
        assert data == git("show", f"{ref}:{name}"), f"Source changed: {name}"
        immutable.append({"path": name, "source_sha": ref, "sha256": hashlib.sha256(data).hexdigest()})
    local = {str(p.relative_to(ROOT)) for prefix in prefixes for p in (ROOT / prefix).rglob("*") if p.is_file()}
    assert local == set(names), f"Unexpected/missing {core} source files"

base_states = git("ls-tree", "-r", "--name-only", BASE, "--", ".dev/workstreams").decode().splitlines()
for name in base_states:
    assert (ROOT / name).read_bytes() == git("show", f"{BASE}:{name}"), f"Other owner evidence changed: {name}"

# Includes staged, unstaged and untracked paths before the first commit as well.
changed = set(git("diff", "--name-only", BASE).decode().splitlines())
changed.update(git("ls-files", "--others", "--exclude-standard").decode().splitlines())
for name in changed:
    assert any(fnmatch.fnmatchcase(name, g) for g in row["allowed_paths"]), name
    assert not any(fnmatch.fnmatchcase(name, g) for g in row["forbidden_paths"]), name
workflow = yaml.safe_load((ROOT / ".github/workflows/0.2-host-integration.yml").read_text())
steps = workflow["jobs"]["host-foundations"]["steps"]
app_step = next(s["run"] for s in steps if s.get("name", "").startswith("Owned app host tests"))
assert 'cargo fmt --manifest-path "$manifest" -- --check' in app_step
assert 'cargo fmt --manifest-path "$manifest" --all' not in app_step
assert '--package nagi-calendar-core' in app_step
assert '--all-targets --locked -- -D warnings' in app_step
assert '--locked --offline' in app_step
print(json.dumps({"result": "PASS", "base_sha": BASE, "registry_streams": len(registry["workstreams"]),
                  "state_files": len(states), "changed_paths": sorted(changed), "immutable_files": immutable}, indent=2))
