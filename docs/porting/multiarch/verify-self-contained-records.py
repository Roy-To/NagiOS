#!/usr/bin/env python3
"""Check packaging and run unchanged validators in their exact source snapshots."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile

DOC = "docs/porting/multiarch/"
SPEC = importlib.util.spec_from_file_location("restore_objects", Path(__file__).with_name("restore-review-objects.py"))
objects = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(objects)
SIDE_FILES = {DOC + name for name in ("source-records-composition.json", "source-records-integration.md",
                                     "verify-integration.py", "test-integration-records.py")}
PACKAGING = {DOC + name for name in ("review-dependencies.bundle", "review-dependencies.json",
                                    "restore-review-objects.py", "verify-self-contained-records.py",
                                    "test-self-contained-records.py", "self-contained-records.md",
                                    "proposed-records-ci.yml")}
BASE = "b50e8ff181efcf5093c928472fc729d5c9e231cd"
MAIN = "0362427747e769b00558053e4c7f4c756d303802"
SOURCE = "cc52cadcf0ef7c8b6c0683b1a9be3a85ef4d7e4e"
OLD_SOURCE = "e6c2c08860df75c84a85c31151e8c32b2613433b"


def entries(root, commit):
    result = {}
    for line in subprocess.check_output(["git", "ls-tree", "-rz", commit], cwd=root).split(b"\0"):
        if line:
            header, path = line.split(b"\t", 1)
            result[path.decode()] = header
    return result


def check_tree(actual, current, previous):
    if not SIDE_FILES <= previous.keys() or SIDE_FILES & current.keys():
        raise ValueError("prior-side file set is not the reviewed missing set")
    expected = dict(current)
    expected.update({path: previous[path] for path in SIDE_FILES})
    if not PACKAGING <= actual.keys() or PACKAGING & expected.keys():
        raise ValueError("packaging file set missing or overlaps an input")
    if any(not actual[path].startswith(b"100644 blob ") for path in PACKAGING):
        raise ValueError("packaging files must be regular nonexecutable files")
    if {path: value for path, value in actual.items() if path not in PACKAGING} != expected:
        raise ValueError("packaging changed reviewed source, 482 files, records or file modes")


def snapshot(root, commit, script, args):
    with tempfile.TemporaryDirectory(prefix="pr57-fixed-snapshot-") as temporary:
        checkout = Path(temporary) / "source"
        objects.git(root, "worktree", "add", "--detach", str(checkout), commit)
        try:
            result = subprocess.run([sys.executable, "-B", str(checkout / script), *args], cwd=checkout,
                                    capture_output=True, text=True)
            if result.returncode:
                raise ValueError("fixed snapshot validation failed: " + result.stderr.strip())
            return json.loads(result.stdout)
        finally:
            objects.git(root, "worktree", "remove", str(checkout))


def verify(root, regressions=False):
    root = Path(root).resolve()
    if objects.git(root, "status", "--porcelain", "--untracked-files=all"):
        raise ValueError("packaging checkout must be clean")
    anchors = objects.restore(root)
    head = objects.git(root, "rev-parse", "HEAD")
    current, prior, history = (anchors[k] for k in ("current_records", "prior_composition", "historical_records"))
    objects.git(root, "merge-base", "--is-ancestor", current, head)
    check_tree(entries(root, head), entries(root, current), entries(root, prior))
    current_result = snapshot(root, current, DOC + "verify-additional-runtime.py", [
        "--base", BASE, "--source", SOURCE, "--records", history, "--main", MAIN])
    prior_result = snapshot(root, prior, DOC + "verify-integration.py", [
        "--base", BASE, "--source", OLD_SOURCE, "--records", history])
    if (current_result.get("record_completeness") != "COMPLETE_METADATA_ONLY"
            or current_result.get("manual_formal_receipt") != "NOT_GENERATED"
            or current_result.get("phase_c") != "PENDING"
            or prior_result.get("owner_scope_adoption") != "PENDING"
            or prior_result.get("phase_c") != "PENDING"):
        raise ValueError("snapshot acceptance boundary changed")
    if regressions:
        scripts = [DOC + "test-self-contained-records.py", DOC + "test-integration-records.py",
                   DOC + "test-additional-runtime.py", DOC + "test-attestation-source-scope.py",
                   DOC + "test-runtime-records.py",
                   DOC + "runtime/arm64-0039f0c3-independent-cloud-20261104/test_check_backend.py"]
        for script in scripts:
            subprocess.run([sys.executable, "-B", str(root / script), "-v"], cwd=root,
                           stdout=sys.stderr, check=True)
        subprocess.run([sys.executable, "-B", "-m", "unittest", "test_runner", "test_attestation", "-v"],
                       cwd=root / "experiments/nagi-aarch64-spike", stdout=sys.stderr, check=True)
    return {"packaging_integrity": "PASS", "head": head, "offline_dependencies": "RESTORED_EXACTLY_NO_REF_UPDATES",
            "current_records_snapshot": current, "prior_482_snapshot": prior, "history_snapshot": history,
            "prior_482_files": "PRESERVED_BYTE_AND_MODE_EXACT", "current_record": current_result["record_completeness"],
            "manual_formal_receipt": "NOT_GENERATED", "phase_c": "PENDING", "physical": "NOT_RUN",
            "owner_adoption": "PENDING", "regressions_requested": regressions}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--regressions", action="store_true")
    args = parser.parse_args()
    try:
        result = verify(Path(__file__).resolve().parents[3], args.regressions)
    except (ValueError, OSError, subprocess.CalledProcessError) as exc:
        parser.exit(2, f"self-contained records rejected: {exc}\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
