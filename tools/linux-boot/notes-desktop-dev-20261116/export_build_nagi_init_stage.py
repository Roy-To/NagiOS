#!/usr/bin/env python3
"""Export a pinned Nagi source snapshot and build nagi-init in isolation.

The stage is a one-shot, non-Git snapshot. Every mutating command is run with
the snapshot as its explicit working directory. Failed stages are retained for
inspection; callers must choose a fresh label rather than reusing one.
"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
from typing import Mapping, Sequence


SOURCE_COMMIT = "7de0284c2ad781f951b177af8237f9da447d0c40"
COMMIT_ID_RE = re.compile(r"(?:[0-9a-fA-F]{40}|[0-9a-fA-F]{64})\Z")
LANE_RELATIVE = Path("tools/linux-boot/notes-desktop-dev-20261116")
TARGET = "x86_64-unknown-linux-musl"
TOOLCHAIN = "1.98.1-aarch64-apple-darwin"


class StageError(RuntimeError):
    def __init__(self, message: str, exit_code: int = 1) -> None:
        super().__init__(message)
        self.exit_code = exit_code


def validate_label(label: str) -> str:
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,47}", label):
        raise StageError("stage label must be 1-48 ASCII letters, digits, '_' or '-' and start alphanumeric", 2)
    return label


def validate_commit_id(commit: str) -> str:
    """Require a full immutable Git object ID, never a branch or abbreviation."""
    if not COMMIT_ID_RE.fullmatch(commit):
        raise StageError("source commit must be a full 40- or 64-character hexadecimal object ID", 2)
    return commit.lower()


def reject_symlinked_path_components(path: Path, label: str) -> None:
    """Reject lexical symlink components before using a destination path."""
    candidate = path.expanduser()
    if not candidate.is_absolute():
        candidate = Path.cwd() / candidate

    current = Path(candidate.anchor)
    for component in candidate.parts[1:]:
        if component in ("", "."):
            continue
        current = current / component
        try:
            metadata = current.lstat()
        except FileNotFoundError:
            # A missing suffix cannot contain an existing child. The caller
            # decides whether the final path is required to exist.
            break
        except OSError as error:
            raise StageError(f"could not inspect {label} path component: {current}: {error}") from error
        if stat.S_ISLNK(metadata.st_mode):
            raise StageError(f"refusing symlinked {label} path component: {current}")


def stage_destination(parent: Path, label: str) -> Path:
    """Validate and return the absolute fresh-stage path without creating it."""
    validate_label(label)
    reject_symlinked_path_components(parent, "stage parent")
    absolute_parent = Path(os.path.abspath(parent.expanduser()))
    if absolute_parent.exists() or absolute_parent.is_symlink():
        parent_metadata = absolute_parent.lstat()
        if stat.S_ISLNK(parent_metadata.st_mode):
            raise StageError(f"refusing symlinked stage parent: {absolute_parent}")
        if not stat.S_ISDIR(parent_metadata.st_mode):
            raise StageError(f"stage parent is not a real directory: {absolute_parent}")

    destination = absolute_parent / f"source-{label}"
    if destination.exists() or destination.is_symlink():
        raise StageError(f"refusing existing stage; choose a fresh label: {destination}")
    return destination


def create_stage_directory(parent: Path, label: str) -> Path:
    """Create a unique stage without following a destination symlink."""
    destination = stage_destination(parent, label)
    parent = destination.parent
    parent.mkdir(parents=True, exist_ok=True)
    reject_symlinked_path_components(parent, "stage parent")
    if parent.is_symlink() or not parent.is_dir():
        raise StageError(f"stage parent is not a real directory: {parent}")
    try:
        destination.mkdir(mode=0o755)
    except FileExistsError as error:
        raise StageError(f"refusing concurrently created stage: {destination}") from error
    return destination.resolve(strict=True)


def _stage_git_environment(stage_root: Path, base: Mapping[str, str] | None = None) -> dict[str, str]:
    env = dict(os.environ if base is None else base)
    for name in (
        "GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_NAMESPACE",
        "GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS",
    ):
        env.pop(name, None)
    for name in tuple(env):
        if name.startswith("GIT_CONFIG_KEY_") or name.startswith("GIT_CONFIG_VALUE_"):
            env.pop(name, None)
    # Git checks the current directory before applying a ceiling, so use the
    # stage's parent to stop discovery before it reaches the enclosing repo.
    env["GIT_CEILING_DIRECTORIES"] = str(stage_root.resolve().parent)
    return env


def _assert_not_git_checkout(stage_root: Path) -> None:
    if (stage_root / ".git").exists() or (stage_root / ".git").is_symlink():
        raise StageError(f"export unexpectedly contains .git: {stage_root}")
    env = _stage_git_environment(stage_root)
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=stage_root,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if result.returncode == 0:
        raise StageError(f"stage is being discovered as a Git worktree: {result.stdout.strip()}")


def export_commit(repo_root: Path, commit: str, stage_root: Path) -> None:
    """Extract a committed tree into an already-created, empty stage."""
    repo_root = repo_root.resolve(strict=True)
    reject_symlinked_path_components(stage_root, "stage")
    try:
        stage_metadata = stage_root.lstat()
    except OSError as error:
        raise StageError(f"stage must be an existing real directory: {stage_root}: {error}") from error
    if stat.S_ISLNK(stage_metadata.st_mode):
        raise StageError(f"refusing symlinked stage: {stage_root}")
    if not stat.S_ISDIR(stage_metadata.st_mode):
        raise StageError(f"stage must be an existing real directory: {stage_root}")
    stage_root = stage_root.resolve(strict=True)
    if not stage_root.is_dir() or any(stage_root.iterdir()):
        raise StageError(f"stage must be a new empty directory: {stage_root}")
    if (stage_root / ".git").exists() or (stage_root / ".git").is_symlink():
        raise StageError(f"refusing stage containing .git: {stage_root}")

    tar = shutil.which("tar")
    if tar is None:
        raise StageError("tar executable is required; the empty stage is retained")
    try:
        archive = subprocess.Popen(
            ["git", "-C", str(repo_root), "archive", "--format=tar", commit],
            cwd=repo_root,
            env=_stage_git_environment(stage_root),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except OSError as error:
        raise StageError(f"could not start git archive; empty stage retained at {stage_root}: {error}") from error
    assert archive.stdout is not None
    assert archive.stderr is not None
    try:
        extractor = subprocess.Popen(
            [tar, "-xf", "-", "-C", str(stage_root)],
            cwd=stage_root,
            env=_stage_git_environment(stage_root),
            stdin=archive.stdout,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
    except OSError as error:
        archive.kill()
        archive.stdout.close()
        archive.stderr.close()
        archive.wait()
        raise StageError(f"could not start tar; empty stage retained at {stage_root}: {error}") from error
    archive.stdout.close()
    _, tar_stderr = extractor.communicate()
    git_stderr = archive.stderr.read()
    archive.stderr.close()
    git_code = archive.wait()
    if git_code != 0 or extractor.returncode != 0:
        detail = b" ".join(part.strip() for part in (git_stderr, tar_stderr or b"") if part.strip())
        raise StageError(
            f"git archive/extract failed (git={git_code}, tar={extractor.returncode}); "
            f"partial stage retained at {stage_root}: {detail.decode('utf-8', 'replace')}"
        )

    _assert_not_git_checkout(stage_root)
    required = (
        stage_root / "nagi",
        stage_root / "Cargo.lock",
        stage_root / "user/nagi-init/Cargo.toml",
        stage_root / LANE_RELATIVE / "build_nagi_init_elf.sh",
    )
    missing = [
        str(path.relative_to(stage_root))
        for path in required
        if not path.is_file() or path.is_symlink()
    ]
    if missing:
        raise StageError(f"pinned export is incomplete; partial stage retained: {', '.join(missing)}")


def stage_environment(stage_root: Path, cargo_home: Path, target_dir: Path) -> dict[str, str]:
    """Return a Cargo environment whose writable paths belong to this stage."""
    env = _stage_git_environment(stage_root)
    for name in tuple(env):
        if name.startswith("CARGO_") or name.startswith("RUSTC_"):
            env.pop(name, None)
    for name in (
        "RUSTC", "RUSTDOC", "RUSTFLAGS", "RUSTUP_TOOLCHAIN", "CC", "CXX", "AR",
        "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "NAGI_TARGET_CLANG",
    ):
        env.pop(name, None)

    rustup_home = Path(env.get("RUSTUP_HOME", Path.home() / ".rustup"))
    toolchain_bin = rustup_home / "toolchains" / TOOLCHAIN / "bin"
    env["PATH"] = f"{toolchain_bin}{os.pathsep}{env.get('PATH', '')}"
    env["CARGO_HOME"] = str(cargo_home)
    env["CARGO_TARGET_DIR"] = str(target_dir)
    env["CARGO_BUILD_TARGET_DIR"] = str(target_dir)
    env["CARGO_BUILD_BUILD_DIR"] = str(target_dir)
    return env


def run_in_stage(command: Sequence[str], stage_root: Path, env: Mapping[str, str]) -> None:
    command_env = _stage_git_environment(stage_root, env)
    print(f"cwd={stage_root}", flush=True)
    print("command=" + " ".join(command), flush=True)
    try:
        result = subprocess.run(command, cwd=stage_root, env=command_env, check=False)
    except OSError as error:
        raise StageError(f"could not run stage command; stage retained at {stage_root}: {error}") from error
    if result.returncode != 0:
        raise StageError(
            f"stage command exited {result.returncode}; stage retained at {stage_root}: {' '.join(command)}",
            result.returncode if result.returncode > 0 else 1,
        )


def pinned_toolchain() -> tuple[Path, Path]:
    if sys.platform != "darwin" or os.uname().machine != "arm64":
        raise StageError("full stage build requires the pinned macOS arm64 host")
    rustup_home = Path(os.environ.get("RUSTUP_HOME", Path.home() / ".rustup"))
    toolchain_bin = rustup_home / "toolchains" / TOOLCHAIN / "bin"
    cargo = toolchain_bin / "cargo"
    rustc = toolchain_bin / "rustc"
    rustdoc = toolchain_bin / "rustdoc"
    for tool in (cargo, rustc, rustdoc, Path("/usr/bin/cc"), Path("/usr/bin/clang"), Path("/opt/homebrew/bin/ld.lld")):
        if not tool.is_file() or not os.access(tool, os.X_OK):
            raise StageError(f"required pinned build tool missing: {tool}")
    cargo_version = subprocess.run([cargo, "--version"], text=True, capture_output=True, check=False)
    rustc_version = subprocess.run([rustc, "--version"], text=True, capture_output=True, check=False)
    if cargo_version.returncode or not cargo_version.stdout.startswith("cargo 1.98.1 "):
        raise StageError(f"Cargo 1.98.1 is required: {cargo_version.stdout.strip()} {cargo_version.stderr.strip()}")
    if rustc_version.returncode or not rustc_version.stdout.startswith("rustc 1.98.1 "):
        raise StageError(f"rustc 1.98.1 is required: {rustc_version.stdout.strip()} {rustc_version.stderr.strip()}")
    return toolchain_bin, cargo


def verify_source_commit(repo_root: Path, commit: str) -> str:
    commit = validate_commit_id(commit)
    env = _stage_git_environment(repo_root)
    result = subprocess.run(
        ["git", "-C", str(repo_root), "rev-parse", "--verify", f"{commit}^{{commit}}"],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if result.returncode != 0 or result.stdout.strip().lower() != commit:
        raise StageError(f"source commit is unavailable as a full object ID: {commit}: {result.stderr.strip()}")
    root_result = subprocess.run(
        ["git", "-C", str(repo_root), "rev-parse", "--show-toplevel"],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if root_result.returncode != 0 or Path(root_result.stdout.strip()).resolve() != repo_root.resolve():
        raise StageError(f"script source directory is not the expected Git root: {root_result.stderr.strip()}")
    return commit


def plan_pipeline(repo_root: Path, stage_parent: Path, label: str, commit: str) -> Path:
    """Validate a fixed source and destination, printing no mutable artifacts."""
    validate_label(label)
    commit = validate_commit_id(commit)
    reject_symlinked_path_components(stage_parent, "stage parent")
    verify_source_commit(repo_root, commit)
    stage_root = stage_destination(stage_parent, label)
    print(f"source_commit={commit}", flush=True)
    print(f"stage_root={stage_root}", flush=True)
    print("plan_only=true", flush=True)
    return stage_root


def execute_pipeline(repo_root: Path, stage_parent: Path, label: str, commit: str | None = None) -> Path:
    validate_label(label)
    selected_commit = validate_commit_id(SOURCE_COMMIT if commit is None else commit)
    reject_symlinked_path_components(stage_parent, "stage parent")
    selected_commit = verify_source_commit(repo_root, selected_commit)
    _, cargo = pinned_toolchain()
    stage_root = create_stage_directory(stage_parent, label)
    print(f"source_commit={selected_commit}", flush=True)
    print(f"stage_root={stage_root}", flush=True)

    try:
        export_commit(repo_root, selected_commit, stage_root)
        lane_dir = stage_root / LANE_RELATIVE
        cargo_home = lane_dir / "cargo-home-rust198"
        fetch_target = lane_dir / f"target-source-stage-fetch-{label}"
        build_target = lane_dir / f"target-nagi-init-rust198-source-stage-{label}"
        for path in (cargo_home, fetch_target, build_target):
            if path.exists() or path.is_symlink():
                raise StageError(f"refusing pre-existing stage output path: {path}")

        env = stage_environment(stage_root, cargo_home, fetch_target)
        run_in_stage(["./nagi", "fetch"], stage_root, env)
        run_in_stage(
            [str(cargo), "fetch", "--locked", "--manifest-path", str(stage_root / "Cargo.toml"), "--target", TARGET],
            stage_root,
            env,
        )
        run_in_stage(
            [str(cargo), "fetch", "--locked", "--manifest-path", str(stage_root / "Cargo.toml")],
            stage_root,
            env,
        )
        build_env = stage_environment(stage_root, cargo_home, build_target)
        run_in_stage(
            ["./tools/linux-boot/notes-desktop-dev-20261116/build_nagi_init_elf.sh", f"source-stage-{label}"],
            stage_root,
            build_env,
        )
    except StageError:
        print(f"stage_retained=true path={stage_root}; choose a fresh label to retry", file=sys.stderr)
        raise

    output = (
        stage_root / LANE_RELATIVE / f"target-nagi-init-rust198-source-stage-{label}" /
        TARGET / "debug" / "nagi-init"
    )
    if not output.is_file() or output.is_symlink():
        raise StageError(f"build helper returned success without a regular init ELF: {output}")
    print(f"nagi_init_stage_build=PASS output={output}", flush=True)
    return output


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Export one fixed Git commit into a new non-Git stage, fetch pinned dependencies, "
            "and build a static Linux nagi-init ELF."
        )
    )
    parser.add_argument("label", help="fresh unique stage label (letters, digits, '_' and '-')")
    parser.add_argument(
        "--commit",
        metavar="FULL_SHA",
        help=f"full 40- or 64-character commit ID (default: {SOURCE_COMMIT})",
    )
    parser.add_argument("--plan", action="store_true", help="validate and print the selected commit and fresh stage path without creating it")
    args = parser.parse_args(argv)
    try:
        label = validate_label(args.label)
        commit = validate_commit_id(SOURCE_COMMIT if args.commit is None else args.commit)
        script_path = Path(__file__).resolve(strict=True)
        lane_dir = script_path.parent
        repo_root = script_path.parents[3]
        stage_parent = lane_dir / "source-stages"
        if args.plan:
            plan_pipeline(repo_root, stage_parent, label, commit)
            return 0
        output = execute_pipeline(repo_root, stage_parent, label, commit)
    except StageError as error:
        print(f"stage_error: {error}", file=sys.stderr)
        return error.exit_code
    except (OSError, subprocess.SubprocessError) as error:
        print(f"stage_error: {error}", file=sys.stderr)
        return 1
    print(f"output={output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
