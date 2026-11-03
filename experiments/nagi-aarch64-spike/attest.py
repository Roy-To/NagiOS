#!/usr/bin/env python3
"""Build and run a source-bound ARM64 spike receipt. No existing ELF input exists."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tarfile

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
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_DIST_SERVER",
    "CARGO_NET_OFFLINE",
    "CARGO_TERM_COLOR",
}
FORBIDDEN_PREFIXES = ("CARGO_BUILD_", "CARGO_PROFILE_", "CARGO_TARGET_", "CARGO_UNSTABLE_")
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
    return sorted(
        key for key in env
        if key in FORBIDDEN_EXACT or any(key.startswith(prefix) for prefix in FORBIDDEN_PREFIXES)
    )


def build_environment(temp_dir, target, cargo_home, toolchain):
    temp_dir = Path(temp_dir)
    home = temp_dir / "home"
    temp = temp_dir / "tmp"
    home.mkdir(parents=True, exist_ok=True)
    temp.mkdir(parents=True, exist_ok=True)
    paths = [
        str(Path(toolchain["cargo_path"]).parent),
        str(Path(toolchain["rust_lld_path"]).parent),
        "/usr/bin",
        "/bin",
    ]
    return {
        "HOME": str(home), "TMPDIR": str(temp), "TMP": str(temp), "TEMP": str(temp),
        "PATH": os.pathsep.join(dict.fromkeys(paths)), "LANG": "C", "LC_ALL": "C",
        "CARGO_HOME": str(cargo_home), "CARGO_TARGET_DIR": str(target),
        "CARGO_NET_OFFLINE": "true", "CARGO_TERM_COLOR": "never",
        "RUSTC": str(toolchain["rustc_path"]),
    }


def runtime_environment(temp_dir, toolchain):
    temp_dir = Path(temp_dir)
    home = temp_dir / "home"
    temp = temp_dir / "tmp"
    home.mkdir(parents=True, exist_ok=True)
    temp.mkdir(parents=True, exist_ok=True)
    paths = [str(Path(toolchain["python_path"]).parent), "/usr/bin", "/bin"]
    return {
        "HOME": str(home), "TMPDIR": str(temp), "TMP": str(temp), "TEMP": str(temp),
        "PATH": os.pathsep.join(dict.fromkeys(paths)), "LANG": "C", "LC_ALL": "C",
        "PYTHONNOUSERSITE": "1", "PYTHONDONTWRITEBYTECODE": "1",
    }

def resolve_qemu(value):
    found = shutil.which(str(value))
    candidate = found or str(value)
    path = Path(candidate).expanduser().resolve(strict=True)
    if not path.is_file() or not os.access(path, os.X_OK):
        raise AttestationError(f"QEMU is not an executable file: {path}")
    return path


def hash_bytes(data):
    return hashlib.sha256(data).hexdigest()


def reject_ambient_cargo_config(cwd, cargo_home):
    cargo_home = Path(cargo_home).resolve()
    current = Path(cwd).resolve()
    for directory in (current, *current.parents):
        for name in ("config", "config.toml"):
            config = directory / ".cargo" / name
            if config.exists():
                resolved = config.resolve()
                if resolved.is_relative_to(cargo_home):
                    continue
                raise AttestationError("ambient Cargo config can inject build settings: " + str(config))


def hash_file(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def archive_spike_source(root, source_commit):
    result = run(["git", "archive", "--format=tar", source_commit, "--", SPIKE_REL], root, text=False)
    if result.returncode:
        raise AttestationError(result.stderr.decode("utf-8", "replace").strip() or "cannot archive source commit")
    expected = git(root, "ls-tree", "-r", "--name-only", source_commit, "--", SPIKE_REL).splitlines()
    observed = []
    try:
        with tarfile.open(fileobj=io.BytesIO(result.stdout), mode="r:") as archive:
            for member in archive.getmembers():
                path = PurePosixPath(member.name)
                if path.is_absolute() or any(part in ("", ".", "..") for part in path.parts):
                    raise AttestationError("source archive contains an unsafe path")
                if member.isdir() and path.as_posix() == "experiments":
                    continue
                if path.parts[:2] != tuple(SPIKE_REL.split("/")):
                    raise AttestationError("source archive contains a path outside the spike")
                if member.isdir():
                    continue
                if not member.isfile():
                    raise AttestationError("source archive contains a non-regular file")
                observed.append(path.as_posix())
    except (tarfile.TarError, OSError) as exc:
        raise AttestationError(f"cannot inspect source archive: {exc}") from exc
    if not expected or sorted(expected) != sorted(observed):
        raise AttestationError("git archive file set differs from the committed spike tree")
    return result.stdout, sorted(expected)


def materialize_spike_source(archive_bytes, expected_paths, destination):
    destination = Path(destination)
    if destination.exists():
        raise AttestationError("source snapshot destination already exists")
    destination.mkdir(parents=True)
    observed = []
    try:
        with tarfile.open(fileobj=io.BytesIO(archive_bytes), mode="r:") as archive:
            for member in archive.getmembers():
                path = PurePosixPath(member.name)
                if path.is_absolute() or any(part in ("", ".", "..") for part in path.parts):
                    raise AttestationError("source archive contains an unsafe path")
                if member.isdir() and path.as_posix() == "experiments":
                    continue
                if path.parts[:2] != tuple(SPIKE_REL.split("/")):
                    raise AttestationError("source archive contains a path outside the spike")
                target = destination.joinpath(*path.parts)
                if member.isdir():
                    target.mkdir(parents=True, exist_ok=True)
                    continue
                if not member.isfile() or path.as_posix() not in expected_paths:
                    raise AttestationError("source archive contains an unexpected file")
                stream = archive.extractfile(member)
                if stream is None:
                    raise AttestationError("source archive file cannot be read")
                target.parent.mkdir(parents=True, exist_ok=True)
                with stream, target.open("xb") as output:
                    shutil.copyfileobj(stream, output)
                target.chmod(0o444)
                observed.append(path.as_posix())
    except (tarfile.TarError, OSError) as exc:
        raise AttestationError(f"cannot materialize source snapshot: {exc}") from exc
    if sorted(observed) != sorted(expected_paths):
        raise AttestationError("materialized snapshot differs from the committed spike tree")
    for path in sorted(destination.rglob("*"), key=lambda item: len(item.parts), reverse=True):
        if path.is_symlink():
            raise AttestationError("source snapshot must not contain symlinks")
        if path.is_dir():
            path.chmod(0o555)
    destination.chmod(0o555)
    return destination / SPIKE_REL



def toolchain_record(source_spike):
    source_spike = Path(source_spike)
    toolchain = (source_spike / "rust-toolchain.toml").read_text()
    match = re.search(r'(?m)^\s*channel\s*=\s*"([^"]+)"\s*$', toolchain)
    if not match:
        raise AttestationError("rust-toolchain.toml has no pinned channel")
    pinned = match.group(1)
    rustup_found = shutil.which("rustup")
    if not rustup_found:
        raise AttestationError("rustup executable is unavailable")
    rustup = Path(rustup_found).resolve(strict=True)
    home = Path.home().resolve()
    rustup_home = Path(os.environ.get("RUSTUP_HOME", home / ".rustup")).expanduser().resolve()
    env = {
        "HOME": str(home), "RUSTUP_HOME": str(rustup_home),
        "PATH": os.pathsep.join((str(rustup.parent), "/usr/bin", "/bin")),
        "LANG": "C", "LC_ALL": "C",
    }

    def rustup_output(args):
        return subprocess.check_output([str(rustup), *args], cwd=source_spike, env=env, text=True).strip()

    active = rustup_output(["show", "active-toolchain"]).split()[0]
    if not (active == pinned or active.startswith(pinned + "-")):
        raise AttestationError(f"active Rust toolchain does not match {pinned}")
    rustc = Path(rustup_output(["which", "rustc"])).resolve(strict=True)
    cargo = Path(rustup_output(["which", "cargo"])).resolve(strict=True)
    tool_env = {
        "HOME": str(home), "RUSTUP_HOME": str(rustup_home),
        "PATH": os.pathsep.join((str(cargo.parent), str(rustc.parent), "/usr/bin", "/bin")),
        "LANG": "C", "LC_ALL": "C",
    }
    rustc_vv = subprocess.check_output([str(rustc), "-Vv"], cwd=source_spike, env=tool_env, text=True).strip()
    cargo_v = subprocess.check_output([str(cargo), "-V"], cwd=source_spike, env=tool_env, text=True).strip()
    host = next((line.split(":", 1)[1].strip() for line in rustc_vv.splitlines() if line.startswith("host: ")), None)
    if not host:
        raise AttestationError("rustc -Vv did not report a host triple")
    sysroot = Path(subprocess.check_output(
        [str(rustc), "--print", "sysroot"], cwd=source_spike, env=tool_env, text=True
    ).strip()).resolve(strict=True)
    lld = next((p for p in (
        sysroot / "lib/rustlib" / host / "bin/rust-lld",
        sysroot / "bin/rust-lld",
    ) if p.is_file()), None)
    if lld is None:
        raise AttestationError("pinned toolchain rust-lld executable is unavailable")
    python = Path(sys.executable).resolve(strict=True)
    return {
        "rust_toolchain_toml": pinned,
        "rust_toolchain_toml_sha256": hash_bytes(toolchain.encode()),
        "rustup_active": active,
        "rustup_path": str(rustup), "rustup_sha256": hash_file(rustup),
        "rustc_vV": rustc_vv, "cargo_v": cargo_v,
        "cargo_path": str(cargo), "cargo_sha256": hash_file(cargo),
        "rustc_path": str(rustc), "rustc_sha256": hash_file(rustc),
        "rust_lld_path": str(lld.resolve()), "rust_lld_sha256": hash_file(lld),
        "python_path": str(python), "python_version": sys.version.split()[0],
        "python_sha256": hash_file(python),
    }

def object_identity(root, source_commit, relative):
    blob = git(root, "rev-parse", f"{source_commit}:{relative}")
    raw = git(root, "show", f"{source_commit}:{relative}", text=False)
    return {"path": relative, "git_blob": blob, "sha256": hash_bytes(raw)}

def build_once(output, label, source_spike, source, archive_sha256, toolchain):
    temp_dir = output / ".build" / label
    target = temp_dir / "target"
    cargo_home = temp_dir / "cargo-home"
    target.mkdir(parents=True)
    cargo_home.mkdir(parents=True)
    env = build_environment(temp_dir, target, cargo_home, toolchain)
    reject_ambient_cargo_config(source_spike, cargo_home)
    command = [toolchain["cargo_path"], *CARGO_COMMAND[1:]]
    build_dir = output / f"build-{label}"
    try:
        result = run(command, source_spike, env=env, timeout=1800, text=False)
        build_dir.mkdir()
        (build_dir / "stdout.log").write_bytes(result.stdout)
        (build_dir / "stderr.log").write_bytes(result.stderr)
        if result.returncode:
            raise AttestationError(f"cargo build {label} failed with exit {result.returncode}")
        elf = target / "aarch64-unknown-none/release/nagi-aarch64-feasibility-spike"
        if not elf.is_file():
            raise AttestationError(f"cargo build {label} did not produce the expected ELF")
        data = elf.read_bytes()
        if len(data) < 20 or data[:6] != b"\x7fELF\x02\x01" or int.from_bytes(data[18:20], "little") != 183:
            raise AttestationError(f"cargo build {label} did not produce ELF64 little-endian AArch64")
        retained = build_dir / "nagi-aarch64-feasibility-spike"
        shutil.copyfile(elf, retained)
        return {
            "id": label, "source_commit": source["commit"], "source_tree": source["tree"],
            "source_archive_sha256": archive_sha256, "argv": command,
            "cwd": "source-snapshot/" + SPIKE_REL,
            "target_dir": f".build/{label}/target", "cargo_home": f".build/{label}/cargo-home",
            "cargo_home_seed": "empty",
            "environment_policy": "minimal-allowlist; inherited build variables discarded",
            "cargo_config_policy": "reject external .cargo/config and .cargo/config.toml in cwd ancestors; empty isolated CARGO_HOME",
            "exit_code": result.returncode,
            "stdout_path": f"build-{label}/stdout.log", "stdout_sha256": hash_file(build_dir / "stdout.log"),
            "stderr_path": f"build-{label}/stderr.log", "stderr_sha256": hash_file(build_dir / "stderr.log"),
            "elf_path": f"build-{label}/nagi-aarch64-feasibility-spike",
            "elf_sha256": hash_file(retained), "elf_bytes": len(data), "elf_machine": 183,
        }
    except subprocess.TimeoutExpired as exc:
        build_dir.mkdir(exist_ok=True)
        (build_dir / "stdout.log").write_bytes(exc.stdout or b"")
        (build_dir / "stderr.log").write_bytes(exc.stderr or b"")
        raise AttestationError(f"cargo build {label} timed out; see build-{label} logs")
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)
        parent = temp_dir.parent
        if parent.exists() and not any(parent.iterdir()):
            parent.rmdir()

def run_one(output, build_record, qemu, label, virtualization, source_spike, toolchain):
    binary = output / build_record["elf_path"]
    run_dir = output / "runs" / label
    command = [
        toolchain["python_path"], "run.py", "--qemu", str(qemu),
        "--binary", str(binary), "--virtualization", virtualization,
        "--output", str(run_dir),
    ]
    runtime_dir = output / ".runtime" / label
    try:
        result = run(command, source_spike, env=runtime_environment(runtime_dir, toolchain), timeout=30, text=False)
    finally:
        shutil.rmtree(runtime_dir, ignore_errors=True)
        if runtime_dir.parent.exists() and not any(runtime_dir.parent.iterdir()):
            runtime_dir.parent.rmdir()
    log_path = output / f"runner-{label}.stdout.log"
    err_path = output / f"runner-{label}.stderr.log"
    log_path.write_bytes(result.stdout)
    err_path.write_bytes(result.stderr)
    report_path = run_dir / "result.json"
    if not report_path.is_file():
        raise AttestationError(f"runner {label} did not write result.json; see runner logs")
    report = json.loads(report_path.read_text())
    return {
        "id": label, "virtualization": virtualization, "argv": command,
        "cwd": "source-snapshot/" + SPIKE_REL,
        "runner_source_path": "source-snapshot/" + SPIKE_REL + "/run.py",
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
    ignored_overrides = forbidden_environment(os.environ)
    qemu = resolve_qemu(args.qemu or os.environ.get("QEMU_AARCH64", "qemu-system-aarch64"))
    output.parent.mkdir(parents=True, exist_ok=True)
    output.mkdir()

    archive_bytes, source_paths = archive_spike_source(repo, source["commit"])
    archive_path = output / "source" / "spike.tar"
    archive_path.parent.mkdir()
    archive_path.write_bytes(archive_bytes)
    snapshot_root = output / "source-snapshot"
    source_spike = materialize_spike_source(archive_bytes, source_paths, snapshot_root)
    toolchain = toolchain_record(source_spike)
    archive_sha256 = hash_bytes(archive_bytes)

    builds = [
        build_once(output, "a", source_spike, source, archive_sha256, toolchain),
        build_once(output, "b", source_spike, source, archive_sha256, toolchain),
    ]
    build_match = builds[0]["elf_sha256"] == builds[1]["elf_sha256"]
    qemu_sha = hash_file(qemu)
    temp = output / ".qemu-version"
    qemu_version = subprocess.check_output(
        [str(qemu), "--version"], env=runtime_environment(temp, toolchain), text=True
    ).splitlines()[0]
    shutil.rmtree(temp, ignore_errors=True)
    runs = [
        run_one(output, builds[0], qemu, "el1", "off", source_spike, toolchain),
        run_one(output, builds[0], qemu, "el2", "on", source_spike, toolchain),
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

    cargo_toml = (source_spike / "Cargo.toml").read_text()
    lock_text = (source_spike / "Cargo.lock").read_text()
    package_count = len(re.findall(r"(?m)^\[\[package\]\]\s*$", lock_text))
    attestation = {
        "schema": "nagi-spike-attestation/2",
        "mode": "build-and-run",
        "generator": object_identity(repo, args.source_commit, f"{SPIKE_REL}/attest.py"),
        "runner": object_identity(repo, args.source_commit, f"{SPIKE_REL}/run.py"),
        "source": source | {"head_after_build": source_after["head"], "tree_after_build": args.source_tree},
        "source_snapshot": {
            "path": "source-snapshot/" + SPIKE_REL,
            "archive_path": "source/spike.tar",
            "archive_sha256": archive_sha256,
            "file_count": len(source_paths),
            "selection": "git archive of explicit source commit",
            "snapshot_permissions": "read-only during build and run",
        },
        "toolchain": toolchain,
        "environment": {
            "policy": "clean allowlist; inherited variables are not passed to Cargo or runner; external Cargo config files are rejected",
            "discarded_override_names": ignored_overrides,
            "inherited_environment_values_recorded": False,
            "cargo_home_isolated": True,
            "cargo_home_seed": "empty",
            "cargo_offline": True,
            "locked_package_count": package_count,
            "external_dependencies_declared": bool(re.search(
                r"(?m)^\s*\[(?:dev-|build-)?dependencies\]\s*$|^\s*\[target\..*\.(?:dev-|build-)?dependencies\]\s*$",
                cargo_toml,
            )),
        },
        "builds": builds,
        "build_reproducibility": {
            "two_builds_match": build_match,
            "status": "MATCHED" if build_match else "DIFFERENT",
            "interpretation": "same source snapshot and pinned tools, separate empty Cargo homes and target dirs; same-host diagnostic only, not a path-independent or independent rebuild proof",
        },
        "qemu": {"path_realpath": str(qemu), "sha256": qemu_sha, "version": qemu_version},
        "runs": runs,
        "acceptance": {
            "el1": "PASS", "el2_negative": "MET", "source_bound_runs": "PASS",
            "two_build_comparison": "MATCHED" if build_match else "DIFFERENT",
            "phase_c": "PENDING", "independent_rebuild": "NOT_RUN",
            "independent_rebuild_note": "receipt JSON and hashes are not a trusted signature; independent rebuild review is required",
        },
    }
    attestation["files"] = file_manifest(output)
    path = output / "attestation.json"
    path.write_text(json.dumps(attestation, indent=2) + "\n")
    return path

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
    comparison = record["build_reproducibility"]["status"]
    if comparison == "DIFFERENT":
        print("SOURCE-BOUND RUNS RECORDED; two-build comparison=DIFFERENT; no reproducibility or phase-C PASS is claimed")
        return 2
    print("SOURCE-BOUND RUNS: PASS; two-build comparison=MATCHED; full checkpoint remains PENDING_INDEPENDENT_REBUILD")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
