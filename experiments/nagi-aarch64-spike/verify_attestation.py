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


def check_source(root, record, expected_main=None):
    source = record.get("source", {})
    main = source.get("main")
    commit = full_commit(root, source.get("commit"), "source.commit")
    tree = source.get("tree")
    if not isinstance(tree, str) or not FULL_SHA.fullmatch(tree):
        raise VerificationError("source.tree must be a full tree SHA")
    if git(root, "rev-parse", f"{commit}^{{tree}}").strip() != tree:
        raise VerificationError("source tree does not match its commit")
    main = full_commit(root, main, "source.main")
    if expected_main is not None and full_commit(root, expected_main, "--main") != main:
        raise VerificationError("attestation main differs from explicit --main")
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


def check_builds(output, record, commit, tree):
    builds = record.get("builds")
    if not isinstance(builds, list) or [item.get("id") for item in builds] != ["a", "b"]:
        raise VerificationError("attestation must contain exactly build a and build b")
    expected_command = ["cargo", "build", "--release", "--locked", "--offline"]
    for build in builds:
        if build.get("argv") != expected_command or build.get("exit_code") != 0:
            raise VerificationError("build command/result is not a successful locked offline release build")
        for field in ("stdout_path", "stderr_path", "elf_path"):
            path = safe_file(output, build.get(field))
            if not path.is_file():
                raise VerificationError(f"build artifact is missing: {build.get(field)}")
        for path_field, hash_field in (("stdout_path", "stdout_sha256"), ("stderr_path", "stderr_sha256"), ("elf_path", "elf_sha256")):
            if sha_file(safe_file(output, build[path_field])) != build.get(hash_field):
                raise VerificationError(f"build artifact digest mismatch: {build[path_field]}")
        elf = safe_file(output, build["elf_path"]).read_bytes()
        if not valid_elf(elf) or build.get("elf_machine") != 183:
            raise VerificationError("build artifact is not ELF64 little-endian AArch64")
        if build.get("source_commit") != commit or build.get("source_tree") != tree:
            raise VerificationError("build block is not bound to source identity")
    return builds


def check_runs(output, record, builds):
    runs = record.get("runs")
    if not isinstance(runs, list) or [item.get("id") for item in runs] != ["el1", "el2"]:
        raise VerificationError("attestation must contain EL1 and EL2 runs")
    qemu = record.get("qemu", {})
    if not HEX_SHA256.fullmatch(str(qemu.get("sha256", ""))) or not qemu.get("path_realpath"):
        raise VerificationError("QEMU identity is incomplete")
    for item in runs:
        if item.get("runner_exit_code") != (0 if item["id"] == "el1" else 1):
            raise VerificationError("runner exit code does not match the expected branch")
        for path_key, hash_key in (
            ("result_path", "result_json_sha256"),
            ("serial_path", "serial_sha256"),
            ("stderr_path", "stderr_sha256"),
            ("runner_stdout_path", "runner_stdout_sha256"),
            ("runner_stderr_path", "runner_stderr_sha256"),
        ):
            path = safe_file(output, item.get(path_key))
            if not path.is_file() or sha_file(path) != item.get(hash_key):
                raise VerificationError(f"run artifact missing or hash mismatch: {item.get(path_key)}")
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
        if "--qemu" not in argv or argv.count("--qemu") != 1:
            raise VerificationError("run command must name exactly one attested QEMU executable")
        if Path(argv[argv.index("--qemu") + 1]).resolve() != Path(qemu["path_realpath"]).resolve():
            raise VerificationError("run command QEMU path differs from attested QEMU")
        if "--binary" not in argv or argv.count("--binary") != 1:
            raise VerificationError("run command must name exactly one build artifact")
        binary_arg = Path(argv[argv.index("--binary") + 1]).resolve()
        expected_binary = safe_file(output, builds[0]["elf_path"]).resolve()
        if binary_arg != expected_binary or Path(result.get("binary_path", "")).resolve() != expected_binary:
            raise VerificationError("run did not use the ELF generated in this attestation output")
        serial_path = safe_file(output, item["serial_path"])
        stderr_path = safe_file(output, item["stderr_path"])
        if result.get("serial_sha256") != sha_file(serial_path) or result.get("stderr_sha256") != sha_file(stderr_path):
            raise VerificationError("runner serial/stderr digest differs from saved output")
        expected_virtualization = "off" if item["id"] == "el1" else "on"
        if result.get("virtualization") != expected_virtualization or item.get("virtualization") != expected_virtualization:
            raise VerificationError("run virtualization mode does not match its expected EL")
        if item["id"] == "el1":
            if result.get("outcome") != "PASS" or result.get("returncode") != 0 or result.get("timed_out") is not False:
                raise VerificationError("EL1 runner result is not a complete PASS")
            if not accepted(result.get("returncode"), serial, result.get("timed_out") is True):
                raise VerificationError("EL1 serial does not satisfy the complete marker sequence")
            if result.get("binary_sha256") != builds[0].get("elf_sha256"):
                raise VerificationError("EL1 did not run build a's ELF")
        else:
            if result.get("outcome") != "FAIL" or item.get("derived_verdict") != "MET":
                raise VerificationError("EL2 must remain runner FAIL while its negative condition is met")
            if not expected_el2_rejection(result.get("returncode"), serial, result.get("timed_out") is True):
                raise VerificationError("EL2 did not meet the expected-negative condition")
            if result.get("binary_sha256") != builds[0].get("elf_sha256"):
                raise VerificationError("EL2 did not run build a's ELF")
    acceptance = record.get("acceptance", {})
    if acceptance.get("el1") != "PASS" or acceptance.get("el2_negative") != "MET":
        raise VerificationError("acceptance summary does not match verified run records")
    if acceptance.get("independent_rebuild") != "NOT_RUN":
        raise VerificationError("independent rebuild must remain NOT_RUN until performed")


def verify_attestation(repo, attestation_path, expected_main=None):
    repo = Path(repo).resolve()
    path = Path(attestation_path).resolve(strict=True)
    if not path.is_file():
        raise VerificationError("attestation path is not a regular file")
    record = json.loads(path.read_text())
    if record.get("schema") != "nagi-spike-attestation/1" or record.get("mode") != "build-and-run":
        raise VerificationError("unsupported attestation schema/mode")
    source_commit, source_tree, main = check_source(repo, record, expected_main)
    toolchain = record.get("toolchain", {})
    toolchain_bytes = git(repo, "show", f"{source_commit}:{SPIKE_REL}/rust-toolchain.toml", text=False)
    match = re.search(rb'(?m)^\s*channel\s*=\s*"([^"]+)"\s*$', toolchain_bytes)
    if not match or toolchain.get("rust_toolchain_toml") != match.group(1).decode():
        raise VerificationError("toolchain record does not match committed rust-toolchain.toml")
    if hashlib.sha256(toolchain_bytes).hexdigest() != toolchain.get("rust_toolchain_toml_sha256"):
        raise VerificationError("rust-toolchain.toml digest differs from the source commit")
    if not (toolchain.get("rustup_active") == toolchain.get("rust_toolchain_toml")
            or str(toolchain.get("rustup_active", "")).startswith(toolchain.get("rust_toolchain_toml", "") + "-")):
        raise VerificationError("recorded active Rust toolchain differs from the pinned toolchain")
    for key in ("cargo_sha256", "rustc_sha256", "rust_lld_sha256"):
        if not HEX_SHA256.fullmatch(str(toolchain.get(key, ""))):
            raise VerificationError(f"toolchain hash is missing or malformed: {key}")
    if not toolchain.get("cargo_path") or not toolchain.get("rustc_path"):
        raise VerificationError("toolchain executable paths are incomplete")
    output = path.parent.resolve()
    check_manifest(output, record)
    builds = check_builds(output, record, source_commit, source_tree)
    comparison = record.get("build_reproducibility", {})
    same_build = builds[0]["elf_sha256"] == builds[1]["elf_sha256"]
    if comparison.get("two_builds_match") is not same_build:
        raise VerificationError("two-build comparison does not match ELF hashes")
    expected_comparison = "MATCHED" if same_build else "DIFFERENT"
    if record.get("acceptance", {}).get("two_build_comparison") != expected_comparison:
        raise VerificationError("two-build summary does not match ELF hashes")
    check_runs(output, record, builds)
    return {"main": main, "source": source_commit, "tree": source_tree, "files": len(record["files"])}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--attestation", type=Path, required=True)
    parser.add_argument("--main", required=True, help="explicit full SHA; no moving refs")
    args = parser.parse_args(argv)
    try:
        result = verify_attestation(args.repo, args.attestation, args.main)
    except (VerificationError, OSError, KeyError, ValueError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    print(f"PASS receipt integrity; main={result['main']} source={result['source']} tree={result['tree']}; {result['files']} manifested files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
