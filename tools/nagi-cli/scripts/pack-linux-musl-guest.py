#!/usr/bin/env python3
"""Pack the prebuilt Linux CLI and its source fixtures for a Nagi guest run."""

from __future__ import annotations

import datetime as dt
import filecmp
import gzip
import hashlib
import json
import os
import shutil
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path, PurePosixPath
from typing import Iterable


TARGET = "x86_64-unknown-linux-musl"
SCRIPT_PATH = Path(__file__).resolve()
REPO_ROOT = SCRIPT_PATH.parents[3]
BUILD_MANIFEST_NAME = "nagi-cli-linux-musl-build.json"
BUILD_SCRIPT_PATH = "tools/nagi-cli/scripts/build-linux-musl.sh"
BUILD_SUPPORT_PATHS = (
    BUILD_SCRIPT_PATH,
    "tools/nagi-cli/scripts/zig-cc-linux-musl.sh",
    "tools/nagi-cli/scripts/verify-cargo-lock.py",
    "tools/nagi-cli/scripts/pack-linux-musl-guest.py",
)
BUILD_ARTIFACT_LABELS = (
    "release_cli",
    "debug_cli",
    "nagi_cli",
    "main_test",
    "cli",
    "run_receipt_cli",
)
LIBC_SOURCE = Path(
    "registry/src/index.crates.io-1949cf8c6b5b557f/libc-0.2.189"
)
PINNED_SUPPLEMENTAL_FILES = {
    "third_party/mozjs-sys-nagi/build.rs": (
        "0403b1d899aee80181015851c964286898a44a3e7b1ad6cf3ca2366680c35222"
    ),
}
SYNTHETIC_WORKSTREAM_STATE_PATH = ".dev/workstreams/development-foundation/state.json"
SYNTHETIC_INTEGRATION_STATE_PATH = ".dev/workstreams/integration-next-phase/state.json"

# Keep the guest input to tracked files consumed by nagi-cli and its tests.
# The source checkout's .git, .dev state, untracked files, and credentials are
# never copied. A small synthetic .dev registry and Git repository are added
# below because the CLI tests exercise branch and repository discovery.
SOURCE_PATHS = (
    ".gitignore",
    "Cargo.lock",
    "Cargo.toml",
    "nagi",
    "crates/nagi-abi/src/lib.rs",
    "README.md",
    "THIRD_PARTY_NOTICES.md",
    ".github/workflows/ci.yml",
    "nagi.toml",
    "rust-toolchain.toml",
    "assets/nagi/nagi_logo_formal.svg",
    "docs/Nagi_OS_0.1_Codex_Implementation_Spec.md",
    "docs/implementation_status.md",
    "docs/testing/diagnostic-report.schema.json",
    "kernel/src/syscall.rs",
    "kernel/src/user_process.rs",
    "loader/Cargo.toml",
    "samples/hello-nagi/Cargo.toml",
    "tests/acceptance",
    "tests/fixtures",
    "tools/linux-boot/make_dev_bundle.py",
    "tools/linux-boot/run_dev_image.py",
    ":(glob)third_party/cc-nagi-patches/*.patch",
    "third_party/fonts.lock",
    ":(glob)third_party/freetype-sys-patches/*.patch",
    ":(glob)third_party/hyper-util-servo-patches/*.patch",
    ":(glob)third_party/inventory-nagi-patches/*.patch",
    ":(glob)third_party/libc-servo-patches/*.patch",
    ":(glob)third_party/llama-cpp-patches/*.patch",
    ":(glob)third_party/mesa-patches/*.patch",
    ":(glob)third_party/mio-servo-patches/*.patch",
    "third_party/models.lock",
    ":(glob)third_party/mozjs-sys-nagi-patches/*.patch",
    ":(glob)third_party/relibc-patches/*.patch",
    "third_party/relibc/src/header/pthread/mod.rs",
    "third_party/relibc/src/header/stdlib/cbindgen.toml",
    "third_party/relibc/src/nagi.rs",
    ":(glob)third_party/rust-std/patches/*.patch",
    ":(glob)third_party/servo-patches/*.patch",
    ":(glob)third_party/socket2-servo-patches/*.patch",
    "third_party/sources.lock",
    ":(glob)third_party/surfman-patches/*.patch",
    ":(glob)third_party/tempfile-nagi-patches/*.patch",
    ":(glob)third_party/tokio-servo-patches/*.patch",
    ":(glob)third_party/whisper-cpp-patches/*.patch",
    "tools/llama/build-nagi-target.sh",
    "tools/mesa/build.sh",
    "tools/mesa/nagi-cxx-runtime.cpp",
    "tools/mesa/nagi-headers/sys/mman.h",
    "tools/mesa/nagi-libcpp-abi.cpp",
    "tools/mesa/nagi-libcpp-sort.cpp",
    "tools/nagi-cli/Cargo.toml",
    "tools/nagi-cli/scripts/install-alpine-test-deps.sh",
    "tools/nagi-cli/src",
    "tools/nagi-cli/tests",
    "tools/nagi-pkg/Cargo.toml",
    "tools/nagi-target-cc.sh",
    "tools/whisper/build-nagi-target.sh",
    "user/libnagi/src/lib.rs",
    "user/libnagi/src/storage.rs",
    "user/nagi-albert/src/lib.rs",
    "user/nagi-albert/src/m18_acceptance.rs",
    "user/nagi-init/build.rs",
    "user/nagi-init/linker.ld",
    "user/nagi-init/src/main.rs",
    "user/nagi-model-manager/tests/fixtures",
    "user/nagi-net/src/smoltcp_stack.rs",
    "user/nagi-posix/src/abi.rs",
    "user/nagi-posix/src/runtime.rs",
)

REQUIRED_SOURCE_FILES = (
    ".gitignore",
    "nagi",
    "Cargo.toml",
    "Cargo.lock",
    "nagi.toml",
    "rust-toolchain.toml",
    "crates/nagi-abi/src/lib.rs",
    "tools/nagi-cli/Cargo.toml",
    "tools/nagi-cli/scripts/install-alpine-test-deps.sh",
    "tools/nagi-cli/src/main.rs",
    "tools/nagi-cli/tests/cli.rs",
    "tools/nagi-cli/tests/run_receipt_cli.rs",
    ".github/workflows/ci.yml",
    "tests/acceptance/registry.tsv",
    "tests/fixtures/m20_model_store_reader.rs",
    "third_party/sources.lock",
    "third_party/models.lock",
    "third_party/fonts.lock",
)

HARNESS_PREFIXES = {
    "nagi_cli": "nagi_cli-",
    "main_test": "nagi-",
    "cli": "cli-",
    "run_receipt_cli": "run_receipt_cli-",
}

# Cargo's target-specific fingerprint marker hashes match the executable
# suffix in debug/deps, including when a normal bin has the same prefix.
HARNESS_FINGERPRINTS = {
    "nagi_cli": "test-lib-nagi_cli",
    "main_test": "test-bin-nagi",
    "cli": "test-integration-test-cli",
    "run_receipt_cli": "test-integration-test-run_receipt_cli",
}
HARNESS_SOURCES = {
    "nagi_cli": "tools/nagi-cli/src/lib.rs",
    "main_test": "tools/nagi-cli/src/main.rs",
    "cli": "tools/nagi-cli/tests/cli.rs",
    "run_receipt_cli": "tools/nagi-cli/tests/run_receipt_cli.rs",
}

WORKSTREAM_REGISTRY = {
    "schema_ref": "schemas/workstreams.schema.json",
    "schema_version": 1,
    "workstreams": [
        {
            "id": "cli-guest-fixture",
            "owner": "CLI guest fixture",
            "owner_branch": "codex/cli-fixture-sentinel",
            "recommended_worktree": "sparse CLI guest fixture",
            "state_file": ".dev/workstreams/cli-guest-fixture/state.json",
            "dependencies": [],
            "allowed_paths": ["tools/nagi-cli"],
            "forbidden_paths": [],
            "activation_gate": "test-only fixture",
            "merge_boundary": "test-only fixture",
        },
        {
            "id": "development-foundation",
            "owner": "Nagi CLI guest fixture",
            "owner_branch": "codex/cli-guest-foundation",
            "recommended_worktree": "synthetic CLI guest fixture",
            "state_file": SYNTHETIC_WORKSTREAM_STATE_PATH,
            "dependencies": [],
            "allowed_paths": ["tools/nagi-cli"],
            "forbidden_paths": [],
            "activation_gate": "test-only synthetic state; no product milestone is asserted",
            "merge_boundary": "test-only synthetic state",
        },
        {
            "id": "integration-next-phase",
            "owner": "CLI synthetic integration fixture",
            "owner_branch": "codex/integration-next-phase",
            "recommended_worktree": "synthetic CLI integration fixture",
            "state_file": SYNTHETIC_INTEGRATION_STATE_PATH,
            "dependencies": [],
            "allowed_paths": ["tools/nagi-cli"],
            "forbidden_paths": [],
            "activation_gate": "test-only synthetic integration fixture",
            "merge_boundary": "test-only synthetic integration fixture",
        }
    ],
}

SYNTHETIC_WORKSTREAM_STATES = {
    SYNTHETIC_WORKSTREAM_STATE_PATH: {
        "schema_ref": "../../schemas/workstream-state.schema.json",
        "schema_version": 1,
        "workstream_id": "development-foundation",
        "current_milestone": "CLI-GUEST-FIXTURE",
        "current_checkpoint": "CLI-GUEST-FIXTURE-STATE",
        "status": "NOT_STARTED",
        "last_verified": {
            "commit_sha": "0000000000000000000000000000000000000000",
            "verified_at": "2000-01-01T00:00:00Z",
            "normal_state": "No product workstream state is included in this fixture.",
            "commands": [],
            "tests": [],
            "ci_runs": [],
        },
        "release_line_gate": {
            "source": "docs/implementation_status.md",
            "last_checked_at": "2000-01-01T00:00:00Z",
            "activation_requirement": "Synthetic CLI guest fixture only.",
        },
        "blocker": {
            "active": False,
            "class": None,
            "summary": None,
            "reproduction_commands": [],
            "failure_log_summary": None,
            "attempted_fixes": [],
            "do_not_try": [],
        },
        "next_action": "No action; this is a test-only fixture.",
        "acceptance_criteria": [
            "Allow CLI development-state validation tests to execute in the sparse guest fixture."
        ],
        "deferred": [],
        "cross_workstream_dependencies": [],
    }
}
SYNTHETIC_WORKSTREAM_STATES[SYNTHETIC_INTEGRATION_STATE_PATH] = {
    **SYNTHETIC_WORKSTREAM_STATES[SYNTHETIC_WORKSTREAM_STATE_PATH],
    "workstream_id": "integration-next-phase",
}


class PackError(RuntimeError):
    """An input or output failed a fail-closed packaging check."""


def _within(path: Path, parent: Path) -> bool:
    try:
        path.relative_to(parent)
        return True
    except ValueError:
        return False


def _assert_no_symlink_components(path: Path, boundary: Path, label: str) -> None:
    path = Path(os.path.abspath(path))
    boundary = Path(os.path.abspath(boundary))
    if not _within(path, boundary):
        raise PackError(f"{label}: path escapes {boundary}")
    cursor = boundary
    if cursor.is_symlink():
        raise PackError(f"{label}: refusing symlink path {cursor}")
    for component in path.relative_to(boundary).parts:
        cursor = cursor / component
        if cursor.is_symlink():
            raise PackError(f"{label}: refusing symlink path {cursor}")


def _git_env(overrides: dict[str, str] | None = None) -> dict[str, str]:
    """Drop inherited Git redirection/configuration before each Git command."""
    process_env = {
        key: value for key, value in os.environ.items() if not key.upper().startswith("GIT_")
    }
    process_env.update(
        {
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_TERMINAL_PROMPT": "0",
        }
    )
    if overrides:
        process_env.update(overrides)
    return process_env


def _target_path(value: str | Path, repo_root: Path, label: str) -> Path:
    path = Path(value)
    if not path.is_absolute():
        path = repo_root / path
    path = Path(os.path.abspath(path))
    target_root = repo_root / "target"
    if target_root.is_symlink() or not target_root.is_dir():
        raise PackError(f"{label}: repository target directory is missing or redirected")
    if not _within(path, target_root):
        raise PackError(f"{label}: must stay under {target_root}")
    cursor = target_root
    for part in path.relative_to(target_root).parts:
        cursor = cursor / part
        if cursor.is_symlink():
            raise PackError(f"{label}: refusing symlink path {cursor}")
        if cursor.exists() and not cursor.is_dir():
            raise PackError(f"{label}: expected directory at {cursor}")
    return path


def _run_git(repo_root: Path, args: list[str], *, env: dict[str, str] | None = None) -> bytes:
    process_env = _git_env(env)
    result = subprocess.run(
        ["git", *args],
        cwd=repo_root,
        env=process_env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode:
        detail = result.stderr.decode("utf-8", errors="replace").strip()
        raise PackError(f"git {' '.join(args)} failed: {detail or result.returncode}")
    return result.stdout


def _source_paths(repo_root: Path) -> list[str]:
    changed = subprocess.run(
        ["git", "diff", "--quiet", "HEAD", "--", *SOURCE_PATHS],
        cwd=repo_root,
        env=_git_env(),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        check=False,
    )
    if changed.returncode:
        raise PackError("allowlisted guest source differs from HEAD; commit or restore it before packing")
    untracked = _run_git(
        repo_root,
        ["ls-files", "--others", "--exclude-standard", "-z", "--", *SOURCE_PATHS],
    )
    if untracked:
        names = ", ".join(untracked.decode("utf-8", errors="replace").split("\0")[:4])
        raise PackError(f"untracked allowlisted source is not packable: {names}")
    tracked = _run_git(repo_root, ["ls-files", "-z", "--", *SOURCE_PATHS])
    paths = sorted(item for item in tracked.decode("utf-8").split("\0") if item)
    path_set = set(paths)
    missing = [path for path in REQUIRED_SOURCE_FILES if path not in path_set]
    if missing:
        raise PackError(f"required tracked source is missing: {', '.join(missing)}")
    if not paths:
        raise PackError("Git returned no allowlisted guest inputs")
    return paths


def _copy_regular_file(source_root: Path, stage_root: Path, relative: str) -> None:
    relative_path = Path(relative)
    if relative_path.is_absolute() or ".." in relative_path.parts:
        raise PackError(f"unsafe source path: {relative}")
    source = source_root / relative_path
    try:
        metadata = source.lstat()
    except OSError as error:
        raise PackError(f"cannot inspect source {relative}: {error}") from error
    if stat.S_ISLNK(metadata.st_mode):
        raise PackError(f"refusing symlink source: {relative}")
    if not stat.S_ISREG(metadata.st_mode):
        raise PackError(f"guest source is not a regular file: {relative}")
    resolved = source.resolve(strict=True)
    if not _within(resolved, source_root):
        raise PackError(f"source resolves outside the checkout: {relative}")
    destination = stage_root / relative_path
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists() or destination.is_symlink():
        raise PackError(f"refusing to overwrite staged source: {relative}")
    shutil.copy2(source, destination, follow_symlinks=False)


def _sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _fingerprint_source_inputs(repo_root: Path, source_paths: list[str]) -> dict[str, str]:
    paths = sorted(set(source_paths) | set(PINNED_SUPPLEMENTAL_FILES))
    fingerprints: dict[str, str] = {}
    for relative in paths:
        source = repo_root / relative
        _assert_no_symlink_components(source, repo_root, f"build source {relative}")
        try:
            metadata = source.lstat()
        except OSError as error:
            raise PackError(f"cannot inspect build source {relative}: {error}") from error
        if not stat.S_ISREG(metadata.st_mode):
            raise PackError(f"build source is not a regular file: {relative}")
        digest = _sha256_file(source)
        expected = PINNED_SUPPLEMENTAL_FILES.get(relative)
        if expected is not None and digest != expected:
            raise PackError(f"pinned build source hash mismatch: {relative}")
        fingerprints[relative] = digest
    return fingerprints


def _reachable_local_cargo_packages(
    repo_root: Path, metadata: dict[str, object]
) -> dict[str, str]:
    """Return local packages in nagi-cli's resolved Cargo dependency graph."""
    repo_root = repo_root.resolve(strict=True)
    packages = metadata.get("packages")
    resolve = metadata.get("resolve")
    if not isinstance(packages, list) or not isinstance(resolve, dict):
        raise PackError("Cargo metadata has no resolved package graph")
    raw_nodes = resolve.get("nodes")
    if not isinstance(raw_nodes, list):
        raise PackError("Cargo metadata has no resolved dependency nodes")

    packages_by_id = {
        package["id"]: package
        for package in packages
        if isinstance(package, dict) and isinstance(package.get("id"), str)
    }
    nodes_by_id = {
        node["id"]: node
        for node in raw_nodes
        if isinstance(node, dict) and isinstance(node.get("id"), str)
    }
    cli_manifest = (repo_root / "tools/nagi-cli/Cargo.toml").resolve(strict=True)
    cli_ids = [
        package_id
        for package_id, package in packages_by_id.items()
        if package.get("name") == "nagi-cli"
        and Path(str(package.get("manifest_path", ""))).resolve(strict=True)
        == cli_manifest
    ]
    if len(cli_ids) != 1:
        raise PackError("Cargo metadata must resolve exactly one mainline nagi-cli package")

    reachable: set[str] = set()
    pending = [cli_ids[0]]
    while pending:
        package_id = pending.pop()
        if package_id in reachable:
            continue
        node = nodes_by_id.get(package_id)
        if node is None:
            raise PackError(f"Cargo metadata is missing dependency node {package_id}")
        reachable.add(package_id)
        dependencies = node.get("deps", [])
        if not isinstance(dependencies, list):
            raise PackError(f"Cargo metadata has invalid dependencies for {package_id}")
        for dependency in dependencies:
            if not isinstance(dependency, dict) or not isinstance(dependency.get("pkg"), str):
                raise PackError(f"Cargo metadata has an invalid dependency edge for {package_id}")
            pending.append(dependency["pkg"])

    local_packages: dict[str, str] = {}
    for package_id in sorted(reachable - {cli_ids[0]}):
        package = packages_by_id.get(package_id)
        if package is None:
            raise PackError(f"Cargo metadata is missing package {package_id}")
        if package.get("source") is not None:
            continue
        manifest = Path(str(package.get("manifest_path", ""))).resolve(strict=True)
        try:
            relative_manifest = manifest.relative_to(repo_root).as_posix()
        except ValueError as error:
            raise PackError(f"local Cargo dependency escapes the checkout: {manifest}") from error
        if not manifest.is_file() or manifest.name != "Cargo.toml":
            raise PackError(f"local Cargo dependency manifest is invalid: {manifest}")
        name = package.get("name")
        if not isinstance(name, str) or name in local_packages:
            raise PackError(f"Cargo metadata has an ambiguous local package name: {name}")
        local_packages[name] = relative_manifest
    return dict(sorted(local_packages.items()))


def _tracked_cargo_dependency_sources(
    repo_root: Path, local_packages: dict[str, str]
) -> list[str]:
    """List clean, tracked files for local Cargo dependencies without packaging them."""
    repo_root = repo_root.resolve(strict=True)
    package_roots = sorted(
        {PurePosixPath(manifest).parent.as_posix() for manifest in local_packages.values()}
    )
    if not package_roots:
        return []
    changed = subprocess.run(
        ["git", "diff", "--quiet", "HEAD", "--", *package_roots],
        cwd=repo_root,
        env=_git_env(),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        check=False,
    )
    if changed.returncode:
        raise PackError("local Cargo dependency source differs from HEAD")
    untracked = _run_git(
        repo_root,
        ["ls-files", "--others", "--exclude-standard", "-z", "--", *package_roots],
    )
    if untracked:
        names = ", ".join(untracked.decode("utf-8", errors="replace").split("\0")[:4])
        raise PackError(f"untracked local Cargo dependency source is not packable: {names}")
    tracked = _run_git(repo_root, ["ls-files", "-z", "--", *package_roots])
    paths = sorted(item for item in tracked.decode("utf-8").split("\0") if item)
    path_set = set(paths)
    for name, manifest in local_packages.items():
        if manifest not in path_set:
            raise PackError(f"local Cargo dependency manifest is not tracked: {manifest}")
    if not paths:
        raise PackError("Cargo metadata found local dependencies but Git listed no source files")
    return paths


def _cargo_local_dependency_inputs(repo_root: Path) -> tuple[dict[str, str], list[str]]:
    cargo_bin = os.environ.get("CARGO", "cargo")
    command = [
        cargo_bin,
        "metadata",
        "--locked",
        "--offline",
        "--filter-platform",
        TARGET,
        "--format-version",
        "1",
        "--manifest-path",
        str(repo_root / "Cargo.toml"),
    ]
    result = subprocess.run(
        command,
        cwd=repo_root,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode:
        detail = result.stderr.decode("utf-8", errors="replace").strip()
        raise PackError(
            f"locked offline Cargo dependency inspection failed: {detail or result.returncode}"
        )
    try:
        metadata = json.loads(result.stdout)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise PackError(f"Cargo metadata returned invalid JSON: {error}") from error
    local_packages = _reachable_local_cargo_packages(repo_root, metadata)
    source_paths = _tracked_cargo_dependency_sources(repo_root, local_packages)
    return local_packages, source_paths


def _fingerprint_build_support(repo_root: Path) -> dict[str, str]:
    fingerprints: dict[str, str] = {}
    for relative in BUILD_SUPPORT_PATHS:
        source = repo_root / relative
        _assert_no_symlink_components(source, repo_root, f"build support {relative}")
        try:
            metadata = source.lstat()
        except OSError as error:
            raise PackError(f"cannot inspect build support {relative}: {error}") from error
        if not stat.S_ISREG(metadata.st_mode):
            raise PackError(f"build support is not a regular file: {relative}")
        fingerprints[relative] = _sha256_file(source)
    return fingerprints


def _copy_pinned_supplemental_files(source_root: Path, stage_root: Path) -> list[str]:
    copied: list[str] = []
    for relative, expected_sha256 in sorted(PINNED_SUPPLEMENTAL_FILES.items()):
        source = source_root / relative
        _assert_no_symlink_components(
            source, source_root, f"supplemental guest source {relative}"
        )
        try:
            metadata = source.lstat()
        except OSError as error:
            raise PackError(
                f"cannot inspect supplemental guest source {relative}: {error}"
            ) from error
        if not stat.S_ISREG(metadata.st_mode):
            raise PackError(f"supplemental guest source is not a regular file: {relative}")
        digest = hashlib.sha256(source.read_bytes()).hexdigest()
        if digest != expected_sha256:
            raise PackError(f"supplemental guest source hash does not match pinned input: {relative}")
        _copy_regular_file(source_root, stage_root, relative)
        staged = stage_root / relative
        if hashlib.sha256(staged.read_bytes()).hexdigest() != expected_sha256:
            raise PackError(f"staged supplemental guest source changed: {relative}")
        copied.append(relative)
    return copied


def _write_synthetic_workstream_files(rootfs: Path) -> list[str]:
    registry = rootfs / ".dev" / "workstreams.json"
    registry.parent.mkdir(parents=True, exist_ok=True)
    registry.write_text(
        json.dumps(WORKSTREAM_REGISTRY, indent=2) + "\n", encoding="utf-8"
    )
    written = [".dev/workstreams.json"]
    for relative, state in SYNTHETIC_WORKSTREAM_STATES.items():
        path = rootfs / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8")
        written.append(relative)
    return written


def _verify_synthetic_workstream_fixture(rootfs: Path) -> None:
    registry_path = rootfs / ".dev/workstreams.json"
    state_path = rootfs / SYNTHETIC_WORKSTREAM_STATE_PATH
    try:
        registry = json.loads(registry_path.read_text(encoding="utf-8"))
        state = json.loads(state_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
            raise PackError(
                f"guest archive is missing a valid synthetic workstream fixture: {error}"
            ) from error
    if not isinstance(registry, dict) or not isinstance(state, dict):
        raise PackError("guest archive synthetic workstream fixture must contain JSON objects")
    streams = registry.get("workstreams")
    if not isinstance(streams, list):
        raise PackError("guest archive synthetic workstream registry has no workstreams list")
    foundation = next(
        (
            item
            for item in streams
            if isinstance(item, dict) and item.get("id") == "development-foundation"
        ),
        None,
    )
    if (
        foundation is None
        or foundation.get("state_file") != SYNTHETIC_WORKSTREAM_STATE_PATH
    ):
        raise PackError(
            "guest archive is missing the synthetic development-foundation registry entry"
        )
    if state.get("workstream_id") != "development-foundation":
        raise PackError("guest archive synthetic foundation state has the wrong workstream id")
    integration = next(
        (
            item
            for item in streams
            if isinstance(item, dict) and item.get("id") == "integration-next-phase"
        ),
        None,
    )
    if (
        integration is None
        or integration.get("state_file") != SYNTHETIC_INTEGRATION_STATE_PATH
        or integration.get("owner_branch") != "codex/integration-next-phase"
    ):
        raise PackError("guest archive is missing the synthetic integration workstream entry")
    integration_state_path = rootfs / SYNTHETIC_INTEGRATION_STATE_PATH
    try:
        integration_state = json.loads(integration_state_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise PackError(f"guest archive is missing a valid synthetic integration state: {error}") from error
    if integration_state.get("workstream_id") != "integration-next-phase":
        raise PackError("guest archive synthetic integration state has the wrong workstream id")


def _verify_fixed_guest_inputs(rootfs: Path) -> None:
    required = (
        "nagi",
        "tools/nagi-cli/scripts/install-alpine-test-deps.sh",
        ".github/workflows/ci.yml",
        "crates/nagi-abi/src/lib.rs",
        SYNTHETIC_WORKSTREAM_STATE_PATH,
        SYNTHETIC_INTEGRATION_STATE_PATH,
        *PINNED_SUPPLEMENTAL_FILES.keys(),
    )
    for relative in required:
        path = rootfs / relative
        if path.is_symlink() or not path.is_file():
            raise PackError(f"guest archive is missing required fixed input: {relative}")
    _verify_synthetic_workstream_fixture(rootfs)
    for relative, expected_sha256 in PINNED_SUPPLEMENTAL_FILES.items():
        digest = hashlib.sha256((rootfs / relative).read_bytes()).hexdigest()
        if digest != expected_sha256:
            raise PackError(f"guest archive supplemental input hash mismatch: {relative}")


def _copy_regular_tree(source: Path, destination: Path) -> None:
    if source.is_symlink() or not source.is_dir():
        raise PackError(f"required cache directory is missing or redirected: {source}")
    for current, directories, files in os.walk(source, topdown=True, followlinks=False):
        current_path = Path(current)
        directories.sort()
        files.sort()
        for name in directories:
            path = current_path / name
            if path.is_symlink():
                raise PackError(f"refusing symlink in Cargo cache: {path}")
        relative = current_path.relative_to(source)
        destination_dir = destination / relative
        destination_dir.mkdir(parents=True, exist_ok=True)
        for name in files:
            source_file = current_path / name
            metadata = source_file.lstat()
            if not stat.S_ISREG(metadata.st_mode):
                raise PackError(f"refusing non-regular Cargo cache input: {source_file}")
            target_file = destination_dir / name
            shutil.copy2(source_file, target_file, follow_symlinks=False)


def validate_static_x86_64_elf(path: Path) -> None:
    if path.is_symlink() or not path.is_file():
        raise PackError(f"ELF artifact is missing or redirected: {path}")
    if not os.access(path, os.X_OK):
        raise PackError(f"ELF artifact is not executable: {path}")
    data = path.read_bytes()
    if len(data) < 64 or data[:4] != b"\x7fELF":
        raise PackError(f"not an ELF executable: {path}")
    if data[4] != 2 or data[5] != 1 or data[6] != 1:
        raise PackError(f"expected little-endian ELF64: {path}")
    (
        elf_type,
        machine,
        version,
        _entry,
        phoff,
        _shoff,
        _flags,
        ehsize,
        phentsize,
        phnum,
        _shentsize,
        _shnum,
        _shstrndx,
    ) = struct.unpack_from("<HHIQQQIHHHHHH", data, 16)
    if elf_type not in (2, 3) or machine != 62 or version != 1 or ehsize != 64:
        raise PackError(f"expected an x86-64 Linux executable ELF: {path}")
    if phnum == 0 or phentsize < 56 or phoff + phentsize * phnum > len(data):
        raise PackError(f"invalid ELF program header table: {path}")

    has_load_segment = False
    for index in range(phnum):
        offset = phoff + index * phentsize
        p_type, _p_flags, p_offset, _vaddr, _paddr, p_filesz, _memsz, _align = (
            struct.unpack_from("<IIQQQQQQ", data, offset)
        )
        if p_offset + p_filesz > len(data):
            raise PackError(f"ELF segment extends beyond the file: {path}")
        if p_type == 1:  # PT_LOAD
            has_load_segment = True
        elif p_type == 3:  # PT_INTERP
            raise PackError(f"ELF requires an external interpreter: {path}")
        elif p_type == 2:  # PT_DYNAMIC
            if p_filesz % 16:
                raise PackError(f"invalid ELF dynamic table: {path}")
            terminated = False
            for position in range(p_offset, p_offset + p_filesz, 16):
                tag, _value = struct.unpack_from("<qQ", data, position)
                if tag == 1:  # DT_NEEDED
                    raise PackError(f"ELF requires an external shared library: {path}")
                if tag == 0:  # DT_NULL
                    terminated = True
                    break
            if not terminated:
                raise PackError(f"unterminated ELF dynamic table: {path}")
    if not has_load_segment:
        raise PackError(f"ELF has no loadable segment: {path}")


def _validate_build_id(path: Path, expected_build_id: str, label: str) -> None:
    try:
        marker = bytes.fromhex(expected_build_id)
    except ValueError as error:
        raise PackError(f"invalid build fingerprint for {label}") from error
    if len(marker) != 20:
        raise PackError(f"invalid build fingerprint for {label}")

    data = path.read_bytes()
    if len(data) < 64 or data[:7] != b"\x7fELF\x02\x01\x01":
        raise PackError(f"{label} is not a little-endian ELF64 executable: {path}")
    (
        _elf_type,
        _machine,
        _version,
        _entry,
        phoff,
        _shoff,
        _flags,
        ehsize,
        phentsize,
        phnum,
        _shentsize,
        _shnum,
        _shstrndx,
    ) = struct.unpack_from("<HHIQQQIHHHHHH", data, 16)
    if ehsize != 64 or phentsize < 56 or phnum == 0:
        raise PackError(f"invalid ELF program header table for {label}: {path}")
    if phoff > len(data) or phentsize * phnum > len(data) - phoff:
        raise PackError(f"invalid ELF program header table for {label}: {path}")

    build_ids: list[bytes] = []
    for index in range(phnum):
        header_offset = phoff + index * phentsize
        p_type, _p_flags, p_offset, _vaddr, _paddr, p_filesz, _memsz, _align = (
            struct.unpack_from("<IIQQQQQQ", data, header_offset)
        )
        if p_offset > len(data) or p_filesz > len(data) - p_offset:
            raise PackError(f"ELF segment extends beyond the file for {label}: {path}")
        if p_type != 4:  # PT_NOTE
            continue

        note_data = data[p_offset : p_offset + p_filesz]
        position = 0
        while position < len(note_data):
            remaining = len(note_data) - position
            if not any(note_data[position:]):
                break  # ELF permits zero padding at the end of a note segment.
            if remaining < 12:
                raise PackError(f"truncated ELF note table for {label}: {path}")
            name_size, desc_size, note_type = struct.unpack_from("<III", note_data, position)
            name_start = position + 12
            desc_start = name_start + ((name_size + 3) & ~3)
            note_end = desc_start + ((desc_size + 3) & ~3)
            if note_end > len(note_data):
                raise PackError(f"truncated ELF note entry for {label}: {path}")
            name = note_data[name_start : name_start + name_size]
            descriptor = note_data[desc_start : desc_start + desc_size]
            if name_size == 4 and name == b"GNU\0" and note_type == 3:
                build_ids.append(descriptor)
            position = note_end

    if len(build_ids) != 1 or build_ids[0] != marker:
        raise PackError(
            f"{label} does not carry the current build-script ID {expected_build_id}: {path}"
        )


def _makefile_prerequisites(contents: bytes) -> set[bytes]:
    """Read escaped prerequisite tokens from rustc's Makefile-style dep-info."""
    lines: list[bytes] = []
    current = bytearray()
    index = 0
    while index < len(contents):
        value = contents[index]
        if value == 92 and contents[index + 1 : index + 2] == b"\n":
            current.append(32)
            index += 2
        elif value == 92 and contents[index + 1 : index + 3] == b"\r\n":
            current.append(32)
            index += 3
        elif value == 10 or contents[index : index + 2] == b"\r\n":
            lines.append(bytes(current))
            current.clear()
            index += 2 if contents[index : index + 2] == b"\r\n" else 1
        else:
            current.append(value)
            index += 1
    if current:
        lines.append(bytes(current))

    prerequisites: set[bytes] = set()
    for line in lines:
        delimiter = -1
        index = 0
        while index < len(line):
            value = line[index]
            if value == 92 and index + 1 < len(line):
                index += 2
            elif value == 35:  # Unescaped Makefile comment.
                break
            elif value == 58:  # Rule delimiter.
                delimiter = index
                break
            else:
                index += 1
        if delimiter < 0:
            continue

        token = bytearray()
        index = delimiter + 1
        while index < len(line):
            value = line[index]
            if value == 92 and index + 1 < len(line):
                token.append(line[index + 1])
                index += 2
            elif value == 35:  # Unescaped Makefile comment.
                break
            elif value in (9, 32):
                if token:
                    prerequisites.add(bytes(token))
                    token.clear()
                index += 1
            elif value == 36 and line[index + 1 : index + 2] == b"$":
                token.append(value)
                index += 2
            else:
                token.append(value)
                index += 1
        if token:
            prerequisites.add(bytes(token))
    return prerequisites


def _validate_dep_info_source(
    dep_info: Path, expected_source: str, cargo_target_dir: Path, label: str
) -> None:
    _assert_no_symlink_components(dep_info, cargo_target_dir, f"{label} Cargo dep-info")
    if not dep_info.is_file():
        raise PackError(f"{label} Cargo dep-info is missing: {dep_info}")
    if os.fsencode(expected_source) not in _makefile_prerequisites(dep_info.read_bytes()):
        raise PackError(
            f"{label} Cargo dep-info does not list exact prerequisite {expected_source}: {dep_info}"
        )


def _fingerprinted_artifacts(
    fingerprints: Path,
    deps: Path,
    cargo_target_dir: Path,
    marker_name: str,
    artifact_prefix: str,
    label: str,
) -> list[Path]:
    matches: list[Path] = []
    fingerprint_prefix = "nagi-cli-"
    for fingerprint_dir in fingerprints.iterdir():
        if not fingerprint_dir.name.startswith(fingerprint_prefix):
            continue
        if fingerprint_dir.is_symlink():
            raise PackError(f"{label}: refusing symlink Cargo fingerprint {fingerprint_dir}")
        if not fingerprint_dir.is_dir():
            continue
        marker = fingerprint_dir / marker_name
        if marker.is_symlink():
            raise PackError(f"{label}: refusing symlink Cargo fingerprint marker {marker}")
        if not marker.is_file():
            continue
        artifact_hash = fingerprint_dir.name.removeprefix(fingerprint_prefix)
        candidate = deps / f"{artifact_prefix}{artifact_hash}"
        _assert_no_symlink_components(candidate, cargo_target_dir, label)
        if candidate.is_file() and os.access(candidate, os.X_OK):
            matches.append(candidate)
    return sorted(matches)


def discover_artifacts(cargo_target_dir: Path) -> dict[str, Path]:
    target_root = cargo_target_dir / TARGET
    debug = target_root / "debug"
    debug_cli = debug / "nagi"
    deps = debug / "deps"
    fixed = {"debug_cli": debug_cli}
    for label, path in fixed.items():
        _assert_no_symlink_components(path, cargo_target_dir, label)
        validate_static_x86_64_elf(path)

    _assert_no_symlink_components(deps, cargo_target_dir, "Cargo test artifact directory")
    if deps.is_symlink() or not deps.is_dir():
        raise PackError(f"Cargo test artifact directory is missing or redirected: {deps}")
    found: dict[str, Path] = dict(fixed)
    fingerprints = debug / ".fingerprint"
    _assert_no_symlink_components(fingerprints, cargo_target_dir, "Cargo fingerprint directory")
    if fingerprints.is_symlink() or not fingerprints.is_dir():
        raise PackError(f"Cargo fingerprint directory is missing or redirected: {fingerprints}")

    normal_bins = _fingerprinted_artifacts(
        fingerprints,
        deps,
        cargo_target_dir,
        "bin-nagi",
        "nagi-",
        "debug CLI Cargo artifact",
    )
    if len(normal_bins) != 1:
        raise PackError(
            f"expected exactly one fingerprinted Cargo bin artifact for debug_cli, found {len(normal_bins)}"
        )
    validate_static_x86_64_elf(normal_bins[0])
    _validate_dep_info_source(
        debug / "nagi.d",
        str(REPO_ROOT / "tools/nagi-cli/src/main.rs"),
        cargo_target_dir,
        "debug_cli",
    )
    _validate_dep_info_source(
        deps / f"{normal_bins[0].name}.d",
        "tools/nagi-cli/src/main.rs",
        cargo_target_dir,
        "debug_cli Cargo artifact",
    )
    if not filecmp.cmp(debug_cli, normal_bins[0], shallow=False):
        raise PackError("debug_cli does not match the fingerprinted Cargo bin artifact")

    for label, marker_name in HARNESS_FINGERPRINTS.items():
        matches = _fingerprinted_artifacts(
            fingerprints,
            deps,
            cargo_target_dir,
            marker_name,
            HARNESS_PREFIXES[label],
            label,
        )
        if len(matches) != 1:
            raise PackError(
                f"expected exactly one executable Cargo test artifact for {label} ({marker_name}), found {len(matches)}"
            )
        validate_static_x86_64_elf(matches[0])
        _validate_dep_info_source(
            deps / f"{matches[0].name}.d",
            HARNESS_SOURCES[label],
            cargo_target_dir,
            label,
        )
        found[label] = matches[0]
    return found


def _discover_build_artifacts(cargo_target_dir: Path) -> dict[str, Path]:
    artifacts = discover_artifacts(cargo_target_dir)
    release_cli = cargo_target_dir / TARGET / "release" / "nagi"
    _assert_no_symlink_components(release_cli, cargo_target_dir, "release CLI")
    validate_static_x86_64_elf(release_cli)
    ordered = {"release_cli": release_cli, **artifacts}
    return {label: ordered[label] for label in BUILD_ARTIFACT_LABELS}


def _build_manifest_data(
    repo_root: Path,
    cargo_target_dir: Path,
    source_paths: list[str],
    artifacts: dict[str, Path],
) -> dict[str, object]:
    support_hashes = _fingerprint_build_support(repo_root)
    build_script_hash = support_hashes[BUILD_SCRIPT_PATH]
    build_id = build_script_hash[:40]
    artifact_records: dict[str, dict[str, str]] = {}
    for label in BUILD_ARTIFACT_LABELS:
        path = artifacts[label]
        _validate_build_id(path, build_id, label)
        artifact_records[label] = {
            "path": path.relative_to(repo_root).as_posix(),
            "sha256": _sha256_file(path),
        }
    cargo_local_packages, cargo_local_sources = _cargo_local_dependency_inputs(repo_root)
    return {
        "schema_version": 2,
        "target": TARGET,
        "cargo_target_dir": cargo_target_dir.relative_to(repo_root).as_posix(),
        "build_script_sha256": build_script_hash,
        "build_id": build_id,
        "build_support_sha256": support_hashes,
        "source_sha256": _fingerprint_source_inputs(repo_root, source_paths),
        "cargo_local_dependency_packages": cargo_local_packages,
        "cargo_local_dependency_sha256": _fingerprint_source_inputs(
            repo_root, cargo_local_sources
        ),
        "artifacts": artifact_records,
    }


def _verify_build_manifest(path: Path, expected: dict[str, object]) -> None:
    if path.is_symlink() or not path.is_file():
        raise PackError(
            f"build provenance is missing; run build-linux-musl.sh first: {path}"
        )
    try:
        actual = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise PackError(f"cannot read build provenance manifest: {error}") from error
    if not isinstance(actual, dict):
        raise PackError("build provenance manifest must be a JSON object")
    if actual.get("build_script_sha256") != expected["build_script_sha256"]:
        raise PackError(
            "build provenance was recorded by a different build script; rerun the current build"
        )
    if actual.get("build_id") != expected["build_id"]:
        raise PackError("build provenance has a stale build-script ID")
    if actual.get("build_support_sha256") != expected["build_support_sha256"]:
        raise PackError("build support inputs changed since the Linux build")
    if actual.get("source_sha256") != expected["source_sha256"]:
        raise PackError("guest source inputs changed since the Linux build")
    if (
        actual.get("cargo_local_dependency_packages")
        != expected["cargo_local_dependency_packages"]
        or actual.get("cargo_local_dependency_sha256")
        != expected["cargo_local_dependency_sha256"]
    ):
        raise PackError("local Cargo dependency closure changed since the Linux build")
    if actual.get("artifacts") != expected["artifacts"]:
        raise PackError("Linux build artifacts changed since provenance was recorded")
    if actual != expected:
        raise PackError("build provenance metadata does not match current build inputs")


def record_build_manifest(
    repo_root: Path, build_root: Path, cargo_target_dir: Path
) -> Path:
    repo_root = repo_root.resolve(strict=True)
    build_root = _target_path(build_root, repo_root, "NAGI_BUILD_ROOT")
    cargo_target_dir = _target_path(cargo_target_dir, repo_root, "CARGO_TARGET_DIR")
    if not build_root.is_dir() or not cargo_target_dir.is_dir():
        raise PackError("musl build root or CARGO_TARGET_DIR does not exist")
    source_paths = _source_paths(repo_root)
    artifacts = _discover_build_artifacts(cargo_target_dir)
    guest_artifacts = {key: value for key, value in artifacts.items() if key != "release_cli"}
    _validate_compile_time_paths(guest_artifacts, repo_root, cargo_target_dir)
    manifest = _build_manifest_data(repo_root, cargo_target_dir, source_paths, artifacts)
    destination = build_root / BUILD_MANIFEST_NAME
    _assert_no_symlink_components(destination, build_root, "build provenance manifest")
    temporary = build_root / f".{BUILD_MANIFEST_NAME}.{os.getpid()}.tmp"
    if temporary.exists() or temporary.is_symlink():
        raise PackError(f"refusing to overwrite temporary build manifest: {temporary}")
    try:
        with temporary.open("x", encoding="utf-8") as stream:
            json.dump(manifest, stream, indent=2, sort_keys=True)
            stream.write("\n")
        os.replace(temporary, destination)
    except Exception:
        if temporary.exists():
            temporary.unlink()
        raise
    return destination


def _validate_compile_time_paths(
    artifacts: dict[str, Path], repo_root: Path, cargo_target_dir: Path
) -> None:
    crate_root = os.fsencode(str(repo_root / "tools/nagi-cli"))
    for label in ("nagi_cli", "cli", "run_receipt_cli"):
        if crate_root not in artifacts[label].read_bytes():
            raise PackError(
                f"{label} was built from another checkout; expected embedded root {repo_root}"
            )
    binary = os.fsencode(
        str(cargo_target_dir / TARGET / "debug" / "nagi")
    )
    tempdir = os.fsencode(str(cargo_target_dir / TARGET / "tmp"))
    receipt_test = artifacts["run_receipt_cli"].read_bytes()
    if binary not in receipt_test or tempdir not in receipt_test:
        raise PackError(
            "run_receipt_cli embeds a different CARGO_BIN_EXE_nagi or CARGO_TARGET_TMPDIR"
        )


def _verify_libc_cache(cache: Path) -> None:
    manifest = cache / "Cargo.toml"
    if cache.is_symlink() or not cache.is_dir() or not manifest.is_file():
        raise PackError(f"pinned libc Cargo source is missing: {cache}")
    text = manifest.read_text(encoding="utf-8")
    if 'name = "libc"' not in text or 'version = "0.2.189"' not in text:
        raise PackError(f"Cargo source cache is not libc 0.2.189: {cache}")


def _initialize_synthetic_git(
    rootfs: Path,
    template_dir: Path,
    required_tracked: Iterable[str],
    *,
    force_add: Iterable[str] = (),
) -> None:
    force_add = tuple(sorted(set(force_add)))
    if not set(force_add).issubset(set(required_tracked)):
        raise PackError("synthetic Git force-add inputs must also be required guest inputs")
    if not set(force_add).issubset(PINNED_SUPPLEMENTAL_FILES):
        raise PackError("synthetic Git may force-add only hash-pinned supplemental inputs")
    template_dir.mkdir()
    env = _git_env(
        {
            "GIT_AUTHOR_NAME": "Nagi CLI guest fixture",
            "GIT_AUTHOR_EMAIL": "nagi-cli-fixture@example.invalid",
            "GIT_COMMITTER_NAME": "Nagi CLI guest fixture",
            "GIT_COMMITTER_EMAIL": "nagi-cli-fixture@example.invalid",
            "GIT_AUTHOR_DATE": "2000-01-01T00:00:00Z",
            "GIT_COMMITTER_DATE": "2000-01-01T00:00:00Z",
        }
    )
    commands = (
        ["git", "init", "--quiet", "--initial-branch=codex/cli-guest-input", f"--template={template_dir}"],
        ["git", "add", "--all"],
    )
    for command in commands:
        result = subprocess.run(
            command,
            cwd=rootfs,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode:
            detail = result.stderr.decode("utf-8", errors="replace").strip()
            raise PackError(f"synthetic Git {' '.join(command[1:3])} failed: {detail}")

    if force_add:
        result = subprocess.run(
            ["git", "add", "-f", "--", *force_add],
            cwd=rootfs,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode:
            detail = result.stderr.decode("utf-8", errors="replace").strip()
            raise PackError(f"synthetic Git force-add failed: {detail}")

    tracked = {
        item
        for item in _run_git(rootfs, ["ls-files", "-z"]).decode("utf-8").split("\0")
        if item
    }
    missing = sorted(set(required_tracked) - tracked)
    if missing:
        preview = ", ".join(missing[:8])
        raise PackError(f"synthetic Git ignored required guest inputs: {preview}")

    commit = subprocess.run(
        ["git", "commit", "--quiet", "-m", "Create sparse CLI Linux guest input"],
        cwd=rootfs,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if commit.returncode:
        detail = commit.stderr.decode("utf-8", errors="replace").strip()
        raise PackError(f"synthetic Git commit failed: {detail}")


def _write_archive(rootfs: Path, archive: Path) -> None:
    if archive.exists() or archive.is_symlink():
        raise PackError(f"refusing to overwrite archive: {archive}")
    with archive.open("xb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as tar:
                for source in sorted(rootfs.rglob("*"), key=lambda item: item.relative_to(rootfs).as_posix()):
                    if source.is_symlink():
                        raise PackError(f"refusing symlink in guest root: {source}")
                    info = tarfile.TarInfo("./" + source.relative_to(rootfs).as_posix())
                    metadata = source.stat()
                    info.mode = stat.S_IMODE(metadata.st_mode) & 0o777
                    info.uid = 0
                    info.gid = 0
                    info.uname = ""
                    info.gname = ""
                    info.mtime = 0
                    if source.is_dir():
                        info.type = tarfile.DIRTYPE
                        tar.addfile(info)
                    elif source.is_file():
                        info.type = tarfile.REGTYPE
                        info.size = metadata.st_size
                        with source.open("rb") as stream:
                            tar.addfile(info, stream)
                    else:
                        raise PackError(f"refusing non-file guest input: {source}")


def safe_extract(archive: Path, destination: Path) -> None:
    """Extract only regular files and directories with paths below destination."""
    if destination.exists() or destination.is_symlink():
        raise PackError(f"refusing to extract over existing path: {destination}")
    destination.mkdir(parents=True)
    seen: set[str] = set()
    try:
        with tarfile.open(archive, mode="r:gz") as tar:
            for member in tar:
                name = PurePosixPath(member.name)
                if (
                    name.is_absolute()
                    or ".." in name.parts
                    or "\\" in member.name
                    or (name.parts and ":" in name.parts[0])
                ):
                    raise PackError(f"unsafe archive path: {member.name}")
                if not member.isdir() and not member.isfile():
                    raise PackError(f"archive contains a link or special file: {member.name}")
                parts = tuple(part for part in name.parts if part not in ("", "."))
                key = "/".join(parts) or "."
                if key in seen:
                    raise PackError(f"archive contains duplicate path: {member.name}")
                seen.add(key)
                target = destination.joinpath(*parts)
                if not _within(target, destination):
                    raise PackError(f"archive path escapes destination: {member.name}")
                if member.isdir():
                    target.mkdir(parents=True, exist_ok=True)
                    os.chmod(target, member.mode & 0o777)
                    continue
                target.parent.mkdir(parents=True, exist_ok=True)
                if target.exists() or target.is_symlink():
                    raise PackError(f"archive would overwrite extracted path: {member.name}")
                stream = tar.extractfile(member)
                if stream is None:
                    raise PackError(f"archive file has no contents: {member.name}")
                with stream, target.open("xb") as output:
                    shutil.copyfileobj(stream, output)
                os.chmod(target, member.mode & 0o777)
    except (tarfile.TarError, OSError) as error:
        if isinstance(error, PackError):
            raise
        raise PackError(f"cannot safely extract archive: {error}") from error


def _verify_rootfs(
    rootfs: Path,
    artifacts: dict[str, Path],
    repo_root: Path,
    build_root: Path,
    cargo_target_dir: Path,
) -> None:
    required = [
        "nagi",
        "Cargo.toml",
        "Cargo.lock",
        "nagi.toml",
        "rust-toolchain.toml",
        ".dev/workstreams.json",
        SYNTHETIC_WORKSTREAM_STATE_PATH,
        SYNTHETIC_INTEGRATION_STATE_PATH,
        "tools/nagi-cli/Cargo.toml",
        "tools/nagi-cli/scripts/install-alpine-test-deps.sh",
        "tools/nagi-cli/src/main.rs",
        "tools/nagi-cli/tests/cli.rs",
        "tools/nagi-cli/tests/run_receipt_cli.rs",
        ".github/workflows/ci.yml",
        "crates/nagi-abi/src/lib.rs",
        "tests/acceptance/registry.tsv",
        "tests/fixtures/m20_model_store_reader.rs",
        "third_party/sources.lock",
        "third_party/models.lock",
        "third_party/fonts.lock",
        str(cargo_target_dir.relative_to(repo_root) / TARGET / "tmp"),
        str(build_root.relative_to(repo_root) / "cargo-home" / LIBC_SOURCE / "Cargo.toml"),
        *PINNED_SUPPLEMENTAL_FILES.keys(),
    ]
    for relative in required:
        if not (rootfs / relative).exists():
            raise PackError(f"guest archive is missing required input: {relative}")
    _verify_fixed_guest_inputs(rootfs)
    for label, source in artifacts.items():
        relative = source.relative_to(repo_root)
        candidate = rootfs / relative
        validate_static_x86_64_elf(candidate)
        if not filecmp.cmp(candidate, source, shallow=False):
            raise PackError(f"guest archive changed artifact contents for {label}: {relative}")
    _validate_compile_time_paths(
        {label: rootfs / path.relative_to(repo_root) for label, path in artifacts.items()},
        repo_root,
        cargo_target_dir,
    )
    for current, directories, files in os.walk(rootfs, followlinks=False):
        for name in directories + files:
            path = Path(current) / name
            if path.is_symlink():
                raise PackError(f"guest root contains a symlink: {path.relative_to(rootfs)}")
    status = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=all"],
        cwd=rootfs,
        env=_git_env(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if status.returncode or status.stdout:
        raise PackError("synthetic guest Git checkout is not clean after artifacts were staged")


def pack_bundle(
    repo_root: Path,
    build_root: Path,
    cargo_target_dir: Path,
    bundle_dir: Path,
) -> tuple[Path, dict[str, Path]]:
    repo_root = repo_root.resolve(strict=True)
    build_root = _target_path(build_root, repo_root, "NAGI_BUILD_ROOT")
    cargo_target_dir = _target_path(cargo_target_dir, repo_root, "CARGO_TARGET_DIR")
    bundle_dir = _target_path(bundle_dir, repo_root, "NAGI_GUEST_BUNDLE_DIR")
    target_root = repo_root / "target"
    if build_root == target_root or not _within(build_root, target_root):
        raise PackError("NAGI_BUILD_ROOT must be a directory below repository target/")
    if not build_root.is_dir() or not cargo_target_dir.is_dir():
        raise PackError("musl build root or CARGO_TARGET_DIR does not exist")
    if bundle_dir.exists() or bundle_dir.is_symlink():
        raise PackError(f"refusing to overwrite existing bundle directory: {bundle_dir}")
    cargo_home = build_root / "cargo-home"
    if (
        _within(bundle_dir, cargo_target_dir)
        or _within(cargo_target_dir, bundle_dir)
        or _within(bundle_dir, cargo_home)
        or _within(cargo_home, bundle_dir)
    ):
        raise PackError("bundle output must not overlap CARGO_TARGET_DIR")

    build_artifacts = _discover_build_artifacts(cargo_target_dir)
    artifacts = {
        label: build_artifacts[label]
        for label in BUILD_ARTIFACT_LABELS
        if label != "release_cli"
    }
    _validate_compile_time_paths(artifacts, repo_root, cargo_target_dir)
    source_paths = _source_paths(repo_root)
    expected_manifest = _build_manifest_data(
        repo_root, cargo_target_dir, source_paths, build_artifacts
    )
    manifest_path = build_root / BUILD_MANIFEST_NAME
    _assert_no_symlink_components(manifest_path, build_root, "build provenance manifest")
    _verify_build_manifest(manifest_path, expected_manifest)
    cache = build_root / "cargo-home" / LIBC_SOURCE
    _assert_no_symlink_components(cache, build_root, "Cargo source cache")
    _verify_libc_cache(cache)

    bundle_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(
        tempfile.mkdtemp(prefix=f".{bundle_dir.name}.packing-", dir=bundle_dir.parent)
    )
    try:
        rootfs = temporary / "rootfs"
        rootfs.mkdir()
        for relative in source_paths:
            _copy_regular_file(repo_root, rootfs, relative)

        synthetic_workstream_paths = _write_synthetic_workstream_files(rootfs)
        supplemental_paths = _copy_pinned_supplemental_files(repo_root, rootfs)
        template_dir = temporary / "empty-git-template"
        _initialize_synthetic_git(
            rootfs,
            template_dir,
            [*source_paths, *synthetic_workstream_paths, *supplemental_paths],
            force_add=supplemental_paths,
        )
        template_dir.rmdir()

        build_relative = build_root.relative_to(repo_root)
        cargo_home_relative = build_relative / "cargo-home"
        staged_cache = rootfs / cargo_home_relative / LIBC_SOURCE
        _copy_regular_tree(cache, staged_cache)

        target_relative = cargo_target_dir.relative_to(repo_root)
        for label, source in artifacts.items():
            relative = source.relative_to(repo_root)
            destination = rootfs / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination, follow_symlinks=False)
        runtime_tmp = rootfs / target_relative / TARGET / "tmp"
        runtime_tmp.mkdir(parents=True, exist_ok=True)

        archive = temporary / "cli-guest-input.tar.gz"
        _write_archive(rootfs, archive)
        verification_root = temporary / ".verify-extract"
        safe_extract(archive, verification_root)
        _verify_rootfs(
            verification_root, artifacts, repo_root, build_root, cargo_target_dir
        )
        shutil.rmtree(verification_root)

        if bundle_dir.exists() or bundle_dir.is_symlink():
            raise PackError(f"refusing to overwrite existing bundle directory: {bundle_dir}")
        os.rename(temporary, bundle_dir)
    except Exception:
        if temporary.exists():
            shutil.rmtree(temporary)
        raise
    return bundle_dir / "cli-guest-input.tar.gz", artifacts


def main(argv: Iterable[str] | None = None) -> int:
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-root", default=os.environ.get("NAGI_BUILD_ROOT"))
    parser.add_argument("--cargo-target-dir", default=os.environ.get("CARGO_TARGET_DIR"))
    parser.add_argument("--bundle-dir", default=os.environ.get("NAGI_GUEST_BUNDLE_DIR"))
    parser.add_argument("--record-build-manifest", action="store_true")
    args = parser.parse_args(list(argv) if argv is not None else None)

    build_value = args.build_root or str(REPO_ROOT / "target/nagi-cli-linux-musl")
    build_root = _target_path(build_value, REPO_ROOT, "NAGI_BUILD_ROOT")
    cargo_value = args.cargo_target_dir or str(build_root / "cargo-target")
    cargo_target_dir = _target_path(cargo_value, REPO_ROOT, "CARGO_TARGET_DIR")
    if args.record_build_manifest:
        if args.bundle_dir:
            parser.error("--bundle-dir cannot be combined with --record-build-manifest")
        try:
            manifest = record_build_manifest(REPO_ROOT, build_root, cargo_target_dir)
        except (PackError, OSError, subprocess.SubprocessError) as error:
            print(f"FAIL Linux musl build provenance: {error}", file=sys.stderr)
            return 2
        print(f"BUILD_MANIFEST={manifest}")
        return 0
    if args.bundle_dir:
        bundle_value = args.bundle_dir
    else:
        stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        bundle_value = build_root / f"guest-input-{stamp}-{os.getpid()}"
    bundle_dir = _target_path(bundle_value, REPO_ROOT, "NAGI_GUEST_BUNDLE_DIR")
    try:
        archive, artifacts = pack_bundle(
            REPO_ROOT, build_root, cargo_target_dir, bundle_dir
        )
    except (PackError, OSError, subprocess.SubprocessError) as error:
        print(f"FAIL Linux musl guest pack: {error}", file=sys.stderr)
        return 2

    print(f"GUEST_ARCHIVE={archive}")
    print(f"GUEST_ROOT={REPO_ROOT}")
    print(f"GUEST_CARGO_HOME={build_root.relative_to(REPO_ROOT) / 'cargo-home'}")
    print(f"GUEST_TARGET_TMPDIR={cargo_target_dir.relative_to(REPO_ROOT) / TARGET / 'tmp'}")
    for label in ("debug_cli", "nagi_cli", "main_test", "cli", "run_receipt_cli"):
        source = artifacts[label]
        print(f"GUEST_{label.upper()}={source.relative_to(REPO_ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
