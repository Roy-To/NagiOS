#!/usr/bin/env python3
"""Build and run a source-bound ARM64 spike receipt. No existing ELF input exists."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

from run import expected_el2_rejection


ROOT = Path(__file__).resolve().parents[2]
SPIKE = Path(__file__).resolve().parent
SPIKE_REL = "experiments/nagi-aarch64-spike"
CARGO_COMMAND = ["cargo", "build", "--release", "--locked", "--offline"]
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
LOCAL_GUARD_ROOTS = (".codex", ".entire")
FORBIDDEN_EXACT = {
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUST_TARGET_PATH",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTUP_TOOLCHAIN",
    "CARGO_TARGET_DIR",
}
FORBIDDEN_PREFIXES = ("CARGO_BUILD_", "CARGO_PROFILE_")
ALLOWED_SPIKE_IGNORED_ROOTS = {"target", "out", "__pycache__"}


class AttestationError(ValueError):
    pass


def run(command, cwd, env=None, timeout=None, text=True):
    return subprocess.run(
        command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        check=False, timeout=timeout, text=text,
    )


def git(root, *args, check=True):
    result = run(["git", *args], root, text=True)
    if check and result.returncode != 0:
        raise AttestationError(result.stderr.strip() or f"git {' '.join(args)} failed")
    return result.stdout.strip()


def full_sha(value, label):
    if not isinstance(value, str) or not FULL_SHA.fullmatch(value):
        raise AttestationError(f"{label} must be a full 40-character lowercase SHA")
    return value


def resolve_git_object(root, value, kind, label):
    full_sha(value, label)
    resolved = git(root, "rev-parse", "--verify", f"{value}^{{{kind}}}", check=False)
    if resolved != value:
        raise AttestationError(f"{label} must resolve exactly to the supplied {kind} SHA")
    return resolved


def validate_worktree(root):
    root = Path(root).resolve()
    head = git(root, "rev-parse", "HEAD")
    tracked = git(root, "diff", "--name-only", "HEAD").splitlines()
    staged = git(root, "diff", "--cached", "--name-only", "HEAD").splitlines()
    untracked = git(root, "ls-files", "--others", "--exclude-standard").splitlines()
    guard = sorted(
        path for path in untracked
        if any(path == name or path.startswith(name + "/") for name in LOCAL_GUARD_ROOTS)
    )
    unexpected = sorted(set(untracked) - set(guard))
    if tracked or staged:
        raise AttestationError("source worktree has tracked modifications")
    if unexpected:
        raise AttestationError("source worktree has unexpected untracked paths: " + ", ".join(unexpected))

    ignored = git(root, "ls-files", "--others", "--ignored", "--exclude-standard", "-z").split("\0")
    ignored = [path for path in ignored if path and path.startswith(SPIKE_REL + "/")]
    disallowed_ignored = []
    for path in ignored:
        relative = path[len(SPIKE_REL) + 1:]
        if relative.split("/", 1)[0] not in ALLOWED_SPIKE_IGNORED_ROOTS:
            disallowed_ignored.append(path)
    if disallowed_ignored:
        raise AttestationError("unexpected ignored spike files: " + ", ".join(sorted(disallowed_ignored)))
    return {
        "head": head,
        "tracked_clean": True,
        "source_clean": True,
        "status_porcelain": git(root, "status", "--porcelain=v1", "--untracked-files=all").rstrip("\n"),
        "local_guard_untracked_roots": sorted({path.split("/", 1)[0] for path in guard}),
        "local_guard_untracked_count": len(guard),
        "ignored_paths": sorted(ignored),
    }


def validate_source(root, source_commit, source_tree, main):
    root = Path(root).resolve()
    source_commit = resolve_git_object(root, source_commit, "commit", "--source-commit")
    source_tree = resolve_git_object(root, source_tree, "tree", "--source-tree")
    main = resolve_git_object(root, main, "commit", "--main")
    actual_head = git(root, "rev-parse", "HEAD")
    actual_tree = git(root, "rev-parse", "HEAD^{tree}")
    if actual_head != source_commit:
        raise AttestationError("HEAD does not match --source-commit")
    if actual_tree != source_tree:
        raise AttestationError("HEAD tree does not match --source-tree")
    source_head_tree = git(root, "rev-parse", f"{source_commit}^{{tree}}")
    if source_head_tree != source_tree:
        raise AttestationError("source tree does not match source commit")
    if run(["git", "merge-base", "--is-ancestor", main, source_commit], root).returncode != 0:
        raise AttestationError("--main is not an ancestor of --source-commit")
    status = validate_worktree(root)
    subtree = git(root, "rev-parse", f"{source_commit}:{SPIKE_REL}")
    main_base = git(root, "merge-base", source_commit, main)
    if main_base != main:
        raise AttestationError("source commit does not descend from the explicit main baseline")
    return {
        "commit": source_commit,
        "tree": source_tree,
        "main": main,
        "spike_subtree": subtree,
        "main_base": main_base,
        "tracked_clean": status["tracked_clean"],
        "source_clean": status["source_clean"],
        "status_porcelain": status["status_porcelain"],
        "local_guard_untracked_roots": status["local_guard_untracked_roots"],
        "local_guard_untracked_count": status["local_guard_untracked_count"],
        "ignored_paths": status["ignored_paths"],
    }


def validate_output(root, output, storage_root=None):
    if storage_root is None:
        storage_root = os.environ.get("NAGI_STORAGE_ROOT")
    if not storage_root:
        raise AttestationError("NAGI_STORAGE_ROOT is required; run through the SSD guard")
    storage = Path(storage_root).expanduser().resolve(strict=True)
    raw = Path(output).expanduser()
    if not raw.is_absolute():
        raise AttestationError("--output must be an absolute path on the approved SSD")
    if raw.is_symlink():
        raise AttestationError("--output must not be a symlink")
    resolved = raw.resolve()
    if not resolved.is_relative_to(storage):
        raise AttestationError("--output must be under NAGI_STORAGE_ROOT")
    if resolved.is_relative_to(Path(root).resolve()):
        raise AttestationError("--output must be outside the source repository")
    if resolved.exists():
        raise AttestationError("--output must not already exist")
    return resolved


def forbidden_environment(env):
    keys = sorted(
        key for key in env
        if key in FORBIDDEN_EXACT or any(key.startswith(prefix) for prefix in FORBIDDEN_PREFIXES)
    )
    if keys:
        raise AttestationError("build environment overrides are forbidden: " + ", ".join(keys))
    return sorted(FORBIDDEN_EXACT) + list(FORBIDDEN_PREFIXES)


def resolve_qemu(value):
    found = shutil.which(str(value))
    candidate = found or str(value)
    path = Path(candidate).expanduser().resolve(strict=True)
    if not path.is_file() or not os.access(path, os.X_OK):
        raise AttestationError(f"QEMU is not an executable file: {path}")
    return path


def hash_bytes(data):
    return hashlib.sha256(data).hexdigest()


def hash_file(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def toolchain_record():
    toolchain = (SPIKE / "rust-toolchain.toml").read_text()
    match = re.search(r'(?m)^\s*channel\s*=\s*"([^"]+)"\s*$', toolchain)
    if not match:
        raise AttestationError("rust-toolchain.toml has no pinned channel")
    pinned = match.group(1)
    active = subprocess.check_output(
        ["rustup", "show", "active-toolchain"], cwd=SPIKE, text=True
    ).strip().split()[0]
    if not (active == pinned or active.startswith(pinned + "-")):
        raise AttestationError(f"active Rust toolchain does not match {pinned}")
    rustc_vv = subprocess.check_output(["rustc", "-Vv"], cwd=SPIKE, text=True).strip()
    cargo_v = subprocess.check_output(["cargo", "-V"], cwd=SPIKE, text=True).strip()
    rustc_path = Path(subprocess.check_output(["rustup", "which", "rustc"], cwd=SPIKE, text=True).strip()).resolve(strict=True)
    cargo_path = Path(subprocess.check_output(["rustup", "which", "cargo"], cwd=SPIKE, text=True).strip()).resolve(strict=True)
    host = None
    for line in rustc_vv.splitlines():
        if line.startswith("host: "):
            host = line.split(":", 1)[1].strip()
            break
    if not host:
        raise AttestationError("rustc -Vv did not report a host triple")
    sysroot = Path(subprocess.check_output(["rustc", "--print", "sysroot"], cwd=SPIKE, text=True).strip()).resolve(strict=True)
    lld_candidates = [
        sysroot / "lib/rustlib" / host / "bin/rust-lld",
        sysroot / "bin/rust-lld",
    ]
    lld_path = next((path for path in lld_candidates if path.is_file()), None)
    if lld_path is None:
        found = shutil.which("rust-lld")
        lld_path = Path(found).resolve(strict=True) if found else None
    if lld_path is None:
        raise AttestationError("pinned toolchain rust-lld executable is unavailable")
    return {
        "rust_toolchain_toml": pinned,
        "rust_toolchain_toml_sha256": hash_bytes(toolchain.encode()),
        "rustup_active": active,
        "rustc_vV": rustc_vv,
        "cargo_v": cargo_v,
        "cargo_path": str(cargo_path),
        "cargo_sha256": hash_file(cargo_path),
        "rustc_path": str(rustc_path),
        "rustc_sha256": hash_file(rustc_path),
        "rust_lld_sha256": hash_file(lld_path),
    }


def object_identity(root, source_commit, relative):
    blob = git(root, "rev-parse", f"{source_commit}:{relative}")
    path = Path(root) / relative
    return {"path": relative, "git_blob": blob, "sha256": hash_file(path)}


def build_once(output, label, env_base):
    temp_dir = output / ".build" / label
    target = temp_dir / "target"
    cargo_home = temp_dir / "cargo-home"
    target.mkdir(parents=True)
    cargo_home.mkdir(parents=True)
    env = dict(env_base)
    env["CARGO_HOME"] = str(cargo_home)
    env["CARGO_TARGET_DIR"] = str(target)
    env["CARGO_NET_OFFLINE"] = "true"
    try:
        result = run(CARGO_COMMAND, SPIKE, env=env, timeout=1800, text=False)
        build_dir = output / f"build-{label}"
        build_dir.mkdir()
        (build_dir / "stdout.log").write_bytes(result.stdout)
        (build_dir / "stderr.log").write_bytes(result.stderr)
        if result.returncode != 0:
            raise AttestationError(f"cargo build {label} failed with exit {result.returncode}")
        elf = target / "aarch64-unknown-none/release/nagi-aarch64-feasibility-spike"
        if not elf.is_file():
            raise AttestationError(f"cargo build {label} did not produce the expected ELF")
        data = elf.read_bytes()
        if not (
            len(data) >= 20 and data[:6] == b"\x7fELF\x02\x01"
            and int.from_bytes(data[18:20], "little") == 183
        ):
            raise AttestationError(f"cargo build {label} did not produce ELF64 little-endian AArch64")
        retained = build_dir / "nagi-aarch64-feasibility-spike"
        shutil.copyfile(elf, retained)
        record = {
            "id": label,
            "source_commit": git(ROOT, "rev-parse", "HEAD"),
            "source_tree": git(ROOT, "rev-parse", "HEAD^{tree}"),
            "argv": CARGO_COMMAND,
            "cwd": SPIKE_REL,
            "target_dir": f".build/{label}/target",
            "cargo_home": f".build/{label}/cargo-home",
            "exit_code": result.returncode,
            "stdout_path": f"build-{label}/stdout.log",
            "stdout_sha256": hash_file(build_dir / "stdout.log"),
            "stderr_path": f"build-{label}/stderr.log",
            "stderr_sha256": hash_file(build_dir / "stderr.log"),
            "elf_path": f"build-{label}/nagi-aarch64-feasibility-spike",
            "elf_sha256": hash_file(retained),
            "elf_bytes": len(data),
            "elf_machine": 183,
        }
        return record
    except subprocess.TimeoutExpired as exc:
        temp_dir.parent.mkdir(parents=True, exist_ok=True)
        build_dir = output / f"build-{label}"
        build_dir.mkdir(exist_ok=True)
        stdout = exc.stdout or b""
        stderr = exc.stderr or b""
        (build_dir / "stdout.log").write_bytes(stdout if isinstance(stdout, bytes) else stdout.encode())
        (build_dir / "stderr.log").write_bytes(stderr if isinstance(stderr, bytes) else stderr.encode())
        raise AttestationError(f"cargo build {label} timed out; see build-{label} logs")
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)
        parent = temp_dir.parent
        if parent.exists() and not any(parent.iterdir()):
            parent.rmdir()


def run_one(output, build_record, qemu, label, virtualization):
    binary = output / build_record["elf_path"]
    run_dir = output / "runs" / label
    command = [
        sys.executable,
        str(SPIKE / "run.py"),
        "--qemu", str(qemu),
        "--binary", str(binary),
        "--virtualization", virtualization,
        "--output", str(run_dir),
    ]
    result = run(command, SPIKE, timeout=30, text=False)
    log_path = output / f"runner-{label}.stdout.log"
    err_path = output / f"runner-{label}.stderr.log"
    log_path.write_bytes(result.stdout)
    err_path.write_bytes(result.stderr)
    report_path = run_dir / "result.json"
    if not report_path.is_file():
        raise AttestationError(f"runner {label} did not write result.json; see runner logs")
    report = json.loads(report_path.read_text())
    return {
        "id": label,
        "virtualization": virtualization,
        "argv": command,
        "runner_exit_code": result.returncode,
        "runner_stdout_path": log_path.relative_to(output).as_posix(),
        "runner_stdout_sha256": hash_file(log_path),
        "runner_stderr_path": err_path.relative_to(output).as_posix(),
        "runner_stderr_sha256": hash_file(err_path),
        "result_path": report_path.relative_to(output).as_posix(),
        "result_json_sha256": hash_file(report_path),
        "serial_path": (run_dir / "serial.log").relative_to(output).as_posix(),
        "serial_sha256": hash_file(run_dir / "serial.log"),
        "stderr_path": (run_dir / "stderr.log").relative_to(output).as_posix(),
        "stderr_sha256": hash_file(run_dir / "stderr.log"),
        "binary_sha256": report.get("binary_sha256"),
        "binary_sha256_after": report.get("binary_sha256_after"),
        "qemu_sha256": report.get("qemu_sha256"),
        "qemu_sha256_after": report.get("qemu_sha256_after"),
        "qemu_path": report.get("qemu_path"),
        "runner_outcome": report.get("outcome"),
        "returncode": report.get("returncode"),
        "timed_out": report.get("timed_out"),
        "derived_verdict": None,
    }


def derive_run_verdict(run_record, output):
    result = json.loads((output / run_record["result_path"]).read_text())
    serial = (output / run_record["serial_path"]).read_bytes()
    if run_record["id"] == "el1":
        passed = (
            run_record["runner_exit_code"] == 0
            and result.get("outcome") == "PASS"
            and result.get("returncode") == 0
            and not result.get("timed_out")
            and result.get("binary_sha256") == result.get("binary_sha256_after")
            and result.get("qemu_sha256") == result.get("qemu_sha256_after")
        )
        return "PASS" if passed else "NOT_MET"
    if run_record["id"] == "el2":
        met = (
            run_record["runner_exit_code"] == 1
            and result.get("outcome") == "FAIL"
            and expected_el2_rejection(
                result.get("returncode"), serial, bool(result.get("timed_out"))
            )
            and result.get("binary_sha256") == result.get("binary_sha256_after")
            and result.get("qemu_sha256") == result.get("qemu_sha256_after")
        )
        return "MET" if met else "NOT_MET"
    raise AttestationError(f"unknown run id {run_record['id']}")


def file_manifest(output):
    manifest = {}
    for path in sorted(output.rglob("*")):
        if path.is_symlink():
            raise AttestationError(f"symlink is not allowed in attestation output: {path}")
        if path.is_file() and path.name != "attestation.json":
            manifest[path.relative_to(output).as_posix()] = hash_file(path)
    return manifest


def attest(args):
    repo = Path(git(ROOT, "rev-parse", "--show-toplevel")).resolve()
    if repo != ROOT.resolve():
        raise AttestationError("spike is not inside the expected repository")
    source = validate_source(repo, args.source_commit, args.source_tree, args.main)
    output = validate_output(repo, args.output)
    forbidden_checked = forbidden_environment(os.environ)
    qemu = resolve_qemu(args.qemu or os.environ.get("QEMU_AARCH64", "qemu-system-aarch64"))
    toolchain = toolchain_record()

    output.parent.mkdir(parents=True, exist_ok=True)
    output.mkdir()
    builds = [build_once(output, "a", os.environ), build_once(output, "b", os.environ)]
    build_match = builds[0]["elf_sha256"] == builds[1]["elf_sha256"]
    qemu_sha = hash_file(qemu)
    qemu_version = subprocess.check_output([str(qemu), "--version"], text=True).splitlines()[0]
    runs = [
        run_one(output, builds[0], qemu, "el1", "off"),
        run_one(output, builds[0], qemu, "el2", "on"),
    ]
    for item in runs:
        item["derived_verdict"] = derive_run_verdict(item, output)
    if runs[0]["derived_verdict"] != "PASS" or runs[1]["derived_verdict"] != "MET":
        raise AttestationError("EL1 or expected-negative EL2 run did not meet its derived condition")
    if any(item["qemu_sha256"] != qemu_sha or item["qemu_sha256_after"] != qemu_sha for item in runs):
        raise AttestationError("QEMU executable identity changed during the runs")
    source_after = validate_source(repo, args.source_commit, args.source_tree, args.main)
    if source_after != source:
        raise AttestationError("source identity or worktree status changed during build/run")

    cargo_toml = (SPIKE / "Cargo.toml").read_text()
    lock_text = (SPIKE / "Cargo.lock").read_text()
    external_lock_packages = len(re.findall(r"(?m)^\[\[package\]\]\s*$", lock_text)) - 1
    attestation = {
        "schema": "nagi-spike-attestation/1",
        "mode": "build-and-run",
        "generator": object_identity(repo, args.source_commit, f"{SPIKE_REL}/attest.py"),
        "runner": object_identity(repo, args.source_commit, f"{SPIKE_REL}/run.py"),
        "source": source | {
            "head_after_build": source_after["head"],
            "tree_after_build": args.source_tree,
        },
        "toolchain": toolchain,
        "environment": {
            "forbidden_checked": forbidden_checked,
            "forbidden_present": [],
            "cargo_home_isolated": True,
            "cargo_offline": True,
            "locked_package_count": external_lock_packages + 1,
            "external_dependencies_declared": bool(re.search(
                r"(?m)^\s*\[(?:dev-|build-)?dependencies\]\s*$|^\s*\[target\..*\.(?:dev-|build-)?dependencies\]\s*$",
                cargo_toml,
            )),
        },
        "builds": builds,
        "build_reproducibility": {
            "two_builds_match": build_match,
            "interpretation": (
                "same committed source checkout and pinned toolchain, separate empty target/CARGO_HOME directories; "
                "this is a same-host comparison, not a path-independent reproducibility proof"
            ),
        },
        "qemu": {
            "path_realpath": str(qemu),
            "sha256": qemu_sha,
            "version": qemu_version,
        },
        "runs": runs,
        "acceptance": {
            "el1": "PASS",
            "el2_negative": "MET",
            "source_bound_runs": "PASS",
            "two_build_comparison": "MATCHED" if build_match else "DIFFERENT",
            "independent_rebuild": "NOT_RUN",
            "checkpoint": "PENDING_INDEPENDENT_REBUILD",
        },
    }
    attestation["files"] = file_manifest(output)
    attestation_path = output / "attestation.json"
    attestation_path.write_text(json.dumps(attestation, indent=2) + "\n")
    return attestation


def build_parser():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--source-tree", required=True)
    parser.add_argument("--main", required=True, help="explicit full current-main commit SHA")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--qemu", help="QEMU executable; defaults to QEMU_AARCH64 or qemu-system-aarch64")
    return parser


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        record = attest(args)
    except (AttestationError, OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    print(
        "SOURCE-BOUND RUNS: PASS; "
        f"two-build comparison={record['build_reproducibility']['two_builds_match']}; "
        "full checkpoint remains PENDING_INDEPENDENT_REBUILD"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
