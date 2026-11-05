#!/usr/bin/env python3
"""Validate supplemental public metadata; never claim raw receipt revalidation."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[3]
DOC = "docs/porting/multiarch/"
DIRECTORY = DOC + "runtime/arm64-cc52cad-additional-20261105/"
RECORD = DIRECTORY + "record.json"
EXTRAS = {RECORD, DIRECTORY + "README.md", DOC + "verify-additional-runtime.py",
          DOC + "test-additional-runtime.py"}
SPEC = importlib.util.spec_from_file_location("legacy_records", Path(__file__).with_name("verify-records.py"))
legacy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(legacy)
FIELDS = {"source_archive_sha256", "elf_sha256", "el1_serial_sha256", "el2_serial_sha256"}
ACCEPTANCE = {"phase_c": "PENDING", "physical": "NOT_RUN", "production_arm64": "NOT_RUN",
              "manual_formal_receipt": "NOT_GENERATED", "raw_receipt_rechecked_here": False}


def require(value, message):
    if not value:
        raise ValueError(message)


def digest(value):
    require(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value), "invalid SHA-256")
    return value


def tree(root, revision):
    entries = {}
    for raw in legacy.git(root, "ls-tree", "-rz", revision, text=False).split(b"\0"):
        if raw:
            mode_and_blob, path = raw.split(b"\t", 1)
            entries[path.decode()] = mode_and_blob
    return entries


def anchor(root, value, expected):
    require(isinstance(value, dict) and set(value) == {"commit", "tree"}, "anchor fields invalid")
    require(value["commit"] == legacy.full_commit(root, expected, "explicit anchor"), "source/records commit mismatch")
    require(value["tree"] == legacy.git(root, "rev-parse", expected + "^{tree}").strip(), "anchor tree mismatch")


def summary(root, ref, filename, source):
    require(isinstance(ref, dict) and set(ref) == {"path", "sha256"}, "summary reference invalid")
    if ref == {"path": None, "sha256": None}:
        return None
    require(ref["path"] == DIRECTORY + filename, "summary path is not the fixed supplemental path")
    digest(ref["sha256"])
    path = root / ref["path"]
    require(path.is_file() and not path.is_symlink(), "summary file missing or unsafe")
    data = path.read_bytes()
    require(hashlib.sha256(data).hexdigest() == ref["sha256"], "summary digest mismatch")
    value = json.loads(data)
    require(value.get("source") == source, "summary source/main/tree/subtree mismatch")
    return value


def validate(root, base, source, records, expected_main):
    root = Path(root).resolve()
    require(not legacy.git(root, "status", "--porcelain", "--untracked-files=all").strip(), "worktree must be clean")
    head = legacy.git(root, "rev-parse", "HEAD").strip()
    for child in (source, records, head):
        legacy.full_commit(root, child, "input commit")
        legacy.require_ancestor(root, base, child, "explicit public base")
    legacy.require_ancestor(root, source, head, "current source")
    record = json.loads(legacy.commit_bytes(root, head, RECORD))
    require(set(record) == {"schema", "base", "current_source", "historical_records", "runtime_main",
                            "spike_tree", "formal_summary", "manual_summary", "reported_observations", "acceptance"}, "record fields invalid")
    require(record["schema"] == "nagi-multiarch-additional-runtime/1", "unsupported additional-record schema")
    for key, revision in (("base", base), ("current_source", source), ("historical_records", records)):
        anchor(root, record[key], revision)
    main = legacy.full_commit(root, record["runtime_main"], "fixed runtime main")
    require(main == legacy.full_commit(root, expected_main, "explicit runtime main"), "runtime main differs from explicit anchor")
    legacy.require_ancestor(root, main, base, "fixed runtime main")
    require(record["spike_tree"] == legacy.git(root, "rev-parse", source + ":experiments/nagi-aarch64-spike").strip(), "spike subtree mismatch")
    require(record["acceptance"] == ACCEPTANCE, "supplemental record cannot promote acceptance or manual receipt")
    require(record["reported_observations"] == {
        "formal": "OWNER_REPORT_VERIFIER_EXIT_0_AND_33_ARTIFACTS",
        "manual": "OWNER_REPORT_MATCHING_ARCHIVE_ELF_EL1_EL2_NO_FORMAL_RECEIPT",
    }, "reported observations cannot be promoted to verified artifacts")

    a, s, h, actual = (tree(root, c) for c in (base, source, records, head))
    def changes(left, right):
        return {p for p in left.keys() | right.keys() if left.get(p) != right.get(p)}
    sd, hd = changes(a, s), changes(a, h)
    require(not sd & hd, "source and historical deltas overlap")
    require(all(p.startswith(legacy.PREFIXES) for p in sd), "current source contains out-of-scope paths")
    require(all(p.startswith(legacy.PREFIXES) for p in legacy.git(root, "diff", "--name-only", main, source).splitlines()), "current source exceeds fixed-main receipt scope")
    require(all(p.startswith(DOC) for p in hd), "historical snapshot changes non-record paths")
    require(legacy.commit_bytes(root, source, "docs/implementation_status.md") == legacy.commit_bytes(root, main, "docs/implementation_status.md"), "root status differs from fixed main")
    protected = {p for p in a if p.endswith(".schema.json")} | {DOC + "verify-records.py"}
    require(not protected & (sd | hd), "legacy schema/verifier changes prohibited")
    expected = dict(s)
    for path in hd:
        if path in h:
            expected[path] = h[path]
        else:
            expected.pop(path, None)
    require(not EXTRAS & expected.keys(), "supplemental record overwrites an input artifact")
    require(EXTRAS <= actual.keys(), "supplemental metadata missing")

    source_identity = {**record["current_source"], "main": main, "spike_tree": record["spike_tree"]}
    formal = summary(root, record["formal_summary"], "formal-summary.json", source_identity)
    manual = summary(root, record["manual_summary"], "manual-summary.json", source_identity)
    allowed = set(EXTRAS)
    for value, filename in ((formal, "formal-summary.json"), (manual, "manual-summary.json")):
        if value is not None:
            allowed.add(DIRECTORY + filename)
    require(not allowed & expected.keys(), "supplemental files overwrite an input artifact")
    for path in allowed:
        require(actual[path].startswith(b"100644 blob "), "supplemental files must be regular nonexecutable files")
    require({p: v for p, v in actual.items() if p not in allowed} == expected,
            "tracked files differ from exact input overlay; invented serial/unrelated files prohibited")

    if formal is not None:
        require(set(formal) == {"schema", "kind", "source", "receipt_sha256", "verifier_stdout_sha256",
                                "verifier_exit_code", "manifested_files", "artifacts"}, "formal summary fields invalid")
        require(formal["schema"] == "nagi-receipt-verification-summary/1"
                and formal["kind"] == "REPORTED_FORMAL_RECEIPT_VERIFICATION", "formal summary kind invalid")
        require(type(formal["verifier_exit_code"]) is int and formal["verifier_exit_code"] == 0
                and type(formal["manifested_files"]) is int and formal["manifested_files"] == 33, "formal verification not exit 0 / 33 artifacts")
        digest(formal["receipt_sha256"]); digest(formal["verifier_stdout_sha256"])
    if manual is not None:
        require(set(manual) == {"schema", "kind", "source", "formal_receipt_generated", "artifacts"}, "manual summary cannot contain receipt/PASS claims")
        require(manual["schema"] == "nagi-manual-artifact-comparison/1"
                and manual["kind"] == "MANUAL_OBSERVATION_ONLY"
                and manual["formal_receipt_generated"] is False, "manual observation must remain without formal receipt")
    for value in (formal, manual):
        if value is not None:
            require(isinstance(value["artifacts"], dict) and set(value["artifacts"]) == FIELDS, "artifact digest set invalid")
            for item in value["artifacts"].values():
                digest(item)
    if formal is not None and manual is not None:
        require(formal["artifacts"] == manual["artifacts"], "manual/formal artifact comparison mismatch")
    missing = [label for label, value in (("approved formal-summary.json with measured hashes", formal),
                                         ("approved manual-summary.json with measured hashes", manual)) if value is None]
    return {"head": head, "source": source, "historical_records": records,
            "metadata_integrity": "PASS", "record_completeness": "PENDING_INPUT" if missing else "COMPLETE_METADATA_ONLY",
            "formal_receipt": "REPORTED_VERIFIED" if formal is not None else "REPORTED_NOT_BOUND_TO_PUBLIC_SUMMARY",
            "manual_observation": "DIGESTS_MATCH_NO_FORMAL_RECEIPT" if formal is not None and manual is not None else "REPORTED_NOT_FULLY_BOUND",
            "missing": missing, **ACCEPTANCE}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ("base", "source", "records", "main"):
        p.add_argument("--" + name, required=True)
    args = p.parse_args()
    try:
        result = validate(ROOT, args.base, args.source, args.records, args.main)
    except (ValueError, OSError, KeyError, TypeError, legacy.RecordsError) as exc:
        p.exit(2, f"additional record rejected: {exc}\n")
    print(json.dumps(result, indent=2))
    return 2 if result["missing"] else 0


if __name__ == "__main__":
    sys.exit(main())
