#!/usr/bin/env python3
"""Check a proposed composition; never confer ownership or runtime acceptance."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

DOC = "docs/porting/multiarch/"
MANIFEST = DOC + "source-records-composition.json"
EXTRA = {
    MANIFEST, DOC + "verify-integration.py", DOC + "test-integration-records.py",
    DOC + "source-records-integration.md",
}
PREFIXES = (DOC, "experiments/nagi-multiarch-contracts/", "experiments/nagi-aarch64-spike/")
STATUS = "docs/implementation_status.md"
SHA = re.compile(r"[0-9a-f]{40}")


class IntegrationError(ValueError):
    pass


def git(root, *args, data=None):
    result = subprocess.run(["git", *args], cwd=root, input=data, capture_output=True)
    if result.returncode:
        raise IntegrationError(result.stderr.decode(errors="replace").strip())
    return result.stdout


def text(root, *args):
    return git(root, *args).decode().strip()


def require(condition, message):
    if not condition:
        raise IntegrationError(message)


def commit(root, value):
    require(isinstance(value, str) and SHA.fullmatch(value), "explicit full commit SHA required")
    require(text(root, "rev-parse", value + "^{commit}") == value, "commit identity mismatch")
    return value


def tree(root, revision):
    entries = {}
    for entry in git(root, "ls-tree", "-r", "-z", revision).split(b"\0"):
        if entry:
            header, path = entry.split(b"\t", 1)
            mode, kind, oid = header.decode().split()
            entries[path.decode()] = (mode, kind, oid)
    return entries


def blob(root, revision, path):
    return git(root, "show", revision + ":" + path)


def json_blob(root, revision, path):
    return json.loads(blob(root, revision, path))


def anchored(root, value, expected):
    require(isinstance(value, dict) and set(value) == {"commit", "tree"}, "anchor fields invalid")
    require(value["commit"] == expected, "manifest differs from explicit CLI anchor")
    commit(root, expected)
    require(value["tree"] == text(root, "rev-parse", expected + "^{tree}"), "anchor tree mismatch")


def validate(root, base, source, records):
    root = Path(root).resolve()
    for revision in (base, source, records):
        commit(root, revision)
    require(not text(root, "status", "--porcelain", "--untracked-files=all"), "worktree must be clean")
    head = text(root, "rev-parse", "HEAD")
    manifest = json_blob(root, head, MANIFEST)
    require(set(manifest) == {"schema", "base", "current_source", "historical_records",
                             "historical_runtime", "root_status", "acceptance"}, "manifest fields invalid")
    require(manifest["schema"] == "nagi-multiarch-source-record-composition/1", "unsupported composition schema")
    for key, expected in (("base", base), ("current_source", source), ("historical_records", records)):
        anchored(root, manifest[key], expected)
    for child in (source, records):
        require(text(root, "merge-base", base, child) == base, "base must precede both inputs")
    require(text(root, "merge-base", source, head) == source, "current source must precede composition")
    require(source != records, "current source and historical records must be distinct")
    require(manifest["acceptance"] == {
        "current_source_runtime": "NOT_RUN", "phase_c": "PENDING", "physical": "NOT_RUN",
        "owner_scope_adoption": "PENDING", "scope_owner": "Integration Owner/os-core",
    }, "composition cannot promote acceptance or claim ownership")

    original, current, historical, actual = (tree(root, rev) for rev in (base, source, records, head))
    def changed(left, right):
        return {p for p in left.keys() | right.keys() if left.get(p) != right.get(p)}
    source_paths, record_paths = changed(original, current), changed(original, historical)
    require(source_paths and record_paths, "both input deltas are required")
    require(not source_paths & record_paths, "overlapping input paths require a separately reviewed composition")
    require(all(p.startswith(PREFIXES) or p == STATUS for p in source_paths), "source changes outside bounded scope")
    require(all(p.startswith(DOC) for p in record_paths), "historical records changed non-record paths")
    require(not EXTRA & (source_paths | record_paths), "composition metadata overlaps input deltas")
    protected = {p for p in original if p.endswith(".schema.json")}
    protected.add(DOC + "verify-records.py")
    require(not protected & (source_paths | record_paths), "existing schema/verifier changes are prohibited")

    status = manifest["root_status"]
    require(isinstance(status, dict) and set(status) == {"path", "base_sha256", "source_sha256", "change_kind"}, "root status fields invalid")
    require(status["path"] == STATUS and status["change_kind"] == "prepend-only-exact-reviewed-source", "root status scope invalid")
    before, after = blob(root, base, STATUS), blob(root, source, STATUS)
    require(bool(before) and len(after) > len(before) and after.endswith(before), "root status rewrites existing content")
    require(after.startswith(b"## PR #57 independent ARM64 host boundary candidate"), "root status lacks bounded PR57 prefix")
    require(status["base_sha256"] == hashlib.sha256(before).hexdigest()
            and status["source_sha256"] == hashlib.sha256(after).hexdigest(), "root status digest mismatch")
    require(actual.get(STATUS) == current.get(STATUS), "root status differs from reviewed source")

    expected = dict(current)
    for path in record_paths:
        if path in historical:
            expected[path] = historical[path]
        else:
            expected.pop(path, None)
    require(EXTRA <= actual.keys(), "composition metadata missing")
    for path in EXTRA:
        require(path not in expected, "composition metadata already exists in source input")
        require(actual[path][0:2] == ("100644", "blob"), "composition metadata must be regular files")
    require({p: entry for p, entry in actual.items() if p not in EXTRA} == expected,
            "composition differs from exact source/records overlay (bytes, modes or file set)")

    runtime = manifest["historical_runtime"]
    require(isinstance(runtime, dict) and set(runtime) == {"source", "main", "spike_tree", "summaries"}, "historical runtime fields invalid")
    old = runtime["source"]
    require(isinstance(old, dict) and "commit" in old, "historical source anchor invalid")
    anchored(root, old, commit(root, old["commit"]))
    require(old["commit"] != source, "historical runtime must not be relabeled as current source")
    require(text(root, "merge-base", old["commit"], base) == old["commit"], "historical source must precede base")
    main = commit(root, runtime["main"])
    require(text(root, "merge-base", main, old["commit"]) == main, "historical main must precede historical source")
    require(runtime["spike_tree"] == text(root, "rev-parse", old["commit"] + ":experiments/nagi-aarch64-spike"), "historical spike subtree mismatch")
    paths = runtime["summaries"]
    require(isinstance(paths, list) and len(paths) >= 2 and all(isinstance(p, str) for p in paths)
            and paths == sorted(set(paths)), "historical summary list invalid")
    state = json_blob(root, records, DOC + "state.json")
    require(state["last_verified"]["commit_sha"] == old["commit"], "historical state source mismatch")
    for path in paths:
        require(path.startswith(DOC + "runtime/") and Path(path).name == "summary.json"
                and ".." not in Path(path).parts, "historical summary path invalid")
        summary = json_blob(root, records, path)
        require(summary["source_commit"] == old["commit"] and summary["source_tree"] == old["tree"]
                and summary["main"] == main and summary["source_spike_subtree"] == runtime["spike_tree"],
                "historical summary identity mismatch")
        require(summary["acceptance"]["phase_c"] == "PENDING", "historical phase promotion rejected")

    # Execute the original verifier from its own immutable records checkout.
    # Never redirect it to the composition or weaken its records-only contract.
    with tempfile.TemporaryDirectory(prefix="pr57-history-") as temporary:
        checkout = Path(temporary) / "snapshot"
        git(root, "worktree", "add", "--detach", str(checkout), records)
        try:
            result = subprocess.run([
                sys.executable, "-B", str(checkout / (DOC + "verify-records.py")),
                "--main", main, "--evidence-dir", DOC + "evidence/main-" + main,
            ], cwd=checkout, capture_output=True, text=True)
            require(result.returncode == 0, "historical records verifier rejected fixed snapshot: " + result.stderr.strip())
        finally:
            git(root, "worktree", "remove", str(checkout))
    return {"schema": manifest["schema"], "integrity": "PASS", "head": head,
            "tree": text(root, "rev-parse", head + "^{tree}"), "source": source,
            "records": records, "runtime_source": old["commit"],
            "root_status": "EXACT_REVIEWED_PREPEND", "historical_verifier": "PASS",
            **manifest["acceptance"]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--records", required=True)
    args = parser.parse_args(argv)
    try:
        result = validate(Path(__file__).resolve().parents[3], args.base, args.source, args.records)
    except (IntegrationError, OSError, ValueError, KeyError, TypeError) as exc:
        parser.exit(2, f"composition rejected: {exc}\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
