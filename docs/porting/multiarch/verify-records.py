#!/usr/bin/env python3
"""Validate current records without rewriting evidence or production sources."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

from datetime import datetime
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[3]
DOC = Path(__file__).resolve().parent
EVIDENCE = DOC / "evidence"
HISTORICAL_BASE = "379d19eadb6f8792c27b9bc3f147e1ea48eac4a2"
PREFIXES = (
    "docs/porting/multiarch/",
    "experiments/nagi-multiarch-contracts/",
    "experiments/nagi-aarch64-spike/",
)
SOURCE_PREFIXES = ("kernel/", "loader/", "user/", "targets/", "tools/", "tests/", ".cargo/", ".github/workflows/", "crates/", "sdk/", "patches/")
SOURCE_SUFFIXES = (".rs", ".sh", ".py", ".toml", ".json", ".ld", ".yml", ".patch")
RECORD_ONLY_PREFIX = "docs/porting/multiarch/"
LOCAL_GUARD_ROOTS = (".codex", ".entire")
AUTHORITY_PATHS = (
    "AGENTS.md",
    "docs/implementation_status.md",
    "docs/Nagi_OS_0.1_Codex_Implementation_Spec.md",
    "docs/Nagi_OS_0.2_Codex_Implementation_Spec.md",
    "docs/Nagi_OS_0.3_Architecture_Roadmap.md",
    "docs/0.2/DEVELOPMENT_ARCHITECTURE.md",
    "docs/0.2/WORKSTREAMS.md",
    "docs/0.2/Nagi_OS_0.2_Parallel_Execution_Rules.md",
    ".dev/workstreams.json",
    ".dev/workstreams/integration-next-phase/state.json",
    "docs/decisions/ADR-0001-hybrid-kernel.md",
    "docs/decisions/0014-unified-device-application-model.md",
    "docs/architecture/unified-device-application-model.md",
)
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")


class RecordsError(ValueError):
    pass


def git(root, *args, check=True, text=True):
    result = subprocess.run(
        ["git", *args], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=text, check=False,
    )
    if check and result.returncode != 0:
        message = result.stderr.decode("utf-8", errors="replace").strip() if not text else result.stderr.strip()
        raise RecordsError(message or f"git {' '.join(args)} failed")
    return result.stdout


def full_commit(root, value, label):
    if not isinstance(value, str) or not FULL_SHA.fullmatch(value):
        raise RecordsError(f"{label} must be an explicit full 40-character lowercase commit SHA")
    resolved = git(root, "rev-parse", "--verify", f"{value}^{{commit}}", check=False).strip()
    if resolved != value:
        raise RecordsError(f"{label} does not resolve exactly to the supplied commit SHA")
    return resolved


def validate_current_main(root, inventory_base, expected_main):
    if inventory_base == HISTORICAL_BASE:
        raise RecordsError("the historical 379d19e baseline cannot be accepted as current evidence")
    baseline = full_commit(root, inventory_base, "inventory baseline_sha")
    main = full_commit(root, expected_main, "--main")
    if baseline != main:
        raise RecordsError("inventory baseline_sha must equal the explicit --main SHA")
    return baseline


def require_ancestor(root, ancestor, descendant, label):
    result = subprocess.run(
        ["git", "merge-base", "--is-ancestor", ancestor, descendant],
        cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
    )
    if result.returncode != 0:
        raise RecordsError(f"{label} is not an ancestor of the current records commit")


def validate_owned_scope(root, baseline, head):
    paths = sorted(set(git(root, "diff", "--name-only", baseline, head).splitlines()))
    outside = [path for path in paths if not path.startswith(PREFIXES)]
    if outside:
        raise RecordsError("committed changes outside the owned prefixes: " + ", ".join(outside))
    return paths


def classify_untracked(root):
    raw = git(root, "ls-files", "--others", "--exclude-standard").splitlines()
    paths = sorted(path for path in raw if path)
    guard = [path for path in paths if any(path == name or path.startswith(name + "/") for name in LOCAL_GUARD_ROOTS)]
    unexpected = sorted(set(paths) - set(guard))
    return guard, unexpected


def validate_clean_record_worktree(root):
    unstaged = git(root, "diff", "--name-only", "HEAD").splitlines()
    staged = git(root, "diff", "--cached", "--name-only", "HEAD").splitlines()
    guard, unexpected = classify_untracked(root)
    if unstaged or staged:
        raise RecordsError("tracked worktree is dirty")
    if unexpected:
        raise RecordsError("unexpected untracked paths: " + ", ".join(unexpected))
    return {
        "tracked_clean": True,
        "local_guard_untracked_roots": sorted({path.split("/", 1)[0] for path in guard}),
        "local_guard_untracked_count": len(guard),
    }


def validate_record_commit(root, source_commit, head):
    require_ancestor(root, source_commit, head, "verified source commit")
    changed = sorted(set(git(root, "diff", "--name-only", source_commit, head).splitlines()))
    outside = [path for path in changed if not path.startswith(RECORD_ONLY_PREFIX)]
    if outside:
        raise RecordsError("records-only child changed non-record paths: " + ", ".join(outside))
    changed_experiments = git(
        root, "diff", "--name-only", source_commit, head, "--",
        "experiments/nagi-aarch64-spike", "experiments/nagi-multiarch-contracts",
    ).splitlines()
    if changed_experiments:
        raise RecordsError("record-only child changed experiment source: " + ", ".join(changed_experiments))
    return changed


def commit_bytes(root, commit, path):
    return git(root, "show", f"{commit}:{path}", text=False)


def expected_source_paths(root, source_commit):
    names = git(root, "ls-tree", "-r", "--name-only", source_commit).splitlines()
    return sorted(
        name for name in names
        if name.startswith(SOURCE_PREFIXES) and name.endswith(SOURCE_SUFFIXES)
    )


def legacy_evidence_paths(root):
    evidence_root = Path(root) / "docs/porting/multiarch/evidence"
    paths = []
    for path in evidence_root.rglob("*"):
        if not path.is_file():
            continue
        relative = path.relative_to(evidence_root)
        if relative.as_posix() == "HISTORICAL.json":
            continue
        if relative.parts and relative.parts[0].startswith("main-"):
            continue
        paths.append(path.relative_to(root).as_posix())
    paths.append("docs/porting/multiarch/validation-evidence.json")
    return sorted(paths)


def validate_historical_index(root):
    index_path = Path(root) / "docs/porting/multiarch/evidence/HISTORICAL.json"
    record = json.loads(index_path.read_text())
    if record.get("schema") != "port-multiarch-historical-evidence/1":
        raise RecordsError("unsupported historical evidence index schema")
    if record.get("current_validity") != "NONE":
        raise RecordsError("historical evidence must have current_validity NONE")
    if record.get("base_sha") != HISTORICAL_BASE:
        raise RecordsError("historical index must identify its original 379d19e base")
    entries = record.get("files")
    if not isinstance(entries, list):
        raise RecordsError("historical evidence index files must be a list")
    indexed = [entry.get("path") for entry in entries if isinstance(entry, dict)]
    expected = legacy_evidence_paths(root)
    if sorted(indexed) != expected or len(indexed) != len(entries):
        raise RecordsError("historical evidence index does not cover the exact legacy file set")
    for entry in entries:
        path = Path(root) / entry["path"]
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != entry.get("sha256"):
            raise RecordsError(f"historical evidence changed: {entry['path']}")
    return len(entries)



def validate_schema_value(value, schema, root_schema, path="$"):
    if "$ref" in schema:
        ref = schema["$ref"]
        if not ref.startswith("#/$defs/"):
            raise RecordsError(f"unsupported schema reference at {path}")
        schema = root_schema["$defs"][ref.rsplit("/", 1)[1]]
    if "type" in schema:
        expected = schema["type"]
        allowed = expected if isinstance(expected, list) else [expected]
        matches = {
            "object": lambda v: isinstance(v, dict),
            "array": lambda v: isinstance(v, list),
            "string": lambda v: isinstance(v, str),
            "integer": lambda v: isinstance(v, int) and not isinstance(v, bool),
            "boolean": lambda v: isinstance(v, bool),
            "null": lambda v: v is None,
        }
        if not any(kind in matches and matches[kind](value) for kind in allowed):
            raise RecordsError(f"schema type mismatch at {path}")
    if "const" in schema and value != schema["const"]:
        raise RecordsError(f"schema const mismatch at {path}")
    if "enum" in schema and value not in schema["enum"]:
        raise RecordsError(f"schema enum mismatch at {path}")
    if isinstance(value, str):
        if "minLength" in schema and len(value) < schema["minLength"]:
            raise RecordsError(f"schema minLength mismatch at {path}")
        if "pattern" in schema and re.fullmatch(schema["pattern"], value) is None:
            raise RecordsError(f"schema pattern mismatch at {path}")
        if schema.get("format") == "date-time":
            try:
                datetime.fromisoformat(value.replace("Z", "+00:00"))
            except ValueError as exc:
                raise RecordsError(f"schema date-time mismatch at {path}") from exc
        if schema.get("format") == "uri" and not urlsplit(value).scheme:
            raise RecordsError(f"schema URI mismatch at {path}")
    if isinstance(value, int) and not isinstance(value, bool) and "minimum" in schema and value < schema["minimum"]:
        raise RecordsError(f"schema minimum mismatch at {path}")
    if isinstance(value, list):
        if "minItems" in schema and len(value) < schema["minItems"]:
            raise RecordsError(f"schema minItems mismatch at {path}")
        if "items" in schema:
            for index, item in enumerate(value):
                validate_schema_value(item, schema["items"], root_schema, f"{path}[{index}]")
    if isinstance(value, dict):
        required = schema.get("required", [])
        missing = [key for key in required if key not in value]
        if missing:
            raise RecordsError(f"schema required properties missing at {path}: {', '.join(missing)}")
        properties = schema.get("properties", {})
        if schema.get("additionalProperties") is False:
            extra = sorted(set(value) - set(properties))
            if extra:
                raise RecordsError(f"schema additional properties at {path}: {', '.join(extra)}")
        for key, child in properties.items():
            if key in value:
                validate_schema_value(value[key], child, root_schema, f"{path}.{key}")
    if "anyOf" in schema:
        errors = []
        for alternative in schema["anyOf"]:
            try:
                validate_schema_value(value, alternative, root_schema, path)
                break
            except RecordsError as exc:
                errors.append(str(exc))
        else:
            raise RecordsError(f"schema anyOf mismatch at {path}: {'; '.join(errors)}")


def validate_state_schema(state, schema):
    validate_schema_value(state, schema, schema)


def verify(root, expected_main, evidence_dir):
    root = Path(root).resolve()
    doc = root / "docs/porting/multiarch"
    head = git(root, "rev-parse", "HEAD").strip()
    source_status = validate_clean_record_worktree(root)
    inventory_path = Path(evidence_dir)
    if not inventory_path.is_absolute():
        inventory_path = root / inventory_path
    inventory_path = inventory_path.resolve()
    evidence_root = (doc / "evidence").resolve()
    if not inventory_path.is_relative_to(evidence_root):
        raise RecordsError("--evidence-dir must be inside docs/porting/multiarch/evidence")
    inventory = json.loads((inventory_path / "source-inventory.json").read_text())
    baseline = validate_current_main(root, inventory.get("baseline_sha"), expected_main)
    require_ancestor(root, baseline, head, "inventory baseline")

    state = json.loads((doc / "state.json").read_text())
    source_commit = full_commit(root, state["last_verified"]["commit_sha"], "state source commit")
    if inventory.get("source_commit_sha") != source_commit:
        raise RecordsError("inventory source_commit_sha must equal state.last_verified.commit_sha")
    source_tree = git(root, "rev-parse", f"{source_commit}^{{tree}}").strip()
    if inventory.get("source_tree_sha") != source_tree:
        raise RecordsError("inventory source_tree_sha does not match source commit tree")
    require_ancestor(root, source_commit, head, "source commit")
    validate_record_commit(root, source_commit, head)
    changed = validate_owned_scope(root, baseline, head)

    recorded_paths = sorted(item["path"] for item in inventory.get("files", []))
    expected_paths = expected_source_paths(root, source_commit)
    if recorded_paths != expected_paths:
        raise RecordsError("inventory file set does not match the complete tracked source set")
    for item in inventory["files"]:
        path = item["path"]
        current = commit_bytes(root, source_commit, path)
        baseline_bytes = commit_bytes(root, baseline, path)
        if current != baseline_bytes:
            raise RecordsError(f"source changed from the selected main baseline: {path}")
        if hashlib.sha256(current).hexdigest() != item.get("sha256"):
            raise RecordsError(f"inventory digest mismatch: {path}")

    authorities = json.loads((inventory_path / "authority-sources.json").read_text())
    if sorted(authorities) != sorted(AUTHORITY_PATHS):
        raise RecordsError("authority source set does not match the complete declared authority list")
    for path, digest in authorities.items():
        current = commit_bytes(root, source_commit, path)
        baseline_bytes = commit_bytes(root, baseline, path)
        if current != baseline_bytes:
            raise RecordsError(f"authority source differs from current main: {path}")
        if hashlib.sha256(current).hexdigest() != digest:
            raise RecordsError(f"authority hash mismatch: {path}")

    registry = json.loads(commit_bytes(root, source_commit, ".dev/workstreams.json"))
    if any(item.get("id") == "PORT-MULTIARCH-01" for item in registry.get("workstreams", [])):
        raise RecordsError("PORT-MULTIARCH-01 must remain unregistered")
    schema = json.loads(commit_bytes(root, source_commit, ".dev/schemas/workstream-state.schema.json"))
    validate_state_schema(state, schema)

    registration = json.loads((doc / "registration-proposal.json").read_text())
    if registration.get("registration_status") != "PROPOSED_UNREGISTERED":
        raise RecordsError("registration proposal must remain unapplied")
    current_status = json.loads((inventory_path / "current-status.json").read_text())
    if current_status.get("main_sha") != baseline or current_status.get("source_commit_sha") != source_commit:
        raise RecordsError("current status record is not bound to the selected main/source")
    if current_status.get("source_tree_sha") != source_tree:
        raise RecordsError("current status record is not bound to the attested source tree")
    phases = current_status.get("phases", {})
    if not all(name in phases for name in ("A", "B", "C")):
        raise RecordsError("current status record must describe A, B, and C phases")
    for name in ("A", "B", "C"):
        if phases.get(name, {}).get("status") == "PASS":
            raise RecordsError(f"phase {name} cannot be PASS in the pending current-main record")

    history_count = validate_historical_index(root)
    local_links = 0
    for directory in [doc, root / "experiments/nagi-multiarch-contracts", root / "experiments/nagi-aarch64-spike"]:
        for path in directory.glob("*.md"):
            for link in re.findall(r"\[[^\]]*\]\(([^)]+)\)", path.read_text()):
                if re.match(r"https?://", link) or link.startswith("#"):
                    continue
                if not (path.parent / link.split("#", 1)[0]).exists():
                    raise RecordsError(f"broken local link: {path}: {link}")
                local_links += 1
    python_paths = (
        root / "experiments/nagi-multiarch-contracts/audit.py",
        root / "experiments/nagi-multiarch-contracts/test_audit.py",
        root / "experiments/nagi-multiarch-contracts/test_records.py",
        root / "experiments/nagi-aarch64-spike/run.py",
        root / "experiments/nagi-aarch64-spike/attest.py",
        root / "experiments/nagi-aarch64-spike/verify_attestation.py",
        root / "experiments/nagi-aarch64-spike/test_runner.py",
        root / "experiments/nagi-aarch64-spike/test_attestation.py",
        Path(__file__),
    )
    for path in python_paths:
        compile(path.read_text(), str(path), "exec")
    subprocess.run(["git", "diff", "--check"], cwd=root, check=True)
    return {
        "main": baseline,
        "source": source_commit,
        "changed_paths": len(changed),
        "source_files": len(recorded_paths),
        "authority_files": len(authorities),
        "historical_files": history_count,
        "registry_count": len(registry.get("workstreams", [])),
        "local_links": local_links,
        "source_status": source_status,
    }


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--main", required=True, help="explicit full SHA of the current main baseline")
    parser.add_argument("--evidence-dir", required=True, help="current evidence directory under docs/porting/multiarch/evidence")
    args = parser.parse_args(argv)
    try:
        result = verify(ROOT, args.main, args.evidence_dir)
    except (RecordsError, OSError, KeyError, ValueError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    print(
        f"PASS records-only child; main={result['main']} source={result['source']}; "
        f"{result['source_files']} source files, {result['authority_files']} authority files, "
        f"{result['registry_count']} registry entries, {result['historical_files']} immutable historical files; "
        f"{result['changed_paths']} owned paths, {result['local_links']} local links"
    )
    if result["source_status"]["local_guard_untracked_count"]:
        roots = ", ".join(result["source_status"]["local_guard_untracked_roots"])
        print(f"TRACKED CLEAN; {result['source_status']['local_guard_untracked_count']} untracked local guard files under {roots} are excluded from source scope")


if __name__ == "__main__":
    main()
