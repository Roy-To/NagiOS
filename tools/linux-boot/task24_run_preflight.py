#!/usr/bin/env python3
"""Read-only preflight for a planned Task24 save/read pair.

This tool deliberately has no QEMU or image-creation path. It validates a
manifest emitted by a future integrated runner before a separately authorized
interactive start.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import sys
import uuid
from pathlib import Path
from typing import Any, BinaryIO


SCHEMA = "nagi-task24-paired-run-preflight/v1"
MAX_MANIFEST_BYTES = 1 << 20
MAX_PHASE_TIMEOUT_SECONDS = 600
ARTIFACTS = ("kernel", "initramfs", "system_image")
PHASES = ("save", "read")


class PreflightError(ValueError):
    """The manifest does not describe a safe paired Task24 run."""


class _PinnedArtifact:
    def __init__(
        self, path: Path, digest: str, source: BinaryIO, identity: tuple[int, ...]
    ) -> None:
        self.path = path
        self.digest = digest
        self.source = source
        self.identity = identity

    def close(self) -> None:
        self.source.close()


def _mapping(value: Any, name: str, keys: set[str]) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != keys:
        raise PreflightError(f"{name} has an unsupported field set")
    return value


def _canonical_path(value: Any, name: str, *, must_exist: bool) -> Path:
    if not isinstance(value, str) or not value or "\x00" in value:
        raise PreflightError(f"{name} must be a non-empty path")
    path = Path(value)
    if not path.is_absolute() or value != path.as_posix():
        raise PreflightError(f"{name} must be an absolute normalized path")
    try:
        canonical = path.resolve(strict=must_exist)
    except OSError as error:
        raise PreflightError(f"{name} is unavailable ({error.__class__.__name__})") from None
    if canonical != path:
        raise PreflightError(f"{name} must not traverse symlinks")
    if must_exist:
        try:
            metadata = path.lstat()
        except OSError as error:
            raise PreflightError(f"{name} is unavailable ({error.__class__.__name__})") from None
        if stat.S_ISLNK(metadata.st_mode):
            raise PreflightError(f"{name} must not be a symlink")
    else:
        try:
            parent = path.parent.resolve(strict=True)
        except OSError as error:
            raise PreflightError(f"{name} parent is unavailable ({error.__class__.__name__})") from None
        if parent != path.parent:
            raise PreflightError(f"{name} parent must not traverse symlinks")
    return path


def _regular_file(path: Path, name: str) -> os.stat_result:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise PreflightError(f"{name} is unavailable ({error.__class__.__name__})") from None
    if not stat.S_ISREG(metadata.st_mode):
        raise PreflightError(f"{name} must be a regular file")
    if metadata.st_size == 0:
        raise PreflightError(f"{name} must not be empty")
    return metadata


def _stat_identity(metadata: os.stat_result) -> tuple[int, ...]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_mode,
        metadata.st_nlink,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def _opened_regular_file(source: BinaryIO, name: str) -> os.stat_result:
    try:
        metadata = os.fstat(source.fileno())
    except OSError as error:
        raise PreflightError(f"{name} descriptor cannot be checked ({error.__class__.__name__})") from None
    if not stat.S_ISREG(metadata.st_mode):
        raise PreflightError(f"{name} must be a regular file")
    if metadata.st_size == 0:
        raise PreflightError(f"{name} must not be empty")
    return metadata


def _require_path_matches_descriptor(
    path: Path, descriptor_stat: os.stat_result, name: str
) -> None:
    _canonical_path(path.as_posix(), f"{name}.path", must_exist=True)
    try:
        path_stat = path.lstat()
    except OSError as error:
        raise PreflightError(f"{name} path changed ({error.__class__.__name__})") from None
    if stat.S_ISLNK(path_stat.st_mode) or not stat.S_ISREG(path_stat.st_mode):
        raise PreflightError(f"{name} path no longer names the opened regular file")
    if _stat_identity(path_stat) != _stat_identity(descriptor_stat):
        raise PreflightError(f"{name} path no longer matches the opened file descriptor")


def _hash_open_file(source: BinaryIO, name: str) -> str:
    digest = hashlib.sha256()
    try:
        os.lseek(source.fileno(), 0, os.SEEK_SET)
        while True:
            chunk = os.read(source.fileno(), 1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    except OSError as error:
        raise PreflightError(f"{name} descriptor cannot be read ({error.__class__.__name__})") from None
    return digest.hexdigest()


def _verify_pinned_artifact_contents(artifact: _PinnedArtifact, name: str) -> None:
    before = _opened_regular_file(artifact.source, name)
    if _stat_identity(before) != artifact.identity:
        raise PreflightError(f"{name} changed after its initial hash")
    _require_path_matches_descriptor(artifact.path, before, name)
    actual = _hash_open_file(artifact.source, name)
    after = _opened_regular_file(artifact.source, name)
    if _stat_identity(after) != artifact.identity:
        raise PreflightError(f"{name} changed while being rechecked")
    _require_path_matches_descriptor(artifact.path, after, name)
    if actual != artifact.digest:
        raise PreflightError(f"{name} digest changed after its initial hash")


def _verify_pinned_artifact_identity(artifact: _PinnedArtifact, name: str) -> None:
    current = _opened_regular_file(artifact.source, name)
    if _stat_identity(current) != artifact.identity:
        raise PreflightError(f"{name} changed before preflight PASS")
    _require_path_matches_descriptor(artifact.path, current, name)


def _artifact(
    value: Any,
    name: str,
    run_directory: Path,
) -> _PinnedArtifact:
    artifact = _mapping(value, name, {"path", "sha256"})
    path = _canonical_path(artifact["path"], f"{name}.path", must_exist=True)
    if path == run_directory or run_directory in path.parents:
        raise PreflightError(f"{name} must be a fixed input outside the run directory")
    expected = artifact["sha256"]
    if not isinstance(expected, str) or re.fullmatch(r"[0-9a-f]{64}", expected) is None:
        raise PreflightError(f"{name}.sha256 must be lowercase SHA-256 hex")
    flags = os.O_RDONLY
    flags |= getattr(os, "O_CLOEXEC", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    flags |= getattr(os, "O_NONBLOCK", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise PreflightError(f"{name} cannot be opened read-only ({error.__class__.__name__})") from None
    try:
        source = os.fdopen(descriptor, "rb", buffering=0)
    except OSError as error:
        os.close(descriptor)
        raise PreflightError(f"{name} descriptor cannot be opened ({error.__class__.__name__})") from None

    try:
        before = _opened_regular_file(source, name)
        initial_identity = _stat_identity(before)
        _require_path_matches_descriptor(path, before, name)
        actual = _hash_open_file(source, name)
        after = _opened_regular_file(source, name)
        if _stat_identity(after) != initial_identity:
            raise PreflightError(f"{name} changed while being hashed")
        _require_path_matches_descriptor(path, after, name)
        if actual != expected:
            raise PreflightError(f"{name} digest does not match the manifest")
        return _PinnedArtifact(path, actual, source, initial_identity)
    except BaseException:
        source.close()
        raise


def _directory_identity(path: Path, name: str) -> tuple[int, int]:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise PreflightError(f"{name} is unavailable ({error.__class__.__name__})") from None
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise PreflightError(f"{name} must be an actual directory")
    try:
        if path.resolve(strict=True) != path:
            raise PreflightError(f"{name} must not traverse symlinks")
    except OSError as error:
        raise PreflightError(f"{name} is unavailable ({error.__class__.__name__})") from None
    return metadata.st_dev, metadata.st_ino


def _serial_parent_identity(
    serial_path: Path, phase: str, run_directory: Path
) -> tuple[int, int]:
    expected_path = run_directory / phase / "serial.log"
    current_path = _canonical_path(
        serial_path.as_posix(), f"phases.{phase}.serial_path", must_exist=False
    )
    if current_path != expected_path:
        raise PreflightError(f"phases.{phase} serial path is not phase-local")
    parent_identity = _directory_identity(
        current_path.parent, f"phases.{phase} serial parent"
    )
    if parent_identity[0] != run_directory.stat().st_dev:
        raise PreflightError(f"phases.{phase} serial path leaves the run filesystem")
    try:
        current_path.lstat()
    except FileNotFoundError:
        pass
    except OSError as error:
        raise PreflightError(
            f"phases.{phase} serial path cannot be checked ({error.__class__.__name__})"
        ) from None
    else:
        raise PreflightError(f"phases.{phase} serial path already exists")
    final_parent_identity = _directory_identity(
        current_path.parent, f"phases.{phase} serial parent"
    )
    if final_parent_identity != parent_identity:
        raise PreflightError(
            f"phases.{phase} serial parent identity changed during validation"
        )
    return final_parent_identity


def _recheck_serial_destinations(
    run_directory: Path,
    serial_paths: list[Path],
    expected_identities: list[tuple[int, int]],
) -> None:
    for phase, serial_path, expected in zip(PHASES, serial_paths, expected_identities):
        actual = _serial_parent_identity(serial_path, phase, run_directory)
        if actual != expected:
            raise PreflightError(
                f"phases.{phase} serial parent identity changed before preflight PASS"
            )


def _validate_pair_with_artifacts(
    manifest: dict[str, Any],
    run_directory: Path,
    run_id: str,
    source_commit: str,
    artifacts: dict[str, _PinnedArtifact],
) -> dict[str, Any]:
    data_value = _mapping(
        manifest["data_work_copy"],
        "data_work_copy",
        {"path", "created_for_run_id", "provisioning", "filesystem_uuid"},
    )
    data_path = _canonical_path(data_value["path"], "data_work_copy.path", must_exist=True)
    data_stat = _regular_file(data_path, "data_work_copy")
    if data_path != run_directory / "data.raw":
        raise PreflightError("Data work copy must be the run-local data.raw file")
    if data_stat.st_dev != run_directory.stat().st_dev:
        raise PreflightError("Data work copy must stay on the run directory filesystem")
    if data_value["created_for_run_id"] != run_id:
        raise PreflightError("Data work copy is not assigned to this run")
    if data_value["provisioning"] != "fresh-ext4":
        raise PreflightError("Data work copy must declare fresh-ext4 provisioning")
    if data_stat.st_nlink != 1:
        raise PreflightError("Data work copy must not alias another path by hard link")
    try:
        data_uuid = str(uuid.UUID(data_value["filesystem_uuid"]))
    except (AttributeError, TypeError, ValueError):
        raise PreflightError("Data work copy filesystem_uuid is invalid") from None
    if data_uuid != data_value["filesystem_uuid"]:
        raise PreflightError("Data work copy filesystem_uuid is not canonical")
    system_stat = os.fstat(artifacts["system_image"].source.fileno())
    if (data_stat.st_dev, data_stat.st_ino) == (system_stat.st_dev, system_stat.st_ino):
        raise PreflightError("System and Data must be separate backing files")

    phases_value = _mapping(manifest["phases"], "phases", set(PHASES))
    phase_summary: dict[str, int] = {}
    serial_paths: list[Path] = []
    serial_directory_identities: list[tuple[int, int]] = []
    for phase in PHASES:
        entry = _mapping(
            phases_value[phase],
            f"phases.{phase}",
            {
                "kernel",
                "initramfs",
                "system_image",
                "data_image",
                "filesystem_uuid",
                "serial_path",
                "timeout_seconds",
                "monitor",
                "qmp_enabled",
                "serial_transport",
                "interactive_input",
                "start_authorized",
                "system_read_only",
                "data_read_only",
            },
        )
        for name in ARTIFACTS:
            artifact = artifacts[name]
            reference = entry[name]
            if not isinstance(reference, dict) or set(reference) != {"path", "sha256"}:
                raise PreflightError(f"phases.{phase}.{name} must reference one pinned artifact")
            phase_path = _canonical_path(
                reference["path"], f"phases.{phase}.{name}.path", must_exist=True
            )
            if phase_path != artifact.path or reference["sha256"] != artifact.digest:
                raise PreflightError(f"phases.{phase}.{name} differs from the fixed candidate")
        phase_data = _canonical_path(
            entry["data_image"], f"phases.{phase}.data_image", must_exist=True
        )
        if phase_data != data_path:
            raise PreflightError(f"phases.{phase} does not use the same new Data work copy")
        if entry["filesystem_uuid"] != data_uuid:
            raise PreflightError(f"phases.{phase} Data UUID differs from the work copy")

        serial_path = _canonical_path(
            entry["serial_path"], f"phases.{phase}.serial_path", must_exist=False
        )
        if serial_path != run_directory / phase / "serial.log":
            raise PreflightError(f"phases.{phase} serial path is not phase-local")
        parent_identity = _serial_parent_identity(serial_path, phase, run_directory)
        serial_paths.append(serial_path)
        serial_directory_identities.append(parent_identity)

        timeout = entry["timeout_seconds"]
        if isinstance(timeout, bool) or not isinstance(timeout, int):
            raise PreflightError(f"phases.{phase} timeout_seconds must be an integer")
        if not 1 <= timeout <= MAX_PHASE_TIMEOUT_SECONDS:
            raise PreflightError(f"phases.{phase} timeout_seconds is outside the allowed range")
        if entry["monitor"] != "none" or entry["qmp_enabled"] is not False:
            raise PreflightError(f"phases.{phase} must disable QMP and the monitor")
        if entry["serial_transport"] != "file":
            raise PreflightError(f"phases.{phase} must use its own serial file")
        if entry["interactive_input"] != "manual" or entry["start_authorized"] is not False:
            raise PreflightError(f"phases.{phase} must await a separate explicit start signal")
        if entry["system_read_only"] is not True or entry["data_read_only"] is not False:
            raise PreflightError(f"phases.{phase} System/Data access mode is invalid")
        phase_summary[phase] = timeout

    if serial_paths[0] == serial_paths[1]:
        raise PreflightError("save and read must use distinct serial files")
    if serial_directory_identities[0] == serial_directory_identities[1]:
        raise PreflightError("save and read serial parents must be distinct directories")

    result = {
        "preflight": "PASS",
        "run_id": run_id,
        "candidate_source_commit": source_commit,
        "kernel_sha256": artifacts["kernel"].digest,
        "initramfs_sha256": artifacts["initramfs"].digest,
        "system_sha256": artifacts["system_image"].digest,
        "same_system_candidate": True,
        "same_data_work_copy": True,
        "data_freshness": "manifest-declared fresh-ext4",
        "data_filesystem_uuid": data_uuid,
        "serial_files_separate": True,
        "phase_timeouts_seconds": phase_summary,
        "qmp_enabled": False,
        "interactive_start_authorized": False,
        "qemu_started": False,
    }
    for name, artifact in artifacts.items():
        _verify_pinned_artifact_contents(artifact, name)
    for name, artifact in artifacts.items():
        _verify_pinned_artifact_identity(artifact, name)
    _recheck_serial_destinations(
        run_directory, serial_paths, serial_directory_identities
    )
    return result


def preflight(manifest_path: Path) -> dict[str, Any]:
    manifest_path = _canonical_path(manifest_path.as_posix(), "manifest", must_exist=True)
    _regular_file(manifest_path, "manifest")
    if manifest_path.stat().st_size > MAX_MANIFEST_BYTES:
        raise PreflightError("manifest exceeds the size limit")
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise PreflightError(f"manifest cannot be read ({error.__class__.__name__})") from None

    manifest = _mapping(
        manifest,
        "manifest",
        {
            "schema",
            "run_id",
            "run_directory",
            "candidate_source_commit",
            "artifacts",
            "data_work_copy",
            "phases",
            "interactive_start_authorized",
        },
    )
    if manifest["schema"] != SCHEMA:
        raise PreflightError("unsupported manifest schema")
    run_id = manifest["run_id"]
    if not isinstance(run_id, str) or re.fullmatch(r"task24-[a-z0-9][a-z0-9-]{0,48}", run_id) is None:
        raise PreflightError("run_id is not canonical")
    if manifest["interactive_start_authorized"] is not False:
        raise PreflightError("preflight manifest must not authorize interactive start")

    run_directory = _canonical_path(
        manifest["run_directory"], "run_directory", must_exist=True
    )
    if not run_directory.is_dir() or manifest_path.parent != run_directory:
        raise PreflightError("manifest must reside in its run directory")
    if run_directory.name != run_id:
        raise PreflightError("run directory name must match run_id")

    source_commit = manifest["candidate_source_commit"]
    if not isinstance(source_commit, str) or re.fullmatch(r"[0-9a-f]{40}", source_commit) is None:
        raise PreflightError("candidate_source_commit must be a full Git object ID")

    artifacts_value = _mapping(manifest["artifacts"], "artifacts", set(ARTIFACTS))
    artifacts: dict[str, _PinnedArtifact] = {}
    try:
        for name in ARTIFACTS:
            artifacts[name] = _artifact(artifacts_value[name], name, run_directory)
        return _validate_pair_with_artifacts(
            manifest, run_directory, run_id, source_commit, artifacts
        )
    finally:
        for artifact in artifacts.values():
            artifact.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument(
        "--preflight-only",
        action="store_true",
        required=True,
        help="required: this entry point never launches QEMU",
    )
    args = parser.parse_args()
    try:
        result = preflight(args.manifest)
    except (OSError, PreflightError) as error:
        print(f"task24_preflight=FAIL reason={error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
