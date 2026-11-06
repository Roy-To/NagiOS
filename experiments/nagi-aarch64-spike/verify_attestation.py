#!/usr/bin/env python3
"""Verify source-bound spike receipts without executing Cargo or QEMU."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

from run import accepted, expected_el2_rejection, valid_elf

FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
HEX_SHA256 = re.compile(r"^[0-9a-f]{64}$")
SPIKE_REL = "experiments/nagi-aarch64-spike"
PREFIXES = (
    "docs/porting/multiarch/",
    "experiments/nagi-multiarch-contracts/",
    "experiments/nagi-aarch64-spike/",
)


class VerificationError(ValueError):
    pass


def git(root, *args, check=True, text=True):
    result = subprocess.run(["git", *args], cwd=root, capture_output=True, text=text, check=False)
    if check and result.returncode:
        message = result.stderr if isinstance(result.stderr, str) else result.stderr.decode("utf-8", "replace")
        raise VerificationError(message.strip() or f"git {' '.join(args)} failed")
    return result.stdout


def full_commit(root, value, label):
    if not isinstance(value, str) or not FULL_SHA.fullmatch(value):
        raise VerificationError(f"{label} must be a full commit SHA")
    resolved = git(root, "rev-parse", "--verify", f"{value}^{{commit}}", check=False).strip()
    if resolved != value:
        raise VerificationError(f"{label} is not the supplied commit object")
    return resolved


def full_tree(root, value, label):
    if not isinstance(value, str) or not FULL_SHA.fullmatch(value):
        raise VerificationError(f"{label} must be a full tree SHA")
    resolved = git(root, "rev-parse", "--verify", f"{value}^{{tree}}", check=False).strip()
    if resolved != value:
        raise VerificationError(f"{label} is not the supplied tree object")
    return resolved


def sha_file(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def commit_blob_sha(root, commit, path):
    raw = git(root, "show", f"{commit}:{path}", text=False)
    return hashlib.sha256(raw).hexdigest()


def is_ancestor(root, ancestor, descendant, label):
    result = subprocess.run(
        ["git", "merge-base", "--is-ancestor", ancestor, descendant],
        cwd=root, capture_output=True, check=False,
    )
    if result.returncode:
        raise VerificationError(f"{label} is not an ancestor")


def safe_file(root, relative):
    if not isinstance(relative, str) or not relative or Path(relative).is_absolute():
        raise VerificationError("artifact path must be relative to the attestation directory")
    candidate = (root / relative).resolve()
    if not candidate.is_relative_to(root.resolve()):
        raise VerificationError("artifact path escapes the attestation directory")
    return candidate


def check_manifest(output, record):
    manifest = record.get("files")
    if not isinstance(manifest, dict) or "attestation.json" in manifest:
        raise VerificationError("manifest must be a map and must exclude attestation.json")
    expected = set()
    for path in output.rglob("*"):
        if path.is_symlink():
            raise VerificationError(f"manifest output contains a symlink: {path.relative_to(output)}")
    for relative, digest in manifest.items():
        if not HEX_SHA256.fullmatch(str(digest)):
            raise VerificationError(f"invalid manifest digest: {relative}")
        path = safe_file(output, relative)
        if not path.is_file():
            raise VerificationError(f"manifest file is missing: {relative}")
        if sha_file(path) != digest:
            raise VerificationError(f"manifest hash mismatch: {relative}")
        expected.add(relative)
    actual = {
        path.relative_to(output).as_posix()
        for path in output.rglob("*")
        if path.is_file() and path.name != "attestation.json"
    }
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        raise VerificationError(f"manifest file set mismatch; missing={missing}, extra={extra}")


def check_source(root, record, expected_main, expected_source_commit, expected_source_tree):
    source = record.get("source", {})
    main = source.get("main")
    commit = full_commit(root, source.get("commit"), "source.commit")
    tree = source.get("tree")
    tree = full_tree(root, tree, "source.tree")
    if git(root, "rev-parse", f"{commit}^{{tree}}").strip() != tree:
        raise VerificationError("source tree does not match its commit")
    main = full_commit(root, main, "source.main")
    if full_commit(root, expected_main, "--main") != main:
        raise VerificationError("attestation main differs from explicit --main")
    if full_commit(root, expected_source_commit, "--source-commit") != commit:
        raise VerificationError("attestation source commit differs from explicit --source-commit")
    if full_tree(root, expected_source_tree, "--source-tree") != tree:
        raise VerificationError("attestation source tree differs from explicit --source-tree")
    is_ancestor(root, main, commit, "explicit main")
    head = git(root, "rev-parse", "HEAD").strip()
    is_ancestor(root, commit, head, "attested source commit")
    changed = git(root, "diff", "--name-only", main, commit).splitlines()
    outside = [path for path in changed if not path.startswith(PREFIXES)]
    if outside:
        raise VerificationError("source commit contains out-of-scope paths: " + ", ".join(outside))
    later = git(root, "diff", "--name-only", commit, head).splitlines()
    outside_later = [path for path in later if not path.startswith("docs/porting/multiarch/")]
    if outside_later:
        raise VerificationError("record-only child changed source paths: " + ", ".join(outside_later))
    if git(root, "diff", "--name-only", commit, head, "--", "experiments/nagi-aarch64-spike", "experiments/nagi-multiarch-contracts").splitlines():
        raise VerificationError("record-only child changed experiment sources")
    for key, path in (("generator", f"{SPIKE_REL}/attest.py"), ("runner", f"{SPIKE_REL}/run.py")):
        identity = record.get(key, {})
        if identity.get("path") != path:
            raise VerificationError(f"{key} path does not match committed tool")
        if git(root, "rev-parse", f"{commit}:{path}").strip() != identity.get("git_blob"):
            raise VerificationError(f"{key} blob does not match source commit")
        raw = git(root, "show", f"{commit}:{path}", text=False)
        if hashlib.sha256(raw).hexdigest() != identity.get("sha256"):
            raise VerificationError(f"{key} digest does not match source commit")
    if source.get("tracked_clean") is not True or source.get("source_clean") is not True:
        raise VerificationError("source worktree was not recorded as clean")
    if source.get("head_after_build") != commit:
        raise VerificationError("source HEAD changed during the build/run")
    if source.get("tree_after_build") != tree:
        raise VerificationError("source tree changed during the build/run")
    return commit, tree, main


def check_builds(output, record, commit, tree, toolchain):
    builds = record.get("builds")
    if not isinstance(builds, list) or [item.get("id") for item in builds] != ["a", "b"]:
        raise VerificationError("attestation must contain exactly build a and build b")
    expected_args = ["build", "--release", "--locked", "--offline"]
    source_snapshot = record.get("source_snapshot", {}).get("path")
    for build in builds:
        if build.get("argv") != [toolchain.get("cargo_path"), *expected_args]:
            raise VerificationError("build command does not use the recorded pinned Cargo executable")
        if build.get("exit_code") != 0:
            raise VerificationError("build command/result is not a successful locked offline release build")
        if build.get("cwd") != source_snapshot:
            raise VerificationError("build working directory is not the committed source snapshot")
        if build.get("cargo_home_seed") != "empty":
            raise VerificationError("build must use an isolated empty CARGO_HOME")
        if build.get("environment_policy") != "minimal-allowlist; inherited build variables discarded":
            raise VerificationError("build environment policy is incomplete")
        if build.get("cargo_config_policy") != "allow hash-verified committed source-snapshot .cargo configs; reject external configs and configs in isolated CARGO_HOME":
            raise VerificationError("Cargo config isolation policy is incomplete")
        if build.get("source_archive_sha256") != record.get("source_snapshot", {}).get("archive_sha256"):
            raise VerificationError("build input archive differs from the committed source snapshot")
        for field in ("stdout_path", "stderr_path", "elf_path"):
            path = safe_file(output, build.get(field))
            if not path.is_file():
                raise VerificationError(f"build artifact is missing: {build.get(field)}")
        for field, digest in (("stdout_path", "stdout_sha256"), ("stderr_path", "stderr_sha256"), ("elf_path", "elf_sha256")):
            if sha_file(safe_file(output, build[field])) != build.get(digest):
                raise VerificationError(f"build artifact digest mismatch: {build[field]}")
        if not valid_elf(safe_file(output, build["elf_path"]).read_bytes()) or build.get("elf_machine") != 183:
            raise VerificationError("build artifact is not ELF64 little-endian AArch64")
        if build.get("source_commit") != commit or build.get("source_tree") != tree:
            raise VerificationError("build block is not bound to source identity")
    return builds

def check_runs(output, record, builds, toolchain):
    runs = record.get("runs")
    if not isinstance(runs, list) or [item.get("id") for item in runs] != ["el1", "el2"]:
        raise VerificationError("attestation must contain EL1 and EL2 runs")
    qemu = record.get("qemu", {})
    if not HEX_SHA256.fullmatch(str(qemu.get("sha256", ""))) or not qemu.get("path_realpath"):
        raise VerificationError("QEMU identity is incomplete")
    source_snapshot = record.get("source_snapshot", {}).get("path")
    for item in runs:
        if item.get("runner_exit_code") != (0 if item["id"] == "el1" else 1):
            raise VerificationError("runner exit code does not match the expected branch")
        if item.get("cwd") != source_snapshot or item.get("runner_source_path") != source_snapshot + "/run.py":
            raise VerificationError("runner command is not tied to the committed source snapshot")
        for field, digest in (("result_path", "result_json_sha256"), ("serial_path", "serial_sha256"),
                              ("stderr_path", "stderr_sha256"), ("runner_stdout_path", "runner_stdout_sha256"),
                              ("runner_stderr_path", "runner_stderr_sha256")):
            path = safe_file(output, item.get(field))
            if not path.is_file() or sha_file(path) != item.get(digest):
                raise VerificationError(f"run artifact missing or hash mismatch: {item.get(field)}")
        result = json.loads(safe_file(output, item["result_path"]).read_text())
        serial = safe_file(output, item["serial_path"]).read_bytes()
        if result.get("source_attestation") != "NOT_CLAIMED_BY_RUNNER":
            raise VerificationError("runner must remain source-neutral")
        if result.get("binary_sha256") != result.get("binary_sha256_after") or item.get("binary_sha256") != item.get("binary_sha256_after"):
            raise VerificationError("ELF changed during run")
        if result.get("qemu_sha256") != result.get("qemu_sha256_after") or item.get("qemu_sha256") != item.get("qemu_sha256_after"):
            raise VerificationError("QEMU changed during run")
        if result.get("qemu_sha256") != qemu.get("sha256") or result.get("qemu_path") != qemu.get("path_realpath"):
            raise VerificationError("run QEMU identity differs from attested QEMU")
        argv = item.get("argv", [])
        if len(argv) < 2 or argv[0] != toolchain.get("python_path") or argv[1] != "run.py":
            raise VerificationError("run command must use the recorded Python and snapshot runner")
        if "--qemu" not in argv or argv.count("--qemu") != 1:
            raise VerificationError("run command must name exactly one attested QEMU executable")
        if Path(argv[argv.index("--qemu") + 1]).resolve() != Path(qemu["path_realpath"]).resolve():
            raise VerificationError("run command QEMU path differs from attested QEMU")
        if "--binary" not in argv or argv.count("--binary") != 1:
            raise VerificationError("run command must name exactly one build artifact")
        expected_binary = safe_file(output, builds[0]["elf_path"]).resolve()
        if Path(argv[argv.index("--binary") + 1]).resolve() != expected_binary or Path(result.get("binary_path", "")).resolve() != expected_binary:
            raise VerificationError("run did not use the ELF generated in this attestation output")
        if result.get("serial_sha256") != sha_file(safe_file(output, item["serial_path"])) or result.get("stderr_sha256") != sha_file(safe_file(output, item["stderr_path"])):
            raise VerificationError("runner serial/stderr digest differs from saved output")
        expected_virt = "off" if item["id"] == "el1" else "on"
        if result.get("virtualization") != expected_virt or item.get("virtualization") != expected_virt:
            raise VerificationError("run virtualization mode does not match its expected EL")
        if item["id"] == "el1":
            if result.get("outcome") != "PASS" or result.get("returncode") != 0 or result.get("timed_out") is not False:
                raise VerificationError("EL1 runner result is not a complete PASS")
            if not accepted(result.get("returncode"), serial, result.get("timed_out") is True):
                raise VerificationError("EL1 serial does not satisfy the complete marker sequence")
        else:
            if result.get("outcome") != "FAIL" or item.get("derived_verdict") != "MET":
                raise VerificationError("EL2 must remain runner FAIL while its negative condition is met")
            if not expected_el2_rejection(result.get("returncode"), serial, result.get("timed_out") is True):
                raise VerificationError("EL2 did not meet the expected-negative condition")
        if result.get("binary_sha256") != builds[0].get("elf_sha256"):
            raise VerificationError("run did not use build a's ELF")
    acceptance = record.get("acceptance", {})
    if acceptance.get("el1") != "PASS" or acceptance.get("el2_negative") != "MET" or acceptance.get("source_bound_runs") != "PASS":
        raise VerificationError("acceptance summary does not match verified run records")
    if acceptance.get("independent_rebuild") != "NOT_RUN" or acceptance.get("phase_c") != "PENDING":
        raise VerificationError("phase C cannot pass without an independent rebuild review")

def check_source_snapshot(repo, output, record, source_commit):
    snapshot = record.get("source_snapshot", {})
    relative = snapshot.get("path")
    if relative != "source-snapshot/" + SPIKE_REL or snapshot.get("archive_path") != "source/spike.tar":
        raise VerificationError("source snapshot paths are not canonical")
    archive_bytes = git(repo, "archive", "--format=tar", source_commit, "--", SPIKE_REL, text=False)
    archive_sha = hashlib.sha256(archive_bytes).hexdigest()
    if snapshot.get("archive_sha256") != archive_sha:
        raise VerificationError("source archive digest does not match the selected commit")
    if safe_file(output, snapshot["archive_path"]).read_bytes() != archive_bytes:
        raise VerificationError("saved source archive differs from the selected commit")
    snapshot_root = safe_file(output, "source-snapshot")
    if not snapshot_root.is_dir() or snapshot_root.is_symlink() or snapshot_root.stat().st_mode & 0o222:
        raise VerificationError("committed source snapshot directory is missing, unsafe, or writable")
    expected = git(repo, "ls-tree", "-r", "--name-only", source_commit, "--", SPIKE_REL).splitlines()
    actual = []
    for path in snapshot_root.rglob("*"):
        if path.is_symlink():
            raise VerificationError("source snapshot must not contain symlinks")
        if path.is_file():
            if path.stat().st_mode & 0o222:
                raise VerificationError("source snapshot contains a writable file")
            actual.append(path.relative_to(snapshot_root).as_posix())
    if sorted(actual) != sorted(expected) or snapshot.get("file_count") != len(expected):
        raise VerificationError("source snapshot file set differs from the selected commit")
    for name in expected:
        if safe_file(snapshot_root, name).read_bytes() != git(repo, "show", f"{source_commit}:{name}", text=False):
            raise VerificationError(f"source snapshot file differs from commit: {name}")


def verify_attestation(repo, attestation_path, expected_main, expected_source_commit, expected_source_tree):
    repo = Path(repo).resolve()
    path = Path(attestation_path).resolve(strict=True)
    if not path.is_file():
        raise VerificationError("attestation path is not a regular file")
    record = json.loads(path.read_text())
    if record.get("schema") != "nagi-spike-attestation/2" or record.get("mode") != "build-and-run":
        raise VerificationError("unsupported attestation schema/mode")
    commit, tree, main = check_source(repo, record, expected_main, expected_source_commit, expected_source_tree)
    output = path.parent.resolve()
    check_manifest(output, record)
    check_source_snapshot(repo, output, record, commit)
    toolchain = record.get("toolchain", {})
    toolchain_bytes = git(repo, "show", f"{commit}:{SPIKE_REL}/rust-toolchain.toml", text=False)
    match = re.search(rb'(?m)^\s*channel\s*=\s*"([^"]+)"\s*$', toolchain_bytes)
    if not match or toolchain.get("rust_toolchain_toml") != match.group(1).decode():
        raise VerificationError("toolchain record does not match committed rust-toolchain.toml")
    if hashlib.sha256(toolchain_bytes).hexdigest() != toolchain.get("rust_toolchain_toml_sha256"):
        raise VerificationError("rust-toolchain.toml digest differs from source commit")
    if not (toolchain.get("rustup_active") == toolchain.get("rust_toolchain_toml") or
            str(toolchain.get("rustup_active", "")).startswith(toolchain.get("rust_toolchain_toml", "") + "-")):
        raise VerificationError("recorded active Rust toolchain differs from the pinned toolchain")
    for key in ("rustup_sha256", "cargo_sha256", "rustc_sha256", "rust_lld_sha256", "python_sha256"):
        if not HEX_SHA256.fullmatch(str(toolchain.get(key, ""))):
            raise VerificationError(f"toolchain hash is missing or malformed: {key}")
    for key in ("rustup_path", "cargo_path", "rustc_path", "rust_lld_path", "python_path"):
        if not isinstance(toolchain.get(key), str) or not Path(toolchain[key]).is_absolute():
            raise VerificationError(f"toolchain executable path is incomplete: {key}")
    builds = check_builds(output, record, commit, tree, toolchain)
    reproducibility = record.get("build_reproducibility", {})
    matches = builds[0]["elf_sha256"] == builds[1]["elf_sha256"]
    comparison = "MATCHED" if matches else "DIFFERENT"
    if reproducibility.get("two_builds_match") is not matches or reproducibility.get("status") != comparison:
        raise VerificationError("two-build comparison does not match the recorded ELF hashes")
    if record.get("acceptance", {}).get("two_build_comparison") != comparison:
        raise VerificationError("two-build acceptance summary does not match ELF hashes")
    check_runs(output, record, builds, toolchain)
    return {
        "main": main, "source": commit, "tree": tree, "files": len(record["files"]),
        "receipt_status": "REPRODUCIBILITY_FAILED" if comparison == "DIFFERENT" else "RECEIPT_INTEGRITY_VERIFIED",
        "two_build_comparison": comparison, "phase_c": "PENDING", "independent_rebuild": "NOT_RUN",
    }

def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--attestation", type=Path, required=True)
    parser.add_argument("--main", required=True, help="explicit full baseline SHA; no moving refs")
    parser.add_argument("--source-commit", required=True, help="expected full source commit SHA")
    parser.add_argument("--source-tree", required=True, help="expected full source tree SHA")
    args = parser.parse_args(argv)
    try:
        result = verify_attestation(args.repo, args.attestation, args.main, args.source_commit, args.source_tree)
    except (VerificationError, OSError, KeyError, ValueError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    if result["receipt_status"] == "REPRODUCIBILITY_FAILED":
        print(f"RECEIPT INTEGRITY VERIFIED, REPRODUCIBILITY FAILED; main={result['main']} source={result['source']} tree={result['tree']}; phase C=PENDING; independent rebuild=NOT_RUN")
        return 2
    print(f"RECEIPT INTEGRITY VERIFIED (not phase-C acceptance); main={result['main']} source={result['source']} tree={result['tree']}; {result['files']} manifested files; two-build comparison=MATCHED; phase C=PENDING; independent rebuild=NOT_RUN")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
