"""Immutable input receipts for resumable L11-3 kernel builds."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import stat
import subprocess
from pathlib import Path


class BuildInputError(ValueError):
    pass


def sha256_file(path: Path) -> str:
    try:
        info = path.lstat()
    except FileNotFoundError:
        raise BuildInputError(f"missing build input: {path}") from None
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise BuildInputError(f"build input must be a non-symlink regular file: {path}")
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    digest = hashlib.sha256()
    try:
        opened = os.fstat(fd)
        if (opened.st_dev, opened.st_ino) != (info.st_dev, info.st_ino):
            raise BuildInputError(f"build input changed while opening: {path}")
        with os.fdopen(fd, "rb", closefd=False) as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
        after = os.fstat(fd)
        if (opened.st_dev, opened.st_ino, opened.st_size, opened.st_mtime_ns) != (
            after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns
        ):
            raise BuildInputError(f"build input changed while hashing: {path}")
    finally:
        os.close(fd)
    return digest.hexdigest()


def _canonical_path(path: Path) -> str:
    try:
        return str(path.resolve(strict=True))
    except OSError as error:
        raise BuildInputError(f"build input path is unavailable: {path}: {error}") from None


def _canonical_executable(path: Path) -> Path:
    try:
        resolved = path.expanduser().resolve(strict=True)
        info = resolved.stat()
    except OSError as error:
        raise BuildInputError(f"tool path is unavailable: {path}: {error}") from None
    if not stat.S_ISREG(info.st_mode) or not os.access(resolved, os.X_OK):
        raise BuildInputError(f"tool path must resolve to a regular executable: {path}")
    return resolved


def resolve_rustc_path(requested: str) -> Path:
    """Resolve rustc shims to the actual active compiler executable."""
    if not isinstance(requested, str) or not requested:
        raise BuildInputError("requested rustc path is missing")
    if "/" in requested or (os.altsep and os.altsep in requested):
        candidate = Path(requested).expanduser()
    else:
        found = shutil.which(requested)
        if found is None:
            raise BuildInputError(f"requested rustc executable was not found: {requested}")
        candidate = Path(found)
    resolved = _canonical_executable(candidate)
    if resolved.name.lower() in {"rustup", "rustup.exe"}:
        try:
            result = subprocess.run(
                [str(resolved), "which", "rustc"],
                check=False,
                capture_output=True,
                text=True,
            )
        except OSError as error:
            raise BuildInputError(f"rustup could not resolve the active rustc: {error}") from None
        if result.returncode != 0:
            detail = result.stderr.strip() or f"exit {result.returncode}"
            raise BuildInputError(f"rustup could not resolve the active rustc: {detail}")
        paths = [line.strip() for line in result.stdout.splitlines() if line.strip()]
        if len(paths) != 1:
            raise BuildInputError("rustup returned an invalid active rustc path")
        resolved = _canonical_executable(Path(paths[0]))
        if resolved.name.lower() in {"rustup", "rustup.exe"}:
            raise BuildInputError("rustup did not return the active rustc executable")
    return resolved


def executable_identity(path: Path) -> dict[str, str]:
    """Capture a stable executable hash and its full rustc version output."""
    resolved = _canonical_executable(path)
    try:
        before = resolved.stat()
        digest_before = sha256_file(resolved)
        result = subprocess.run(
            [str(resolved), "--version", "--verbose"],
            check=False,
            capture_output=True,
            text=True,
        )
        digest_after = sha256_file(resolved)
        after = resolved.stat()
    except OSError as error:
        raise BuildInputError(f"pinned rustc identity query failed: {error}") from None
    before_state = (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
    after_state = (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)
    if digest_before != digest_after or before_state != after_state:
        raise BuildInputError("pinned rustc executable changed while querying its identity")
    if result.returncode != 0:
        detail = result.stderr.strip() or f"exit {result.returncode}"
        raise BuildInputError(f"pinned rustc version query failed: {detail}")
    version = result.stdout.rstrip("\r\n")
    if not version:
        raise BuildInputError("pinned rustc version output is empty")
    return {"path": str(resolved), "sha256": digest_after, "version": version}


def verify_executable_identity(path: Path, expected_sha256: str, expected_version: str, name: str) -> None:
    """Fail if a pinned build tool no longer matches its captured identity."""
    actual = executable_identity(path)
    if actual["sha256"] != expected_sha256:
        raise BuildInputError(f"pinned {name} executable changed")
    if actual["version"] != expected_version:
        raise BuildInputError(f"pinned {name} version changed")


def verify_manifest_tool_identity(
    manifest_path: Path,
    name: str,
    expected_path: Path,
    expected_sha256: str,
    expected_version: str,
) -> None:
    """Ensure the immutable input manifest and build receipt name one tool."""
    try:
        info = manifest_path.lstat()
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
            raise BuildInputError("resume input manifest must be a non-symlink regular file")
        manifest = json.loads(manifest_path.read_bytes())
    except FileNotFoundError:
        raise BuildInputError("resume input manifest is missing") from None
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise BuildInputError("resume input manifest is malformed") from None
    expected_record = {"path": _canonical_path(expected_path), "sha256": expected_sha256}
    inputs = manifest.get("inputs") if isinstance(manifest, dict) else None
    versions = manifest.get("tool_versions") if isinstance(manifest, dict) else None
    if not isinstance(inputs, dict) or inputs.get(name) != expected_record:
        raise BuildInputError(f"resume manifest {name} identity differs from the build receipt")
    if not isinstance(versions, dict) or versions.get(name) != expected_version:
        raise BuildInputError(f"resume manifest {name} version differs from the build receipt")


def _record(path: Path) -> dict[str, str]:
    return {"path": _canonical_path(path), "sha256": sha256_file(path)}


def _source_record(source_dir: Path, archive_path: Path, identity: dict[str, str]) -> dict[str, str]:
    if not isinstance(identity, dict):
        raise BuildInputError("pinned source identity is malformed")
    archive_sha = identity.get("archive_sha256")
    tree_sha = identity.get("tree_sha256")
    if not isinstance(archive_sha, str) or len(archive_sha) != 64 or not isinstance(tree_sha, str) or len(tree_sha) != 64:
        raise BuildInputError("pinned source identity hashes are malformed")
    try:
        source_info = source_dir.lstat()
    except FileNotFoundError:
        raise BuildInputError(f"missing extracted source directory: {source_dir}") from None
    if stat.S_ISLNK(source_info.st_mode) or not stat.S_ISDIR(source_info.st_mode):
        raise BuildInputError("extracted source directory must be a non-symlink directory")
    return {
        "source_dir": _canonical_path(source_dir),
        "archive_path": _canonical_path(archive_path),
        "archive_sha256": archive_sha,
        "tree_sha256": tree_sha,
    }


def _host_support_record(path: Path | None) -> dict[str, str | None]:
    if path is None:
        return {"path": None, "sha256": None}
    return {"path": _canonical_path(path), "sha256": sha256_file(path)}


def resume_tool_path(manifest_path: Path, name: str) -> Path:
    """Read an immutable tool path before validating the full resume manifest."""
    try:
        info = manifest_path.lstat()
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
            raise BuildInputError("resume input manifest must be a non-symlink regular file")
        receipt = json.loads(manifest_path.read_bytes())
    except FileNotFoundError:
        raise BuildInputError("resume input manifest is missing; use a new output directory") from None
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise BuildInputError("resume input manifest is malformed; use a new output directory") from None
    if not isinstance(receipt, dict) or receipt.get("schema") != 2:
        raise BuildInputError("resume input manifest schema is invalid; use a new output directory")
    inputs = receipt.get("inputs")
    record = inputs.get(name) if isinstance(inputs, dict) else None
    path_value = record.get("path") if isinstance(record, dict) else None
    digest_value = record.get("sha256") if isinstance(record, dict) else None
    if not isinstance(path_value, str) or not isinstance(digest_value, str):
        raise BuildInputError(f"resume input manifest has no pinned {name} path")
    if len(digest_value) != 64 or any(character not in "0123456789abcdef" for character in digest_value):
        raise BuildInputError(f"resume input manifest has a malformed {name} hash")
    path = Path(path_value)
    if not path.is_absolute():
        raise BuildInputError(f"resume pinned {name} path is not absolute")
    try:
        resolved = path.resolve(strict=True)
        tool_info = resolved.stat()
    except OSError as error:
        raise BuildInputError(f"resume pinned {name} path is unavailable: {error}") from None
    if str(resolved) != path_value or not stat.S_ISREG(tool_info.st_mode) or not os.access(resolved, os.X_OK):
        raise BuildInputError(f"resume pinned {name} path is not a canonical executable")
    if sha256_file(resolved) != digest_value:
        raise BuildInputError(f"resume pinned {name} executable changed")
    return resolved


def _tool_versions_record(tool_versions: dict[str, str]) -> dict[str, str]:
    if not isinstance(tool_versions, dict) or not tool_versions:
        raise BuildInputError("build tool versions are missing")
    records: dict[str, str] = {}
    for name, version in tool_versions.items():
        if not isinstance(name, str) or not name.isidentifier() or not isinstance(version, str) or not version:
            raise BuildInputError("build tool version record is malformed")
        records[name] = version
    return dict(sorted(records.items()))


def _manifest(
    input_paths: dict[str, Path],
    source_dir: Path,
    archive_path: Path,
    source_identity: dict[str, str],
    host_support_path: Path | None,
    tool_versions: dict[str, str],
) -> dict:
    if not input_paths or any(not isinstance(name, str) for name in input_paths):
        raise BuildInputError("build input set is malformed")
    return {
        "schema": 2,
        "inputs": {name: _record(input_paths[name]) for name in sorted(input_paths)},
        "source": _source_record(source_dir, archive_path, source_identity),
        "host_support": _host_support_record(host_support_path),
        "tool_versions": _tool_versions_record(tool_versions),
    }


def write_resume_manifest(
    manifest_path: Path,
    input_paths: dict[str, Path],
    source_dir: Path,
    archive_path: Path,
    source_identity: dict[str, str],
    host_support_path: Path | None,
    tool_versions: dict[str, str],
) -> str:
    payload = (json.dumps(
        _manifest(input_paths, source_dir, archive_path, source_identity, host_support_path, tool_versions),
        indent=2,
        sort_keys=True,
    ) + "\n").encode("utf-8")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(manifest_path, flags, 0o444)
    except FileExistsError:
        raise BuildInputError("resume input manifest already exists") from None
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
            os.fchmod(stream.fileno(), 0o444)
    except BaseException:
        try:
            manifest_path.unlink()
        except OSError:
            pass
        raise
    return hashlib.sha256(payload).hexdigest()


def validate_resume_manifest(
    manifest_path: Path,
    input_paths: dict[str, Path],
    source_dir: Path,
    archive_path: Path,
    source_identity: dict[str, str],
    host_support_path: Path | None,
    tool_versions: dict[str, str],
) -> str:
    try:
        info = manifest_path.lstat()
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
            raise BuildInputError("resume input manifest must be a non-symlink regular file")
        data = manifest_path.read_bytes()
        receipt = json.loads(data)
    except FileNotFoundError:
        raise BuildInputError("resume input manifest is missing; use a new output directory") from None
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise BuildInputError("resume input manifest is malformed; use a new output directory") from None
    expected = _manifest(input_paths, source_dir, archive_path, source_identity, host_support_path, tool_versions)
    if receipt != expected:
        if not isinstance(receipt, dict) or receipt.get("schema") != 2:
            raise BuildInputError("resume input manifest schema is invalid; use a new output directory")
        expected_inputs = expected["inputs"]
        actual_inputs = receipt.get("inputs")
        if not isinstance(actual_inputs, dict):
            raise BuildInputError("resume input records are malformed; use a new output directory")
        for name in sorted(expected_inputs):
            if actual_inputs.get(name) != expected_inputs[name]:
                raise BuildInputError(f"resume build input changed: {name}; use a new output directory")
        if receipt.get("source") != expected["source"]:
            raise BuildInputError("resume pinned Linux source identity changed; use a new output directory")
        if receipt.get("host_support") != expected["host_support"]:
            raise BuildInputError("resume host-support manifest changed; use a new output directory")
        actual_versions = receipt.get("tool_versions")
        for name, expected_version in expected["tool_versions"].items():
            if not isinstance(actual_versions, dict) or actual_versions.get(name) != expected_version:
                raise BuildInputError(f"resume build tool version changed: {name}; use a new output directory")
        raise BuildInputError("resume input manifest contains unexpected fields; use a new output directory")
    return hashlib.sha256(data).hexdigest()
