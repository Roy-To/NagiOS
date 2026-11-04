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


def require_sha256(value, label):
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
        raise RecordsError(f"{label} must be a full SHA-256")


def json_file(path, label):
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        raise RecordsError(f"cannot read {label}: {exc}") from exc


def validate_baseline_drift(root, path, baseline, source_commit, source_tree):
    drift = json_file(path, "baseline-drift record")
    required = {"schema", "old_main_sha", "current_main_sha", "old_main_tree_sha", "current_main_tree_sha",
        "source_commit_sha", "source_tree_sha", "pr_head_sha", "candidate_merge_sha",
        "historical_common_ancestor_sha", "main_changed_paths", "main_changed_path_count",
        "main_changes_in_owned_prefixes", "owned_prefixes_unchanged_from_old_main",
        "pr_to_merge_changes_in_owned_prefixes", "pr_contract_and_spike_tree_preserved_by_main_merge",
        "remote_publish_or_PR_mutation"}
    if not isinstance(drift, dict) or set(drift) != required:
        raise RecordsError("baseline-drift record has missing or extra fields")
    if drift["schema"] != "nagi-pr57-main-drift/1":
        raise RecordsError("unsupported baseline-drift schema")
    old_main = full_commit(root, drift["old_main_sha"], "baseline-drift old_main_sha")
    current = full_commit(root, drift["current_main_sha"], "baseline-drift current_main_sha")
    if current != baseline:
        raise RecordsError("baseline-drift current_main_sha differs from the explicit current baseline")
    old_tree = git(root, "rev-parse", f"{old_main}^{{tree}}").strip()
    current_tree = git(root, "rev-parse", f"{current}^{{tree}}").strip()
    if drift["old_main_tree_sha"] != old_tree or drift["current_main_tree_sha"] != current_tree:
        raise RecordsError("baseline-drift tree hashes do not match their main commits")
    if drift["source_commit_sha"] != source_commit or drift["source_tree_sha"] != source_tree:
        raise RecordsError("baseline-drift source identity differs from current source records")
    pr_head = full_commit(root, drift["pr_head_sha"], "baseline-drift pr_head_sha")
    candidate = full_commit(root, drift["candidate_merge_sha"], "baseline-drift candidate_merge_sha")
    common = git(root, "merge-base", old_main, current).strip()
    if drift["historical_common_ancestor_sha"] != common:
        raise RecordsError("baseline-drift historical common ancestor is incorrect")
    actual = git(root, "diff", "--name-status", old_main, current).splitlines()
    if drift["main_changed_paths"] != actual or drift["main_changed_path_count"] != len(actual):
        raise RecordsError("baseline-drift main path list/count differs from the fixed commit diff")
    owned = sorted({p for line in actual for p in line.split("\t")[1:] if p.startswith(PREFIXES)})
    if drift["main_changes_in_owned_prefixes"] != owned or drift["pr_to_merge_changes_in_owned_prefixes"] != owned:
        raise RecordsError("baseline-drift owned-prefix lists differ from the fixed main diff")
    if drift["owned_prefixes_unchanged_from_old_main"] is not (not owned):
        raise RecordsError("baseline-drift owned-prefix unchanged flag is incorrect")
    spike = sorted({p for line in actual for p in line.split("\t")[1:]
        if p.startswith(("experiments/nagi-multiarch-contracts/", "experiments/nagi-aarch64-spike/"))})
    if drift["pr_contract_and_spike_tree_preserved_by_main_merge"] is not (not spike):
        raise RecordsError("baseline-drift contract/spike preservation flag is incorrect")
    if drift["remote_publish_or_PR_mutation"] != "NOT_RUN":
        raise RecordsError("baseline-drift must not claim remote publication or PR mutation")
    require_ancestor(root, current, candidate, "current main in candidate merge")
    require_ancestor(root, pr_head, candidate, "PR head in candidate merge")
    require_ancestor(root, candidate, source_commit, "candidate merge in source commit")
    return drift


def validate_host_cargo_acceptance(root, status, tools, source_commit, source_tree):
    record = tools.get("host_Cargo_contract_tests")
    if record is None:
        return "NOT_RUN"
    required = {
        "outcome", "source_commit_sha", "source_tree_sha", "command", "toolchain", "rustc_version",
        "locked", "offline", "cargo_tests", "doctests", "bootinfo_tests", "abi_tests",
        "clippy_outcome", "fmt_outcome", "clippy_deny_warnings", "exit_code", "stdout_sha256",
        "stderr_sha256", "metadata_file", "prior_attempts",
    }
    if not isinstance(record, dict) or set(record) != required:
        raise RecordsError("host Cargo contract record has missing or extra fields")
    if (record.get("outcome") != "PASS" or record.get("source_commit_sha") != source_commit
            or record.get("source_tree_sha") != source_tree):
        raise RecordsError("host Cargo result is not bound to the current source commit/tree")
    if record.get("command") != "./experiments/nagi-multiarch-contracts/verify.sh":
        raise RecordsError("host Cargo command is not the pinned contract verification entry point")
    if (record.get("toolchain") != "nightly-2025-08-01"
            or not isinstance(record.get("rustc_version"), str)
            or not record["rustc_version"].startswith("rustc 1.90.0-nightly (adcb3d3b4 2025-07-31)")):
        raise RecordsError("host Cargo result does not identify the pinned nightly rustc")
    if record.get("locked") is not True or record.get("offline") is not True:
        raise RecordsError("host Cargo result must be locked and offline")
    expected_counts = {"cargo_tests": 99, "doctests": 0, "bootinfo_tests": 16, "abi_tests": 5}
    for key, expected in expected_counts.items():
        value = record.get(key)
        if isinstance(value, bool) or value != expected:
            raise RecordsError("host Cargo test count is invalid: " + key)
    if record.get("clippy_outcome") != "PASS" or record.get("fmt_outcome") != "PASS" or record.get("clippy_deny_warnings") is not True:
        raise RecordsError("host Cargo formatting or strict Clippy result is invalid")
    if isinstance(record.get("exit_code"), bool) or record.get("exit_code") != 0:
        raise RecordsError("host Cargo verification must record a zero exit code")
    require_sha256(record.get("stdout_sha256"), "host_Cargo_contract_tests.stdout_sha256")
    require_sha256(record.get("stderr_sha256"), "host_Cargo_contract_tests.stderr_sha256")
    metadata_file = record.get("metadata_file")
    if (not isinstance(metadata_file, str) or metadata_file.startswith("/")
            or metadata_file != "host-cargo-verify-fe52fedc/verify.meta.json"):
        raise RecordsError("host Cargo metadata path must be the fixed relative private-evidence reference")
    attempts = record.get("prior_attempts")
    attempt_fields = {"name", "record_head_sha", "result", "exit_code", "failure", "index_sha256"}
    if not isinstance(attempts, list) or len(attempts) != 4:
        raise RecordsError("host Cargo history must preserve all four earlier attempts")
    for attempt in attempts:
        if not isinstance(attempt, dict) or set(attempt) != attempt_fields:
            raise RecordsError("host Cargo prior-attempt record has missing or extra fields")
        full_commit(root, attempt.get("record_head_sha"), "host Cargo prior attempt")
        require_sha256(attempt.get("index_sha256"), "host Cargo prior-attempt index_sha256")
        if attempt.get("result") not in {"FAIL", "PARTIAL_FAIL"} or not isinstance(attempt.get("failure"), str) or not attempt["failure"].strip():
            raise RecordsError("host Cargo prior attempt must retain its failed result and reason")
        if isinstance(attempt.get("exit_code"), bool) or attempt.get("exit_code") not in {101, 1}:
            raise RecordsError("host Cargo prior-attempt exit code is invalid")
    if attempts[0].get("name") != "host-cargo-validation-ca0a8d2f" or "E0658" not in attempts[0].get("failure", ""):
        raise RecordsError("initial unpinned compiler failure is not preserved")
    if attempts[-1].get("name") != "host-cargo-validation-pinned-38f95bac" or attempts[-1].get("result") != "PARTIAL_FAIL":
        raise RecordsError("last intermediate pinned-nightly failure is not preserved")
    log_metadata = status.get("private_tool_log_metadata")
    if not isinstance(log_metadata, list):
        raise RecordsError("private tool log metadata must be an array")
    matches = [entry for entry in log_metadata if isinstance(entry, dict)
        and entry.get("name") == "host-cargo-verify-fe52fedc" and entry.get("metadata_file") == metadata_file]
    if len(matches) != 1:
        raise RecordsError("host Cargo result must bind to exactly one private metadata entry")
    entry = matches[0]
    for field in ("record_head_sha", "exit_code", "stdout_sha256", "stderr_sha256"):
        expected = source_commit if field == "record_head_sha" else record[field]
        if entry.get(field) != expected:
            raise RecordsError("host Cargo private metadata binding differs: " + field)
    return "PASS"


def validate_current_status(root, status_path, inventory_path, drift_path, baseline, source_commit, source_tree, head):
    status = json_file(status_path, "current status record")
    if not isinstance(status, dict):
        raise RecordsError("current status record must be an object")
    status_schema = status.get("schema")
    if status_schema not in {"nagi-multiarch-current-status/1", "nagi-multiarch-current-status/2"}:
        raise RecordsError("unsupported current status schema")
    expected_tree = git(root, "rev-parse", f"{baseline}^{{tree}}").strip()
    if status.get("main_sha") != baseline or status.get("main_tree_sha") != expected_tree:
        raise RecordsError("current status main commit/tree do not match the fixed baseline")
    if status.get("source_commit_sha") != source_commit or status.get("source_tree_sha") != source_tree:
        raise RecordsError("current status source commit/tree do not match the fixed source")
    expected_artifacts = {"source_inventory_sha256": Path(inventory_path),
        "authority_sources_sha256": Path(inventory_path).parent / "authority-sources.json",
        "baseline_drift_sha256": Path(drift_path)}
    hashes = status.get("artifact_hashes")
    if not isinstance(hashes, dict) or set(hashes) != set(expected_artifacts):
        raise RecordsError("current status artifact_hashes has missing or extra entries")
    for name, path in expected_artifacts.items():
        require_sha256(hashes.get(name), "artifact_hashes." + name)
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != hashes[name]:
            raise RecordsError("current status artifact hash mismatch: " + name)
    raw_validation = status.get("records_validation")
    validation_outcome = raw_validation.get("outcome") if isinstance(raw_validation, dict) else None
    registry_validation_state = "PENDING" if validation_outcome == "PENDING" else "PASS"
    tools = status.get("tool_acceptance", {})
    if not isinstance(tools, dict):
        raise RecordsError("tool_acceptance must be an object")
    host_cargo_state = validate_host_cargo_acceptance(root, status, tools, source_commit, source_tree)
    expected_phases = {
        "A": ("PENDING", {"current_static_inventory": "PASS", "current_authority_hashes": "PASS",
            "registry_unregistered_and_record_validation": registry_validation_state, "ownership_and_live_ref_snapshot": "NOT_RUN"}),
        "B": ("PENDING", {"model_free_tool_acceptance": "PASS", "host_Cargo_contract_tests": host_cargo_state}),
        "C": ("NOT_RUN", {"new_ELF": "NOT_RUN", "QEMU_EL1": "NOT_RUN",
            "QEMU_EL2_negative": "NOT_RUN", "independent_rebuild": "NOT_RUN"}),
    }
    phases = status.get("phases")
    if not isinstance(phases, dict) or set(phases) != set(expected_phases):
        raise RecordsError("current status phases must contain exactly A, B, and C")
    for name, (state, checks) in expected_phases.items():
        phase = phases[name]
        if not isinstance(phase, dict) or phase.get("status") != state or phase.get("checks") != checks:
            raise RecordsError(f"phase {name} status/checks are not the strict pending-state contract")
        if not isinstance(phase.get("reason"), str) or not phase["reason"].strip():
            raise RecordsError(f"phase {name} requires a reason")
    expected_execution = {"cargo_check": "NOT_RUN", "cargo_clippy": "NOT_RUN", "cargo_build": "NOT_RUN",
        "new_ELF_generation": "NOT_RUN", "QEMU": "NOT_RUN", "VM": "NOT_RUN", "Miri": "NOT_RUN",
        "independent_rebuild": "NOT_RUN"}
    if status_schema == "nagi-multiarch-current-status/1":
        if status.get("execution") != expected_execution or "target_execution" in status:
            raise RecordsError("current status v1 execution fields must all remain NOT_RUN")
    elif status.get("target_execution") != expected_execution or "execution" in status:
        raise RecordsError("current status v2 target_execution fields must all remain NOT_RUN")
    validation = status.get("records_validation")
    required_validation = {"outcome", "record_head_sha", "record_tree_sha", "source_commit_sha", "command", "exit_code", "stdout_path", "stdout_sha256"}
    if not isinstance(validation, dict) or set(validation) != required_validation:
        raise RecordsError("records_validation has missing or extra fields")
    expected_command = ("python3 docs/porting/multiarch/verify-records.py --main " + baseline
        + " --evidence-dir docs/porting/multiarch/evidence/main-" + baseline)
    if validation.get("source_commit_sha") != source_commit or validation.get("command") != expected_command:
        raise RecordsError("records_validation is not bound to the explicit main/source command")
    validation_outcome = validation.get("outcome")
    if validation_outcome == "PENDING":
        if any(validation.get(key) is not None for key in ("record_head_sha", "record_tree_sha", "exit_code", "stdout_path", "stdout_sha256")):
            raise RecordsError("pending records_validation must not claim a record head, exit code, or result artifact")
    elif validation_outcome == "PASS":
        record_head = full_commit(root, validation.get("record_head_sha"), "records_validation.record_head_sha")
        record_tree = git(root, "rev-parse", f"{record_head}^{{tree}}").strip()
        if validation.get("record_tree_sha") != record_tree:
            raise RecordsError("records_validation.record_tree_sha does not match its commit")
        if record_head == source_commit:
            raise RecordsError("records_validation record head must be a strict records-only child of source")
        parents = git(root, "rev-list", "--parents", "-n", "1", record_head).split()[1:]
        if parents != [source_commit]:
            raise RecordsError("records_validation record head must directly follow the source commit")
        require_ancestor(root, record_head, head, "records_validation record head")
        validate_record_commit(root, source_commit, record_head)
        if isinstance(validation.get("exit_code"), bool) or validation.get("exit_code") != 0:
            raise RecordsError("records_validation must record a successful zero exit code")
        expected_stdout_path = (
            "docs/porting/multiarch/evidence/" + Path(inventory_path).parent.name + "/records-validation.stdout.log"
        )
        if validation.get("stdout_path") != expected_stdout_path:
            raise RecordsError("records_validation stdout_path is not the fixed evidence log")
        stdout_path = Path(root) / expected_stdout_path
        if stdout_path.is_symlink() or not stdout_path.is_file():
            raise RecordsError("records_validation stdout log is missing or unsafe")
        require_sha256(validation.get("stdout_sha256"), "records_validation.stdout_sha256")
        stdout_bytes = stdout_path.read_bytes()
        if hashlib.sha256(stdout_bytes).hexdigest() != validation["stdout_sha256"]:
            raise RecordsError("records_validation stdout hash does not match the saved execution log")
        expected_prefix = (
            f"PASS records-only child; main={baseline} source={source_commit}; "
            f"record_head={record_head} record_tree={record_tree};"
        )
        try:
            stdout_text = stdout_bytes.decode("utf-8")
        except UnicodeDecodeError as exc:
            raise RecordsError("records_validation stdout log is not UTF-8") from exc
        if not stdout_text.startswith(expected_prefix):
            raise RecordsError("records_validation stdout log does not record the claimed PASS and commit")
    else:
        raise RecordsError("records_validation outcome must be PENDING or PASS")
    tests = tools.get("model_free_python_tests", {})
    contract_count = tests.get("contracts_audit_and_records") if isinstance(tests, dict) else None
    runner_count = tests.get("runner_and_receipt") if isinstance(tests, dict) else None
    total_count = tests.get("tests") if isinstance(tests, dict) else None
    counts_valid = all(isinstance(value, int) and not isinstance(value, bool) and value >= 0
        for value in (contract_count, runner_count, total_count))
    if not counts_valid or tests.get("outcome") != "PASS" or total_count <= 0 or contract_count + runner_count != total_count:
        raise RecordsError("model-free Python test counts are invalid")
    syntax, shell, fmt = tools.get("python_syntax", {}), tools.get("shell_syntax", {}), tools.get("cargo_fmt", {})
    if syntax.get("outcome") != "PASS" or not isinstance(syntax.get("files"), int) or syntax["files"] < 1:
        raise RecordsError("python_syntax acceptance is invalid")
    if shell.get("outcome") != "PASS" or not isinstance(shell.get("scripts"), int) or shell["scripts"] < 1:
        raise RecordsError("shell_syntax acceptance is invalid")
    if fmt.get("outcome") != "PASS" or not isinstance(fmt.get("packages"), int) or fmt["packages"] < 1 or fmt.get("build_performed") is not False:
        raise RecordsError("cargo_fmt acceptance is invalid")
    inventory = json_file(inventory_path, "source inventory")
    static, exact = tools.get("static_inventory", {}), tools.get("static_inventory_exact_check", {})
    if static.get("outcome") != "PASS" or static.get("source_files") != len(inventory.get("files", [])):
        raise RecordsError("static inventory count differs from source inventory")
    if static.get("authority_files") != len(AUTHORITY_PATHS) or exact.get("outcome") != "PASS" or exact.get("source_commit_explicit") is not True:
        raise RecordsError("static inventory exact-check acceptance is invalid")
    return status


def validate_main_record_index(root, current_main):
    evidence = Path(root) / "docs/porting/multiarch/evidence"
    index = json_file(evidence / "main-records-index.json", "main records index")
    if not isinstance(index, dict) or set(index) != {"schema", "current_main_sha", "entries"}:
        raise RecordsError("main records index has missing or extra fields")
    if index.get("schema") != "nagi-multiarch-main-records-index/1" or index.get("current_main_sha") != current_main:
        raise RecordsError("main records index is not bound to current main")
    entries = index.get("entries")
    if not isinstance(entries, list):
        raise RecordsError("main records index entries must be a list")
    actual_dirs = sorted(item.name for item in evidence.glob("main-*") if item.is_dir())
    indexed_dirs, current_count = [], 0
    base_required_names = {"source-inventory.json", "authority-sources.json", "baseline-drift.json", "current-status.json"}
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {"main_sha", "status", "files"}:
            raise RecordsError("main records index entry is malformed")
        main_sha = full_commit(root, entry.get("main_sha"), "main records index main_sha")
        directory = "main-" + main_sha
        indexed_dirs.append(directory)
        expected_status = "CURRENT" if main_sha == current_main else "SUPERSEDED"
        if entry.get("status") != expected_status:
            raise RecordsError("main records index status is invalid: " + directory)
        current_count += int(expected_status == "CURRENT")
        record_dir = evidence / directory
        if not record_dir.is_dir() or record_dir.is_symlink():
            raise RecordsError("indexed main evidence directory is missing or unsafe: " + directory)
        actual_files = []
        for file_path in record_dir.rglob("*"):
            if file_path.is_symlink():
                raise RecordsError("main evidence contains a symlink: " + str(file_path.relative_to(evidence)))
            if file_path.is_file():
                actual_files.append(file_path.relative_to(evidence).as_posix())
        status_record = json_file(record_dir / "current-status.json", "indexed current status")
        validation = status_record.get("records_validation")
        stdout_path = validation.get("stdout_path") if isinstance(validation, dict) else None
        required_names = set(base_required_names)
        if stdout_path is not None:
            expected_stdout_path = (
                "docs/porting/multiarch/evidence/" + directory + "/records-validation.stdout.log"
            )
            if stdout_path != expected_stdout_path:
                raise RecordsError("indexed records-validation stdout path is not fixed: " + directory)
            required_names.add("records-validation.stdout.log")
        if {Path(item).name for item in actual_files} != required_names:
            raise RecordsError("main evidence file set is incomplete or unexpected: " + directory)
        files = entry.get("files")
        if not isinstance(files, list) or any(not isinstance(item, dict) or set(item) != {"path", "sha256"} for item in files):
            raise RecordsError("main records file list is malformed: " + directory)
        paths = [item.get("path") for item in files]
        if sorted(paths) != sorted(actual_files) or len(set(paths)) != len(paths):
            raise RecordsError("main records index does not cover exact file set: " + directory)
        for item in files:
            require_sha256(item.get("sha256"), "main records index digest")
            file_path = evidence / item["path"]
            if not file_path.is_file() or hashlib.sha256(file_path.read_bytes()).hexdigest() != item["sha256"]:
                raise RecordsError("main evidence hash mismatch: " + item["path"])
        if json_file(record_dir / "source-inventory.json", "indexed inventory").get("baseline_sha") != main_sha:
            raise RecordsError("indexed inventory baseline differs from directory name")
        if json_file(record_dir / "current-status.json", "indexed status").get("main_sha") != main_sha:
            raise RecordsError("indexed current status differs from directory name")
    if sorted(indexed_dirs) != actual_dirs or len(set(indexed_dirs)) != len(indexed_dirs):
        raise RecordsError("main records index does not cover exact main-* directory set")
    if current_count != 1 or indexed_dirs != sorted(indexed_dirs):
        raise RecordsError("main records index must have one current entry and sorted full SHAs")
    return len(actual_dirs)


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
    current_status_path = inventory_path / "current-status.json"
    drift_path = inventory_path / "baseline-drift.json"
    current_status = validate_current_status(
        root, current_status_path, inventory_path / "source-inventory.json", drift_path,
        baseline, source_commit, source_tree, head,
    )
    validate_baseline_drift(root, drift_path, baseline, source_commit, source_tree)
    validate_main_record_index(root, baseline)

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
        "record_head": head,
        "record_tree": git(root, "rev-parse", f"{head}^{{tree}}").strip(),
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
        f"record_head={result['record_head']} record_tree={result['record_tree']}; "
        f"{result['source_files']} source files, {result['authority_files']} authority files, "
        f"{result['registry_count']} registry entries, {result['historical_files']} immutable historical files; "
        f"{result['changed_paths']} owned paths, {result['local_links']} local links"
    )
    if result["source_status"]["local_guard_untracked_count"]:
        roots = ", ".join(result["source_status"]["local_guard_untracked_roots"])
        print(f"TRACKED CLEAN; {result['source_status']['local_guard_untracked_count']} untracked local guard files under {roots} are excluded from source scope")


if __name__ == "__main__":
    main()
