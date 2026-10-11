#!/usr/bin/env python3
"""Validate provisional records and immutable production scope; no writes."""
import hashlib
import json
from pathlib import Path
import re
import subprocess

import jsonschema

ROOT = Path(__file__).resolve().parents[3]
DOC = Path(__file__).resolve().parent
BASE = "379d19eadb6f8792c27b9bc3f147e1ea48eac4a2"
PREFIXES = ("docs/porting/multiarch/", "experiments/nagi-multiarch-contracts/",
            "experiments/nagi-aarch64-spike/")


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).splitlines()


def main():
    paths = set(git("diff", "--name-only", BASE))
    paths.update(git("ls-files", "--others", "--exclude-standard"))
    assert all(path.startswith(PREFIXES) for path in paths), sorted(paths)
    inventory = json.loads((DOC / "evidence/source-inventory.json").read_text())
    assert inventory["baseline_sha"] == BASE
    for source in inventory["files"]:
        data = (ROOT / source["path"]).read_bytes()
        assert hashlib.sha256(data).hexdigest() == source["sha256"], source["path"]
        # Demonstrate exact identity to the selected immutable baseline, not just local hashes.
        baseline = subprocess.check_output(["git", "show", f"{BASE}:{source['path']}"], cwd=ROOT)
        assert data == baseline, source["path"]
    for path, digest in json.loads((DOC / "evidence/authority-sources.json").read_text()).items():
        assert hashlib.sha256((ROOT / path).read_bytes()).hexdigest() == digest, path
    proposal = json.loads((DOC / "registration-proposal.json").read_text())
    assert proposal["registration_status"] == "PROPOSED_UNREGISTERED"
    registry = json.loads((ROOT / ".dev/workstreams.json").read_text())
    assert all(w["id"] != "PORT-MULTIARCH-01" for w in registry["workstreams"])
    if (DOC / "state.json").exists():
        schema = json.loads((ROOT / ".dev/schemas/workstream-state.schema.json").read_text())
        jsonschema.Draft202012Validator(schema, format_checker=jsonschema.FormatChecker()).validate(
            json.loads((DOC / "state.json").read_text()))
    count = 0
    for directory in [DOC, ROOT / "experiments/nagi-multiarch-contracts", ROOT / "experiments/nagi-aarch64-spike"]:
        for path in directory.glob("*.md"):
            for link in re.findall(r"\[[^\]]*\]\(([^)]+)\)", path.read_text()):
                if re.match(r"https?://", link) or link.startswith("#"):
                    continue
                assert (path.parent / link.split("#", 1)[0]).exists(), (path, link)
                count += 1
    for path in [ROOT / "experiments/nagi-multiarch-contracts/audit.py", ROOT / "experiments/nagi-aarch64-spike/run.py", ROOT / "experiments/nagi-aarch64-spike/test_runner.py", Path(__file__)]:
        compile(path.read_text(), str(path), "exec")
    subprocess.run(["git", "diff", "--check"], cwd=ROOT, check=True)
    print(f"PASS owned scope, {len(inventory['files'])} unchanged baseline sources, authority hashes, provisional schema, {count} local links and Python syntax")


if __name__ == "__main__":
    main()
