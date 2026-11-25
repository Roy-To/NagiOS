#!/usr/bin/env python3
"""Run a bounded Task24 save/reopen trial from the public preflight.

The fixed-head preflight remains read-only and never starts QEMU. This runner
requires a separate explicit start flag and a board readback hash, uses the
same manifest-declared Data image across boots, and never configures QMP. Input
can be manual Cocoa operation or a manifest-bound loopback RFB development
fixture; neither path claims that post-action display content was observed.
It summarizes guest MemTotal/MemAvailable telemetry already present on serial,
but does not capture pixels or align the guest clock with host input/present
timestamps. A completed runner receipt is not Task24 acceptance: artifact
source provenance is unverified, Data freshness is manifest-declared, and
post-action display content is not observed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import selectors
import signal
import stat
import subprocess
import sys
import time
import uuid
from dataclasses import dataclass, replace
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable

import boot_acceptance
import task24_recovery_serials as recovery
from task24_rfb_automatic import (
    AutomationError,
    AutomaticRfbInput,
    DevelopmentFixture,
    L9_RFB_FLOW,
    l9_flow_markers,
    load_development_fixture,
)


REPO_ROOT = Path(__file__).resolve().parents[2]
QEMU_CHILD_RUNNER = REPO_ROOT / "tools" / "linux-boot" / "task24_run_qemu_child.py"
TASK24_PREFLIGHT_SHA256 = "b9f96eafbd861813fe42af5aff6cf329b09cbdbdf716e95edaf06b0046c3ef9b"
TASK24_SCHEMA = "nagi-task24-paired-run-preflight/v1"
TASK24_HEAD = "118d1554c3567e6b643bc6a66c0e89555b4ee40a"
TASK24_NOTES_SAVE_RETEST_HEAD = "734e8dae2be946dd0e8431f3b18f18bca1f7af5d"
TASK24_NOTES_SAVE_RETEST_INITRAMFS_SHA256 = "21a893a95ca267ba6f2fa2a31ad3c24e4cc85315786873e08623d327ad36829d"
TASK24_C3_RESTART_RETEST_HEAD = "b49781625b13e3eda50705c058c6206ae6582a51"
TASK24_C3_RESTART_RETEST_INITRAMFS_SHA256 = "7e8b24f188337302f362095d031c8c895e2590999474620b46e13334639e340a"
TASK24_F9_8E2A_HEAD = "8e2a5ff11034e0fdbd47cf81b3cdbfa8c281efa9"
TASK24_F9_8E2A_PID1_ELF_SHA256 = "4493bfec50a272f858a72261308eaddfbba5d7cbeaabee1da22abc7616aa0148"
TASK24_F9_8E2A_CPIO_SHA256 = "e0f8f0b960290ac064420ff81318077cacb45617c877c2047de5124afd0cedcb"
TASK24_F9_FIXED_INPUT_RECEIPT_SHA256 = "8bf0d9e1e3ca734f69e38a9353873808a4b299db0b3a010b138771dae73b633a"
TASK24_F9_CPIO_VALIDATION_RECEIPT_SHA256 = "89ba60b7bb52a9935cc073b0aacae8d8865af900aa9efa5112c387fce788aca4"
TASK24_F9_CPIO_VALIDATION_ELF_SIZE = 13_091_280
TASK24_F9_CANDIDATE_CHECK_SCHEMA = "nagi-task24-f9-candidate-check/v1"
ARTIFACT_NAMES = ("kernel", "initramfs", "system_image")
PHASES = ("save", "read")
L9_LOGGED_OUT = "NAGI_DEV_L9_LOGGED_OUT session="
L9_PREVIOUS_LOGOUT = "NAGI_DEV_L9_PREVIOUS_LOGOUT session="
L9_SESSION_ID = re.compile(r"session(?:_epoch\.(0|[1-9][0-9]{0,19}))?_[0-9a-f]{32}")
L9_FRAME_NAMES = ("japanese-unlock", "japanese-dark-desktop", "japanese-dark-settings")
MANUAL_TRIAL_PHASES = ("save", "reject-close", "read")
SERIAL_LIMIT_BYTES = 16 * 1024 * 1024
TERMINAL_MARKER = recovery.CURRENT_TERMINAL
REGISTRATION_WAIT = "NAGI_LINUX_OWNER_REGISTRATION_WAITING_FOR_INPUT"
RECEIPT_NAME = "task24-interactive-run.json"


class RunnerError(RuntimeError):
    """Inputs or observed run results do not satisfy the Task24 contract."""


@dataclass(frozen=True)
class PreparedRun:
    manifest_path: Path
    manifest_sha256: str
    preflight_script: Path
    preflight_script_sha256: str
    run_id: str
    run_directory: Path
    candidate_source_commit: str
    artifacts: dict[str, dict[str, Any]]
    data_path: Path
    data_uuid: str
    phases: dict[str, dict[str, Any]]
    preflight_result: dict[str, Any]
    l9_pin: tuple[str, str] | None = None
    f9_rfb_no_start: bool = False
    f9_pid1_elf_path: Path | None = None
    f9_pid1_elf_sha256: str | None = None
    f9_fixed_input_receipt_path: Path | None = None
    f9_fixed_input_receipt_sha256: str | None = None
    f9_cpio_validation_receipt_path: Path | None = None
    f9_cpio_validation_receipt_sha256: str | None = None


def _canonical_existing_file(value: str | Path, name: str, *, executable: bool = False) -> Path:
    path = Path(value)
    try:
        resolved = path.resolve(strict=True)
        original_stat = path.lstat()
    except OSError as error:
        raise RunnerError(f"{name} is unavailable ({error.__class__.__name__})") from None
    if path != resolved or stat.S_ISLNK(original_stat.st_mode) or not stat.S_ISREG(original_stat.st_mode):
        raise RunnerError(f"{name} must be a canonical regular file")
    if original_stat.st_size <= 0:
        raise RunnerError(f"{name} must not be empty")
    if executable and not os.access(path, os.X_OK):
        raise RunnerError(f"{name} is not executable")
    return path


def sha256_file(path: Path, name: str) -> tuple[str, tuple[int, ...]]:
    path = _canonical_existing_file(path, name)
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise RunnerError(f"{name} cannot be opened ({error.__class__.__name__})") from None
    digest = hashlib.sha256()
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            raise RunnerError(f"{name} must be a regular file")
        identity = _file_identity(before)
        while True:
            chunk = os.read(descriptor, 1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
        after = os.fstat(descriptor)
        if _file_identity(after) != identity:
            raise RunnerError(f"{name} changed while being hashed")
        try:
            current = path.lstat()
        except OSError as error:
            raise RunnerError(f"{name} path changed ({error.__class__.__name__})") from None
        if stat.S_ISLNK(current.st_mode) or _file_identity(current) != identity:
            raise RunnerError(f"{name} path no longer matches the opened file")
        return digest.hexdigest(), identity
    finally:
        os.close(descriptor)


def _file_identity(metadata: os.stat_result) -> tuple[int, ...]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_mode,
        metadata.st_nlink,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def _manifest_bytes(path: Path) -> bytes:
    path = _canonical_existing_file(path, "manifest")
    if path.stat().st_size > 1024 * 1024:
        raise RunnerError("manifest exceeds the 1 MiB limit")
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags)
    try:
        before = os.fstat(descriptor)
        payload = bytearray()
        while len(payload) <= 1024 * 1024:
            chunk = os.read(descriptor, min(65536, 1024 * 1024 + 1 - len(payload)))
            if not chunk:
                break
            payload.extend(chunk)
        after = os.fstat(descriptor)
        if _file_identity(before) != _file_identity(after):
            raise RunnerError("manifest changed while being read")
        if len(payload) > 1024 * 1024:
            raise RunnerError("manifest exceeds the 1 MiB limit")
        return bytes(payload)
    finally:
        os.close(descriptor)


def _json_mapping(payload: bytes, name: str) -> dict[str, Any]:
    try:
        value = json.loads(payload.decode("utf-8"))
    except (UnicodeError, json.JSONDecodeError):
        raise RunnerError(f"{name} is not valid UTF-8 JSON") from None
    if not isinstance(value, dict):
        raise RunnerError(f"{name} must be a JSON object")
    return value


def run_official_preflight(script: Path, manifest: Path) -> tuple[dict[str, Any], str, str]:
    script = _canonical_existing_file(script, "official preflight script")
    digest, _ = sha256_file(script, "official preflight script")
    if digest != TASK24_PREFLIGHT_SHA256:
        raise RunnerError("official preflight script does not match the fixed public HEAD pin")
    before_manifest, _ = sha256_file(manifest, "manifest")
    try:
        completed = subprocess.run(
            [
                sys.executable,
                str(script),
                "--manifest",
                str(manifest),
                "--preflight-only",
            ],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            timeout=600,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise RunnerError(f"official preflight could not complete ({error.__class__.__name__})") from None
    after_digest, _ = sha256_file(manifest, "manifest")
    after_script_digest, _ = sha256_file(script, "official preflight script")
    if before_manifest != after_digest:
        raise RunnerError("manifest changed while official preflight was running")
    if digest != after_script_digest:
        raise RunnerError("official preflight script changed while it was running")
    if completed.returncode != 0:
        detail = completed.stderr.strip().splitlines()
        reason = detail[-1] if detail else f"exit {completed.returncode}"
        raise RunnerError(f"official preflight rejected the manifest: {reason}")
    result = _json_mapping(completed.stdout.encode("utf-8"), "official preflight output")
    return result, digest, before_manifest


def prepare_run(
    manifest_path: Path, preflight_script: Path, *,
    notes_save_retest: bool = False, c3_restart_retest: bool = False,
    l9_pin: tuple[str, str] | None = None,
    f9_rfb_no_start: bool = False,
    f9_pid1_elf_path: Path | None = None,
    f9_fixed_input_receipt_path: Path | None = None,
    f9_cpio_validation_receipt_path: Path | None = None,
) -> PreparedRun:
    if sum((notes_save_retest, c3_restart_retest, l9_pin is not None, f9_rfb_no_start)) > 1:
        raise RunnerError("select only one Task24 retest")
    f9_input_paths = (
        f9_pid1_elf_path, f9_fixed_input_receipt_path, f9_cpio_validation_receipt_path,
    )
    if f9_rfb_no_start != all(path is not None for path in f9_input_paths):
        raise RunnerError("the F9 no-start profile requires its ELF and two fixed candidate receipts")
    if not f9_rfb_no_start and any(path is not None for path in f9_input_paths):
        raise RunnerError("F9 candidate inputs require the fixed F9 no-start profile")
    manifest_path = _canonical_existing_file(manifest_path, "manifest")
    preflight_script = _canonical_existing_file(preflight_script, "official preflight script")
    preflight_result, preflight_digest, preflight_manifest_digest = run_official_preflight(
        preflight_script, manifest_path
    )
    manifest_payload = _manifest_bytes(manifest_path)
    manifest_digest = hashlib.sha256(manifest_payload).hexdigest()
    if manifest_digest != preflight_manifest_digest:
        raise RunnerError("manifest changed after official preflight")
    manifest = _json_mapping(manifest_payload, "manifest")
    if manifest.get("schema") != TASK24_SCHEMA:
        raise RunnerError("manifest schema is unsupported")
    run_id = manifest["run_id"]
    if not isinstance(run_id, str) or re.fullmatch(r"task24-[a-z0-9][a-z0-9-]{0,48}", run_id) is None:
        raise RunnerError("manifest run_id is not canonical")
    run_directory = Path(manifest["run_directory"])
    if not run_directory.is_dir() or run_directory.name != run_id:
        raise RunnerError("run_directory must be an existing directory named for run_id")
    if manifest_path.parent != run_directory:
        raise RunnerError("manifest must reside directly in run_directory")
    source_commit = manifest["candidate_source_commit"]
    artifact_values = manifest["artifacts"]
    f9_elf_path = None
    f9_elf_sha256 = None
    f9_fixed_receipt_path = None
    f9_fixed_receipt_sha256 = None
    f9_validation_receipt_path = None
    f9_validation_receipt_sha256 = None
    if l9_pin is not None:
        head, initramfs = l9_pin
        if (re.fullmatch(r"[0-9a-f]{40}", head) is None
                or re.fullmatch(r"[0-9a-f]{64}", initramfs) is None):
            raise RunnerError("L9 requires a full source and initramfs pin")
        if (source_commit, artifact_values["initramfs"]["sha256"]) != l9_pin:
            raise RunnerError("the preflight manifest does not match the fixed L9 candidate")
    elif notes_save_retest:
        if source_commit != TASK24_NOTES_SAVE_RETEST_HEAD:
            raise RunnerError("manifest candidate source does not match the pinned Notes save retest")
        if artifact_values["initramfs"]["sha256"] != TASK24_NOTES_SAVE_RETEST_INITRAMFS_SHA256:
            raise RunnerError("manifest initramfs does not match the pinned Notes save retest")
    elif c3_restart_retest:
        if source_commit != TASK24_C3_RESTART_RETEST_HEAD:
            raise RunnerError("manifest candidate source does not match the pinned C3 restart retest")
        if artifact_values["initramfs"]["sha256"] != TASK24_C3_RESTART_RETEST_INITRAMFS_SHA256:
            raise RunnerError("manifest initramfs does not match the pinned C3 restart retest")
    elif f9_rfb_no_start:
        if source_commit != TASK24_F9_8E2A_HEAD:
            raise RunnerError("manifest candidate source does not match the pinned F9 8e2a profile")
        if artifact_values["initramfs"]["sha256"] != TASK24_F9_8E2A_CPIO_SHA256:
            raise RunnerError("manifest initramfs CPIO does not match the pinned F9 8e2a profile")
        assert all(path is not None for path in f9_input_paths)
        assert f9_fixed_input_receipt_path is not None and f9_cpio_validation_receipt_path is not None
        f9_elf_path = _canonical_existing_file(f9_pid1_elf_path, "F9 PID 1 ELF")
        f9_elf_sha256, _ = sha256_file(f9_elf_path, "F9 PID 1 ELF")
        if f9_elf_sha256 != TASK24_F9_8E2A_PID1_ELF_SHA256:
            raise RunnerError("F9 PID 1 ELF does not match the pinned 8e2a profile")
        f9_fixed_receipt_path = _canonical_existing_file(
            f9_fixed_input_receipt_path, "F9 fixed-input receipt",
        )
        f9_fixed_receipt_sha256, _ = sha256_file(f9_fixed_receipt_path, "F9 fixed-input receipt")
        if f9_fixed_receipt_sha256 != TASK24_F9_FIXED_INPUT_RECEIPT_SHA256:
            raise RunnerError("F9 fixed-input receipt does not match the pinned candidate")
        f9_validation_receipt_path = _canonical_existing_file(
            f9_cpio_validation_receipt_path, "F9 CPIO validation receipt",
        )
        f9_validation_receipt_sha256 = _verify_f9_cpio_validation_receipt(
            f9_validation_receipt_path,
            elf_sha256=f9_elf_sha256,
        )
    elif source_commit != TASK24_HEAD:
        raise RunnerError("manifest candidate source is not the fixed public Task24 HEAD")
    artifacts = {
        name: {"path": Path(artifact_values[name]["path"]), "sha256": artifact_values[name]["sha256"]}
        for name in ARTIFACT_NAMES
    }
    data_value = manifest["data_work_copy"]
    data_path = Path(data_value["path"])
    data_uuid = data_value["filesystem_uuid"]
    phases = {
        phase: {
            **manifest["phases"][phase],
            "data_image": Path(manifest["phases"][phase]["data_image"]),
            "serial_path": Path(manifest["phases"][phase]["serial_path"]),
        }
        for phase in PHASES
    }
    _validate_preflight_result(preflight_result, run_id, source_commit, artifacts, data_uuid, phases)
    for name, artifact in artifacts.items():
        actual, _ = sha256_file(artifact["path"], f"artifacts.{name}")
        if actual != artifact["sha256"]:
            raise RunnerError(f"artifacts.{name} changed after official preflight")
    _data_identity(data_path)
    return PreparedRun(
        manifest_path=manifest_path,
        manifest_sha256=manifest_digest,
        preflight_script=preflight_script,
        preflight_script_sha256=preflight_digest,
        run_id=run_id,
        run_directory=run_directory,
        candidate_source_commit=source_commit,
        artifacts=artifacts,
        data_path=data_path,
        data_uuid=data_uuid,
        phases=phases,
        preflight_result=preflight_result,
        l9_pin=l9_pin,
        f9_rfb_no_start=f9_rfb_no_start,
        f9_pid1_elf_path=f9_elf_path,
        f9_pid1_elf_sha256=f9_elf_sha256,
        f9_fixed_input_receipt_path=f9_fixed_receipt_path,
        f9_fixed_input_receipt_sha256=f9_fixed_receipt_sha256,
        f9_cpio_validation_receipt_path=f9_validation_receipt_path,
        f9_cpio_validation_receipt_sha256=f9_validation_receipt_sha256,
    )


def _verify_f9_cpio_validation_receipt(path: Path, *, elf_sha256: str) -> str:
    receipt_digest, identity = sha256_file(path, "F9 CPIO validation receipt")
    if receipt_digest != TASK24_F9_CPIO_VALIDATION_RECEIPT_SHA256:
        raise RunnerError("F9 CPIO validation receipt does not match the pinned candidate")
    if identity[4] > 1024 * 1024:
        raise RunnerError("F9 CPIO validation receipt exceeds the size limit")
    payload = path.read_bytes()
    if hashlib.sha256(payload).hexdigest() != receipt_digest:
        raise RunnerError("F9 CPIO validation receipt changed while being read")
    receipt = _json_mapping(payload, "F9 CPIO validation receipt")
    inputs = receipt.get("development_inputs")
    if not isinstance(inputs, list):
        validation = receipt.get("validation")
        inputs = validation.get("development_inputs") if isinstance(validation, dict) else None
    if not isinstance(inputs, list):
        raise RunnerError("F9 CPIO validation receipt has no development input list")
    elf_record = next((item for item in inputs if isinstance(item, dict) and item.get("path") == "nagi-init"), None)
    if (elf_record is None or elf_record.get("sha256") != elf_sha256
            or elf_record.get("size_bytes") != TASK24_F9_CPIO_VALIDATION_ELF_SIZE):
        raise RunnerError("F9 CPIO validation receipt does not bind the pinned nagi-init ELF")
    return receipt_digest


def _read_verified_candidate_bytes(path: Path, expected_sha256: str, name: str) -> bytes:
    digest, identity = sha256_file(path, name)
    if digest != expected_sha256:
        raise RunnerError(f"{name} does not match its fixed candidate pin")
    if identity[4] > 128 * 1024 * 1024:
        raise RunnerError(f"{name} exceeds the candidate input size limit")
    payload = path.read_bytes()
    if hashlib.sha256(payload).hexdigest() != expected_sha256:
        raise RunnerError(f"{name} changed while being read")
    after_digest, after_identity = sha256_file(path, name)
    if after_digest != expected_sha256 or after_identity != identity:
        raise RunnerError(f"{name} changed during candidate validation")
    return payload


def f9_candidate_check_receipt(prepared: PreparedRun) -> dict[str, Any]:
    """Bind the fixed source, separate PID 1 ELF, and preflight-pinned CPIO."""
    if not prepared.f9_rfb_no_start:
        raise RunnerError("F9 candidate receipt requires the fixed F9 no-start profile")
    if (prepared.f9_pid1_elf_path is None or prepared.f9_pid1_elf_sha256 is None
            or prepared.f9_fixed_input_receipt_path is None
            or prepared.f9_fixed_input_receipt_sha256 is None
            or prepared.f9_cpio_validation_receipt_path is None
            or prepared.f9_cpio_validation_receipt_sha256 is None):
        raise RunnerError("F9 candidate inputs are incomplete")
    _verify_fixed_inputs(prepared)
    elf_payload = _read_verified_candidate_bytes(
        prepared.f9_pid1_elf_path,
        TASK24_F9_8E2A_PID1_ELF_SHA256,
        "F9 PID 1 ELF",
    )
    cpio_artifact = prepared.artifacts["initramfs"]
    cpio_payload = _read_verified_candidate_bytes(
        cpio_artifact["path"],
        TASK24_F9_8E2A_CPIO_SHA256,
        "F9 Task24 CPIO",
    )
    try:
        embedded = boot_acceptance.validate_development_cpio_inputs(
            cpio_payload,
            {"system/nagi-init": (0o755, elf_payload)},
        )
    except boot_acceptance.AcceptanceError as error:
        raise RunnerError(f"F9 CPIO does not contain the pinned PID 1 ELF ({error})") from None
    entry = embedded["development_inputs"][0]
    if (embedded.get("development_input_bytes_match") != "PASS"
            or entry.get("sha256") != TASK24_F9_8E2A_PID1_ELF_SHA256):
        raise RunnerError("F9 CPIO embedded PID 1 ELF does not match its fixed input")
    return {
        "schema": TASK24_F9_CANDIDATE_CHECK_SCHEMA,
        "profile": "F9_8E2A_NO_START_RFB",
        "status": "PASS",
        "source": {
            "manifest_pin": TASK24_F9_8E2A_HEAD,
            "candidate_source_commit": prepared.candidate_source_commit,
            "matches_fixed_pin": prepared.candidate_source_commit == TASK24_F9_8E2A_HEAD,
        },
        "pid1_elf": {
            "input": "--pid1-elf",
            "sha256": prepared.f9_pid1_elf_sha256,
            "fixed_sha256": TASK24_F9_8E2A_PID1_ELF_SHA256,
            "matches_fixed_candidate": prepared.f9_pid1_elf_sha256 == TASK24_F9_8E2A_PID1_ELF_SHA256,
        },
        "initramfs_cpio": {
            "manifest_artifact": "initramfs",
            "manifest_sha256": cpio_artifact["sha256"],
            "fixed_sha256": TASK24_F9_8E2A_CPIO_SHA256,
            "matches_fixed_candidate": cpio_artifact["sha256"] == TASK24_F9_8E2A_CPIO_SHA256,
        },
        "fixed_input_receipt": {
            "input": "--f9-fixed-input-receipt",
            "sha256": prepared.f9_fixed_input_receipt_sha256,
            "matches_fixed_candidate": prepared.f9_fixed_input_receipt_sha256
            == TASK24_F9_FIXED_INPUT_RECEIPT_SHA256,
        },
        "cpio_validation_receipt": {
            "input": "--f9-cpio-validation-receipt",
            "sha256": prepared.f9_cpio_validation_receipt_sha256,
            "development_input_path": "nagi-init",
            "development_input_sha256": TASK24_F9_8E2A_PID1_ELF_SHA256,
            "development_input_size_bytes": TASK24_F9_CPIO_VALIDATION_ELF_SIZE,
            "matches_fixed_pid1_elf": True,
        },
        "cpio_embedded_pid1": {
            "path": "system/nagi-init",
            "sha256": entry["sha256"],
            "matches_fixed_pid1_elf": entry["sha256"] == TASK24_F9_8E2A_PID1_ELF_SHA256,
            "validator": "boot_acceptance.validate_development_cpio_inputs",
            "cpio_structure": embedded["cpio_structure"],
            "development_input_bytes_match": embedded["development_input_bytes_match"],
        },
        "qemu_started": False,
        "launch_authorized": False,
    }


def prepare_trial_layout(prepared: PreparedRun, phases: tuple[str, ...]) -> dict[str, Any]:
    """Validate private phase destinations against the pinned Data copy without launching."""
    _verify_fixed_inputs(prepared)
    initial_data = _data_identity(prepared.data_path)
    data_uuid = _read_data_superblock_uuid(prepared.data_path)
    if data_uuid != prepared.data_uuid:
        raise RunnerError("Data superblock UUID differs from the preflight manifest")
    root = prepared.run_directory
    if root.resolve(strict=True) != root:
        raise RunnerError("three-phase trial directory traverses a symlink")
    root_device = root.stat().st_dev
    identities = set()
    serials = {}
    for phase in phases:
        directory = root / phase
        serial = directory / "serial.log"
        if phase in prepared.phases and prepared.phases[phase]["serial_path"] != serial:
            raise RunnerError(f"{phase} serial destination differs from public preflight")
        metadata = directory.lstat()
        identity = metadata.st_dev, metadata.st_ino
        if (
            not stat.S_ISDIR(metadata.st_mode)
            or metadata.st_dev != root_device
            or identity in identities
        ):
            raise RunnerError(f"{phase} must have its own real run directory")
        identities.add(identity)
        outputs = [serial, directory / "qemu.log", directory / "supervisor.log"]
        if phase == "locale-read":
            outputs += [directory / f"{name}.ppm" for name in L9_FRAME_NAMES]
        for output in outputs:
            if output.exists() or output.is_symlink():
                raise RunnerError(f"{phase} already has a run output")
        serials[phase] = serial.as_posix()
    if _data_identity(prepared.data_path) != initial_data:
        raise RunnerError("Data backing file changed during trial preparation")
    return {
        "status": "THREE_PHASE_LAYOUT_VERIFIED" if phases == MANUAL_TRIAL_PHASES else "L9_LAYOUT_VERIFIED",
        "phases": list(phases),
        "serial_destinations": serials,
        "data_filesystem_uuid": data_uuid,
        "data_uuid_verified_in_superblock": True,
        "data_serial": f"NAGI-DATA-{prepared.data_uuid[:8]}",
        "data_identity": {"device": initial_data[0], "inode": initial_data[1]},
        "data_freshness_evidence": "MANIFEST_DECLARED_ONLY",
        "launch_authorized": False,
        "guest_poweroff": "UNVERIFIED",
        "host_stop_policy": "OWNED_CHILD_AFTER_VALIDATED_TERMINAL",
    }


def prepare_three_phase_trial(prepared: PreparedRun) -> dict[str, Any]:
    return prepare_trial_layout(prepared, MANUAL_TRIAL_PHASES)


def prepare_manual_trial(prepared: PreparedRun) -> dict[str, Any]:
    """Preserve the manual-trial preflight interface."""
    return prepare_three_phase_trial(prepared)


def _require_c3_restart_candidate(prepared: PreparedRun) -> None:
    if prepared.candidate_source_commit != TASK24_C3_RESTART_RETEST_HEAD:
        raise RunnerError("automatic development fixture requires the pinned C3 restart retest candidate")
    if prepared.artifacts["initramfs"]["sha256"] != TASK24_C3_RESTART_RETEST_INITRAMFS_SHA256:
        raise RunnerError("automatic development fixture requires the pinned C3 restart retest initramfs")


def prepare_automatic_trial(
    prepared: PreparedRun, fixture_path: Path, *, l9_trial: bool = False,
) -> dict[str, Any]:
    """Validate the phase layout and private fixture without launching QEMU."""
    if l9_trial != (prepared.l9_pin is not None):
        raise RunnerError("L9 trial must use its pinned preflight candidate")
    if prepared.f9_rfb_no_start:
        if l9_trial:
            raise RunnerError("F9 no-start profile cannot use the L9 fixture flow")
    elif not l9_trial:
        _require_c3_restart_candidate(prepared)
    fixture = load_development_fixture(
        fixture_path,
        run_directory=prepared.run_directory,
        run_id=prepared.run_id,
        preflight_manifest_sha256=prepared.manifest_sha256,
    )
    result = {
        **prepare_trial_layout(prepared, tuple(L9_RFB_FLOW) if l9_trial else MANUAL_TRIAL_PHASES),
        "mode": (
            "AUTOMATIC_L9_FIXTURE" if l9_trial else
            "AUTOMATIC_F9_8E2A_NO_START_FIXTURE" if prepared.f9_rfb_no_start else
            "AUTOMATIC_DEVELOPMENT_FIXTURE"
        ),
        "fixture_manifest_sha256": fixture.sha256,
        "fixture_secrets_recorded": False,
        "rfb_endpoint": f"127.0.0.1:{fixture.rfb_port}",
        "launch_authorized": False,
        "post_action_display_content": "NOT_VERIFIED",
    }
    if prepared.f9_rfb_no_start:
        result["coverage"] = "Task24 save/reject-close/read fixture preparation only; no WS-10 RFB evidence"
    return result


def _read_data_superblock_uuid(path: Path) -> str:
    # Linux ext filesystems keep the first superblock at byte 1024. This check
    # binds the manifest UUID to disk bytes. It does not replace e2fsck or a
    # provisioning receipt for a fresh Task24 volume.
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    with os.fdopen(os.open(path, flags), "rb") as source:
        before = os.fstat(source.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1:
            raise RunnerError("Data superblock input must be a single-link regular file")
        source.seek(1024)
        block = source.read(1024)
        after = os.fstat(source.fileno())
    if _file_identity(before) != _file_identity(after) or _file_identity(after) != _file_identity(path.lstat()):
        raise RunnerError("Data image changed while reading its superblock")
    if len(block) != 1024 or block[0x38:0x3A] != b"\x53\xef":
        raise RunnerError("Data image has no ext filesystem superblock")
    return str(uuid.UUID(bytes=block[0x68:0x78]))


def _validate_preflight_result(
    result: dict[str, Any],
    run_id: str,
    source_commit: str,
    artifacts: dict[str, dict[str, Any]],
    data_uuid: str,
    phases: dict[str, dict[str, Any]],
) -> None:
    expected = {
        "preflight": "PASS",
        "run_id": run_id,
        "candidate_source_commit": source_commit,
        "same_system_candidate": True,
        "same_data_work_copy": True,
        "data_freshness": "manifest-declared fresh-ext4",
        "data_filesystem_uuid": data_uuid,
        "serial_files_separate": True,
        "phase_timeouts_seconds": {
            phase: phases[phase]["timeout_seconds"] for phase in PHASES
        },
        "qmp_enabled": False,
        "interactive_start_authorized": False,
        "qemu_started": False,
    }
    for key, value in expected.items():
        if result.get(key) != value:
            raise RunnerError(f"official preflight result has an unexpected {key}")
    for name in ARTIFACT_NAMES:
        if result.get(f"{name if name != 'system_image' else 'system'}_sha256") != artifacts[name]["sha256"]:
            raise RunnerError(f"official preflight result has an unexpected {name} digest")


def _data_identity(path: Path) -> tuple[int, int]:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise RunnerError(f"Data work copy is unavailable ({error.__class__.__name__})") from None
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise RunnerError("Data work copy must remain the same regular file")
    return metadata.st_dev, metadata.st_ino


def _verify_fixed_inputs(prepared: PreparedRun) -> None:
    manifest_digest, _ = sha256_file(prepared.manifest_path, "manifest")
    if manifest_digest != prepared.manifest_sha256:
        raise RunnerError("manifest changed after official preflight")
    preflight_digest, _ = sha256_file(prepared.preflight_script, "official preflight script")
    if preflight_digest != prepared.preflight_script_sha256:
        raise RunnerError("official preflight script changed after validation")
    for name, artifact in prepared.artifacts.items():
        actual, _ = sha256_file(artifact["path"], f"artifacts.{name}")
        if actual != artifact["sha256"]:
            raise RunnerError(f"artifacts.{name} changed after official preflight")
    if prepared.f9_rfb_no_start:
        if (prepared.f9_pid1_elf_path is None or prepared.f9_pid1_elf_sha256 is None
                or prepared.f9_fixed_input_receipt_path is None
                or prepared.f9_fixed_input_receipt_sha256 is None
                or prepared.f9_cpio_validation_receipt_path is None
                or prepared.f9_cpio_validation_receipt_sha256 is None):
            raise RunnerError("F9 candidate inputs are incomplete")
        actual_elf, _ = sha256_file(prepared.f9_pid1_elf_path, "F9 PID 1 ELF")
        if actual_elf != prepared.f9_pid1_elf_sha256:
            raise RunnerError("F9 PID 1 ELF changed after candidate validation")
        actual_fixed_receipt, _ = sha256_file(
            prepared.f9_fixed_input_receipt_path, "F9 fixed-input receipt",
        )
        if actual_fixed_receipt != prepared.f9_fixed_input_receipt_sha256:
            raise RunnerError("F9 fixed-input receipt changed after candidate validation")
        actual_validation_receipt = _verify_f9_cpio_validation_receipt(
            prepared.f9_cpio_validation_receipt_path,
            elf_sha256=prepared.f9_pid1_elf_sha256,
        )
        if actual_validation_receipt != prepared.f9_cpio_validation_receipt_sha256:
            raise RunnerError("F9 CPIO validation receipt changed after candidate validation")


def validate_start_gate(
    slot: int | None, board_readback_sha256: str | None, *, l9_trial: bool = False,
) -> None:
    if l9_trial and slot not in (1, 2):
        raise RunnerError("L9 requires a board-reserved slot")
    if not l9_trial and slot != 2:
        raise RunnerError("Task24 public-head acceptance requires its board-reserved slot 2")
    if not isinstance(board_readback_sha256, str) or re.fullmatch(r"[0-9a-f]{64}", board_readback_sha256) is None:
        raise RunnerError("a fresh board readback SHA-256 is required before interactive start")


def qemu_argv(
    prepared: PreparedRun,
    phase: str,
    qemu: Path,
    *,
    rfb_port: int | None = None,
    previous_l9_session: str | None = None,
) -> list[str]:
    if phase not in (*MANUAL_TRIAL_PHASES, *L9_RFB_FLOW) or phase not in prepared.phases:
        raise RunnerError("unsupported Task24 phase")
    if rfb_port is not None and (
        isinstance(rfb_port, bool) or not isinstance(rfb_port, int) or not 5901 <= rfb_port <= 65535
    ):
        raise RunnerError("automatic Task24 RFB port is outside the supported range")
    phase_data = prepared.phases[phase]
    serial_path: Path = phase_data["serial_path"]
    paths = [
        qemu.as_posix(),
        prepared.artifacts["kernel"]["path"].as_posix(),
        prepared.artifacts["initramfs"]["path"].as_posix(),
        prepared.artifacts["system_image"]["path"].as_posix(),
        prepared.data_path.as_posix(),
        serial_path.as_posix(),
    ]
    if any("," in value or "\n" in value or "\r" in value for value in paths):
        raise RunnerError("QEMU paths cannot contain commas or line breaks")
    if re.fullmatch(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}", prepared.data_uuid) is None:
        raise RunnerError("QEMU Data filesystem UUID must be canonical")
    data_serial = f"NAGI-DATA-{prepared.data_uuid[:8]}"
    kernel_cmdline = (
        "console=ttyS0,115200n8 rdinit=/init "
        f"nagi.dev_data.boot=identity nagi.dev_data.serial={data_serial} "
        f"nagi.dev_data.fs_uuid={prepared.data_uuid} "
        "nagi.dev_data.accounts=accounts nagi.dev_data.snapshots=snapshots"
    )
    if previous_l9_session is not None:
        if phase not in L9_RFB_FLOW or phase == "locale-save" or not valid_l9_session_id(previous_l9_session):
            raise RunnerError("invalid L9 predecessor for QEMU")
        kernel_cmdline += f" nagi.dev_l9_previous_session={previous_l9_session}"
    return [
        qemu.as_posix(),
        "-name",
        f"nagi-task24-{prepared.run_id}-{phase}",
        "-machine",
        "q35,i8042=off",
        "-accel",
        "tcg,thread=multi",
        "-cpu",
        "max",
        "-smp",
        "4",
        "-m",
        "8192",
        "-kernel",
        prepared.artifacts["kernel"]["path"].as_posix(),
        "-initrd",
        prepared.artifacts["initramfs"]["path"].as_posix(),
        "-append",
        kernel_cmdline,
        "-drive",
        f"if=none,file={prepared.artifacts['system_image']['path']},format=raw,readonly=on,id=systemdisk",
        "-device",
        "virtio-blk-pci,drive=systemdisk",
        "-drive",
        f"if=none,file={prepared.data_path},format=raw,id=datadisk",
        "-device",
        f"virtio-blk-pci,drive=datadisk,serial={data_serial}",
        "-vga",
        "none",
        "-device",
        "virtio-vga,xres=640,yres=480,edid=on",
        "-device",
        "virtio-keyboard-pci",
        "-display",
        "cocoa" if rfb_port is None else f"vnc=127.0.0.1:{rfb_port - 5900}",
        "-nic",
        "none",
        "-serial",
        f"file:{serial_path}",
        "-monitor",
        "none",
        "-no-reboot",
    ]


def _read_serial(path: Path) -> str:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return ""
    except OSError as error:
        raise RunnerError(f"serial output cannot be inspected ({error.__class__.__name__})") from None
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise RunnerError("serial output must be a regular file")
    if metadata.st_size > SERIAL_LIMIT_BYTES:
        raise RunnerError("serial output exceeds the 16 MiB limit")
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except OSError as error:
        raise RunnerError(f"serial output cannot be read ({error.__class__.__name__})") from None


def _guest_failure(serial_text: str) -> str | None:
    for line in serial_text.splitlines():
        if line.startswith("Kernel panic - not syncing:"):
            return "kernel panic marker"
        if line.startswith("nagi-init: startup failed"):
            return "nagi-init startup failure marker"
        if line.startswith("NAGI_NOTES_DESKTOP_FAIL "):
            return "Notes failure marker"
        if line.startswith("nagi-init: lifecycle failure ("):
            return "nagi-init lifecycle failure marker"
    return None


def _automatic_failure_marker(serial_text: str) -> bool:
    complete = serial_text
    if not complete.endswith("\n"):
        complete = complete[:complete.rfind("\n") + 1]
    return any(line.startswith(recovery.FAILURES) for line in complete.splitlines())


def _input_ready(serial_text: str, phase: str) -> bool:
    markers = {recovery.LOGIN_WAIT}
    if phase == "save":
        markers.add(REGISTRATION_WAIT)
    return any(line in markers for line in serial_text.splitlines())


def _phase_success(serial_text: str, phase: str) -> dict[str, int]:
    marker = recovery.SAVED if phase == "save" else recovery.REOPENED
    try:
        recovery._validate_phase(serial_text, phase, marker)
    except ValueError as error:
        raise RunnerError(str(error)) from None
    return {
        "input_ready_count": serial_text.splitlines().count(recovery.INPUT),
        "identity_ready_count": 1,
        "pid1_ready_count": 1,
        "notes_pass_count": 1,
        "terminal_consumed_count": 1,
    }


def valid_l9_session_id(value: str) -> bool:
    # The Linux issuer uses a random 128-bit nonce. Identity snapshots can
    # namespace that nonce with the canonical u64 store generation.
    matched = L9_SESSION_ID.fullmatch(value)
    if matched is None:
        return False
    generation = matched.group(1)
    return generation is None or int(generation) <= 2**64 - 1


def l9_logout_session(lines: list[str], marker: str, phase: str) -> tuple[str, int]:
    index = recovery._unique_index(lines, marker, phase, prefix=True)
    session = lines[index][len(marker):]
    if not valid_l9_session_id(session):
        raise ValueError(f"{phase} has a malformed L9 logout Session")
    return session, index


def validate_l9_phase(
    serial: str, phase: str, previous_serial: str | None,
) -> dict[str, int]:
    lines = recovery._complete_lines(serial, phase)
    if tuple(l9_flow_markers(lines, phase)) != L9_RFB_FLOW[phase]:
        raise ValueError(f"{phase} did not finish the ordered L9 input flow")
    mount, session, handshake, ready, desktop, logout, ended, identity_exit = (
        recovery._unique_index(lines, marker, phase)
        for marker in (
            recovery.MOUNT, recovery.SESSION, recovery.HANDSHAKE, recovery.READY,
            recovery.DESKTOP_READY, recovery.DESKTOP_LOGOUT, recovery.SESSION_LOGOUT,
            recovery.IDENTITY_EXIT_REQUESTED,
        )
    )
    reaped = recovery._unique_normal_reap_index(lines, phase)
    c3, c3_index = recovery._receipt_identity(lines, recovery.C3_IDENTITY_NORMAL_EXIT_REAPED, phase)
    terminal, terminal_index = recovery._receipt_identity(lines, recovery.CURRENT_TERMINAL, phase)
    current_session, current_logout = l9_logout_session(lines, L9_LOGGED_OUT, phase)
    if c3 != terminal or not (
        mount < session < handshake < ready < desktop < logout < ended < current_logout < reaped
        < identity_exit < c3_index < terminal_index
    ):
        raise ValueError(f"{phase} Session/READY/logout/terminal evidence differs")
    if (previous_serial is None) != (phase == "locale-save"):
        raise ValueError(f"{phase} requires the expected preceding L9 boot")
    if previous_serial is not None:
        prior = recovery._complete_lines(previous_serial, "preceding L9 boot")
        expected, prior_terminal = recovery._receipt_identity(prior, recovery.CURRENT_TERMINAL, phase)
        expected_session, prior_logout = l9_logout_session(prior, L9_LOGGED_OUT, phase)
        consumed, consumed_index = recovery._receipt_identity(lines, recovery.PREVIOUS_TERMINAL, phase)
        prior_session, previous_logout = l9_logout_session(lines, L9_PREVIOUS_LOGOUT, phase)
        first_unlock = lines.index(recovery.LOGIN_WAIT)
        if (
            consumed != expected or prior_session != expected_session or prior_session == current_session
            or prior_logout >= prior_terminal
            or not mount < consumed_index < previous_logout < first_unlock < session
        ):
            raise ValueError(f"{phase} did not consume the preceding L9 logout before unlock")
    if phase == "password-login":
        rejected = recovery._unique_index(lines, recovery.LOGIN_REJECTED, phase)
        unlocks = [i for i, line in enumerate(lines) if line == recovery.LOGIN_WAIT]
        if not unlocks[0] < rejected < unlocks[1] < session:
            raise ValueError("L9 old credential was not rejected before the new credential attached")
    return {
        "identity_ready_count": 1,
        "pid1_ready_count": 1,
        "terminal_consumed_count": 1,
        "confirmed_rejection_count": int(phase == "password-login"),
    }


def guest_memory_summary(serial_text: str) -> dict[str, Any]:
    samples: list[tuple[int, int, int, int]] = []
    for line in serial_text.splitlines():
        if "NAGI_A_GUEST_MEM" not in line:
            continue
        total_match = re.search(r"\bmem_total_kib=(\d+)\b", line)
        available_match = re.search(r"\bmem_available_kib=(\d+)\b", line)
        time_match = re.search(r"\bt_ms=(\d+)\b", line)
        if total_match is None or available_match is None or time_match is None:
            raise RunnerError("guest memory telemetry line is missing its KiB or guest-clock fields")
        total_kib = int(total_match.group(1))
        available_kib = int(available_match.group(1))
        guest_time_ms = int(time_match.group(1))
        if total_kib <= 0 or available_kib < 0 or available_kib > total_kib:
            raise RunnerError("guest memory telemetry values are outside their valid range")
        samples.append((guest_time_ms, total_kib, available_kib, total_kib - available_kib))
    if not samples:
        return {
            "status": "NOT_AVAILABLE",
            "source": "NAGI_A_GUEST_MEM",
            "samples": 0,
            "unit": "KiB",
            "max_sampled_mem_total_minus_available_kib": None,
            "absolute_peak_claimed": False,
            "terminal_sample_guaranteed": False,
            "clock_domain": "guest Instant; no host alignment claimed",
        }
    return {
        "status": "SAMPLED",
        "source": "NAGI_A_GUEST_MEM",
        "samples": len(samples),
        "unit": "KiB",
        "max_sampled_mem_total_minus_available_kib": max(item[3] for item in samples),
        "absolute_peak_claimed": False,
        "terminal_sample_guaranteed": False,
        "mem_total_kib_values": sorted({item[1] for item in samples}),
        "first_guest_t_ms": samples[0][0],
        "last_guest_t_ms": samples[-1][0],
        "clock_domain": "guest Instant; no host alignment claimed",
    }


def request_visible_confirmation(run_id: str, phase: str, deadline: float) -> dict[str, Any]:
    expected = f"VISIBLE {run_id} {phase}"
    print(
        f"QEMU Cocoa window is open. Verify the guest frame is visible, then enter {expected!r} here. "
        "Click the guest window and perform the planned UI action; this runner sends no guest keys.",
        flush=True,
    )
    if phase == "reject-close":
        print("Submit one intentionally incorrect attempt. Wait for retry confirmation before pressing Escape.", flush=True)
    selector = selectors.DefaultSelector()
    try:
        try:
            selector.register(sys.stdin, selectors.EVENT_READ)
        except (OSError, ValueError) as error:
            raise RunnerError("interactive start requires a readable Terminal input") from error
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise RunnerError(f"{phase} visible-frame confirmation timed out")
            events = selector.select(timeout=min(0.25, remaining))
            if not events:
                continue
            line = sys.stdin.readline()
            if not line:
                raise RunnerError("Terminal input closed before frame visibility confirmation")
            if line.rstrip("\r\n") != expected:
                raise RunnerError("frame confirmation did not match the run and phase")
            return {
                "operator_confirmed_visible_frame": True,
                "confirmed_at_utc": datetime.now(timezone.utc).isoformat(),
            }
    finally:
        selector.close()


def _supervisor_summary(path: Path) -> dict[str, Any]:
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError as error:
        raise RunnerError(f"QEMU supervisor output is unavailable ({error.__class__.__name__})") from None
    summaries: list[dict[str, Any]] = []
    for line in lines:
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and "qemu_exit" in value:
            summaries.append(value)
    if len(summaries) != 1:
        raise RunnerError("QEMU supervisor must record exactly one child exit result")
    if sum(line.startswith("qemu_child_started pid=") for line in lines) != 1:
        raise RunnerError("QEMU supervisor must record exactly one owned child start")
    return summaries[0]


def _shutdown_observation(serial: str, summary: dict[str, Any]) -> str:
    if (
        summary.get("timed_out") is True
        or summary.get("cancelled") is True
        or summary.get("stopped_on_marker") is True
    ):
        return "HOST_RECOVERY"
    lines = serial.splitlines()
    powerdown_lines = [
        index for index, line in enumerate(lines)
        if re.fullmatch(r"(?:\[\s*\d+\.\d+\]\s*)?reboot: Power down", line)
    ]
    terminal_lines = [
        index for index, line in enumerate(lines) if line.startswith(TERMINAL_MARKER + " ")
    ]
    if (
        type(summary.get("qemu_exit")) is int
        and summary["qemu_exit"] == 0
        and summary.get("timed_out") is False
        and summary.get("cancelled") is False
        and summary.get("stopped_on_marker") is False
        and summary.get("wait_for_natural_exit") is True
        and len(powerdown_lines) == len(terminal_lines) == 1
        and terminal_lines[0] < powerdown_lines[0]
    ):
        return "GUEST_POWERDOWN_AND_NATURAL_EXIT"
    return "UNVERIFIED"


def observe_completed_pair(run_directory: Path, *, include_rejection: bool = False) -> dict[str, Any]:
    """Read recorded logs without touching Data or launching a process."""
    phases = MANUAL_TRIAL_PHASES if include_rejection else PHASES
    serials = {
        phase: _read_serial(run_directory / phase / "serial.log")
        for phase in phases
    }
    try:
        if include_rejection:
            validation = recovery.validate_save_reject_read(
                serials["save"], serials["reject-close"], serials["read"],
            )
        else:
            validation = recovery.validate_save_read(serials["save"], serials["read"])
    except ValueError as error:
        raise RunnerError(f"Task24 recovery serial validation failed: {error}") from None
    shutdown = {
        phase: _shutdown_observation(
            serials[phase], _supervisor_summary(run_directory / phase / "supervisor.log")
        )
        for phase in phases
    }
    result = {
        "recovery_serial": "VALIDATED",
        "ready_count_per_boot": validation["ready_count"],
        "shutdown": shutdown,
        "acceptance_status": "NOT_ESTABLISHED",
    }
    if include_rejection:
        result["reject_ready_count"] = validation["reject_ready_count"]
    return result


def _terminate_supervisor(process: subprocess.Popen[str]) -> None:
    if process.poll() is None:
        process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def stop_owned_after_terminal(
    process: subprocess.Popen[str],
    serial_text: str,
    validate_terminal: Callable[[str], Any],
    deadline: float,
) -> bool:
    """Request one normal host stop after the caller's full guest proof passes."""
    if time.monotonic() >= deadline or process.poll() is not None:
        return False
    validate_terminal(serial_text)
    if time.monotonic() >= deadline or process.poll() is not None:
        return False
    process.send_signal(signal.SIGTERM)
    return True


def validated_host_stop_result(
    requested: bool, return_code: int, summary: dict[str, Any]
) -> bool:
    # The supervisor exits 128 + SIGTERM following its owned child's reap.
    # An escalated kill reports -SIGKILL as the QEMU child exit.
    child_exit = summary.get("qemu_exit")
    return (
        requested
        and return_code == 128 + signal.SIGTERM
        and type(child_exit) is int
        and child_exit in (0, -signal.SIGTERM)
        and summary.get("cancelled") is True
        and summary.get("cancel_signal") == signal.SIGTERM
        and summary.get("timed_out") is False
        and summary.get("stopped_on_marker") is False
        and summary.get("wait_for_natural_exit") is True
    )


def run_phase(
    prepared: PreparedRun,
    phase: str,
    qemu: Path,
    confirm_visible: Callable[[str, str, float], dict[str, Any]] | None = None,
    *,
    trial_validation: Callable[[str], dict[str, int]] | None = None,
    automatic_fixture: DevelopmentFixture | None = None,
    previous_l9_session: str | None = None,
) -> dict[str, Any]:
    _verify_fixed_inputs(prepared)
    data_before = _data_identity(prepared.data_path)
    if data_before != _data_identity(prepared.data_path):
        raise RunnerError("Data work copy changed before QEMU launch")
    phase_data = prepared.phases[phase]
    serial_path: Path = phase_data["serial_path"]
    qemu_log = prepared.run_directory / phase / "qemu.log"
    supervisor_log = prepared.run_directory / phase / "supervisor.log"
    for path in (serial_path, qemu_log, supervisor_log):
        if path.exists() or path.is_symlink():
            raise RunnerError(f"refusing to reuse {phase} output")

    timeout = phase_data["timeout_seconds"]
    if phase in L9_RFB_FLOW:
        if prepared.l9_pin is None or automatic_fixture is None:
            raise RunnerError("L9 phase requires the pinned automatic trial")
        if phase != "locale-save" and previous_l9_session is None:
            raise RunnerError("L9 phase requires the preceding boot's logout Session")
    if automatic_fixture is None and confirm_visible is None:
        raise RunnerError("manual Task24 input requires visible-frame confirmation")
    if automatic_fixture is not None and phase not in (*MANUAL_TRIAL_PHASES, *L9_RFB_FLOW):
        raise RunnerError("automatic Task24 input requires a recognized trial")
    frames = {}
    def capture_frame(name: str, frame: bytes) -> None:
        if phase != "locale-read" or name not in L9_FRAME_NAMES or name in frames:
            raise RunnerError("unexpected L9 frame evidence")
        path = prepared.run_directory / phase / f"{name}.ppm"
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as output:
            output.write(frame)
        frames[name] = {"path": path.as_posix(), "sha256": hashlib.sha256(frame).hexdigest()}

    automatic_input = (
        AutomaticRfbInput(automatic_fixture, phase, capture_frame)
        if automatic_fixture is not None else None
    )
    argv = qemu_argv(
        prepared,
        phase,
        qemu,
        rfb_port=automatic_fixture.rfb_port if automatic_fixture is not None else None,
        previous_l9_session=previous_l9_session,
    )
    supervisor_argv = [
        sys.executable,
        "-B",
        str(QEMU_CHILD_RUNNER),
        "--timeout-seconds",
        str(timeout),
        "--shutdown-mode",
        "serial-marker",
        "--serial-log",
        str(serial_path),
        "--success-marker",
        TERMINAL_MARKER,
        "--task24-terminal-receipt",
        "--wait-for-natural-exit",
        "--qemu-log",
        str(qemu_log),
        "--",
        *argv,
    ]
    started_monotonic_ns = time.monotonic_ns()
    started_utc = datetime.now(timezone.utc).isoformat()
    visible_confirmation: dict[str, Any] | None = None
    host_stop_requested = False
    retry_announced = False
    phase_deadline = time.monotonic() + timeout
    supervisor_deadline = phase_deadline + 15
    try:
        with supervisor_log.open("xb") as supervisor_output:
            try:
                process = subprocess.Popen(
                    supervisor_argv,
                    stdin=subprocess.DEVNULL,
                    stdout=supervisor_output,
                    stderr=subprocess.STDOUT,
                    text=True,
                    bufsize=1,
                    cwd=REPO_ROOT,
                    start_new_session=True,
                )
            except OSError as error:
                raise RunnerError(f"QEMU supervisor could not start ({error.__class__.__name__})") from None
            try:
                while process.poll() is None:
                    serial_text = _read_serial(serial_path)
                    failure = _guest_failure(serial_text)
                    if failure is not None:
                        _terminate_supervisor(process)
                        raise RunnerError(f"{phase} guest failure marker: {failure}")
                    if automatic_input is not None and _automatic_failure_marker(serial_text):
                        _terminate_supervisor(process)
                        raise RunnerError(
                            "automatic trial guest reported a recovery or lifecycle failure"
                        )
                    if automatic_input is not None:
                        automatic_input.handle(serial_text, supervisor_log, phase_deadline)
                    elif visible_confirmation is None and _input_ready(serial_text, phase):
                        assert confirm_visible is not None
                        visible_confirmation = confirm_visible(prepared.run_id, phase, phase_deadline)
                    if phase == "reject-close" and automatic_input is None and not retry_announced:
                        lines = serial_text.splitlines()
                        if (
                            lines.count(recovery.LOGIN_WAIT) == 2
                            and lines.count(recovery.LOGIN_REJECTED) == 1
                            and lines.index(recovery.LOGIN_WAIT) < lines.index(recovery.LOGIN_REJECTED)
                            < len(lines) - 1 - lines[::-1].index(recovery.LOGIN_WAIT)
                        ):
                            print("Login rejected and awaiting input again. Press Escape once.", flush=True)
                            retry_announced = True
                    if (
                        trial_validation is not None
                        and (visible_confirmation is not None or automatic_input is not None)
                        and not host_stop_requested
                        and any(line.startswith(TERMINAL_MARKER + " ") for line in serial_text.split("\n")[:-1])
                    ):
                        host_stop_requested = stop_owned_after_terminal(
                            process, serial_text, trial_validation, phase_deadline,
                        )
                    if time.monotonic() >= supervisor_deadline and process.poll() is None:
                        _terminate_supervisor(process)
                        raise RunnerError(f"{phase} QEMU exceeded its bounded natural-exit deadline")
                    time.sleep(0.05)
            except AutomationError as error:
                if process.poll() is None:
                    _terminate_supervisor(process)
                raise RunnerError(str(error)) from None
            except BaseException:
                if process.poll() is None:
                    _terminate_supervisor(process)
                raise
            return_code = process.returncode
    finally:
        if automatic_input is not None:
            automatic_input.close()
    serial_text = _read_serial(serial_path)
    if automatic_input is not None and _automatic_failure_marker(serial_text):
        raise RunnerError("automatic trial guest reported a recovery or lifecycle failure")
    if automatic_input is not None:
        try:
            automatic_input.verify_complete()
        except AutomationError as error:
            raise RunnerError(str(error)) from None
    elif visible_confirmation is None:
        raise RunnerError(f"{phase} ended without a visible-frame confirmation")
    if _guest_failure(serial_text) is not None:
        raise RunnerError(f"{phase} guest reported a failure marker")
    summary = _supervisor_summary(supervisor_log)
    natural_exit = (
        return_code == 0
        and _shutdown_observation(serial_text, summary) == "GUEST_POWERDOWN_AND_NATURAL_EXIT"
        and summary.get("success_marker_seen") is True
    )
    host_stop = validated_host_stop_result(host_stop_requested, return_code, summary)
    if not (natural_exit or host_stop):
        raise RunnerError(f"{phase} did not finish through natural QEMU exit: {summary}")
    marker_counts = (trial_validation(serial_text) if trial_validation is not None
                     else _phase_success(serial_text, phase))
    data_after = _data_identity(prepared.data_path)
    if data_after != data_before:
        raise RunnerError(f"{phase} Data backing file identity changed during QEMU")
    serial_digest, _ = sha256_file(serial_path, f"{phase} serial output")
    finished_monotonic_ns = time.monotonic_ns()
    result = {
        "phase": phase,
        "status": "RUNNER_PHASE_COMPLETE",
        "qemu_exit": summary["qemu_exit"],
        "runner_exit": return_code,
        "timed_out": False,
        "cancelled": summary["cancelled"],
        "natural_exit": natural_exit,
        "guest_powerdown": "VERIFIED" if natural_exit else "UNVERIFIED",
        "owned_host_stop_after_terminal": host_stop,
        "started_at_utc": started_utc,
        "started_monotonic_ns": started_monotonic_ns,
        "finished_monotonic_ns": finished_monotonic_ns,
        "elapsed_seconds": round((finished_monotonic_ns - started_monotonic_ns) / 1_000_000_000, 3),
        "serial_sha256": serial_digest,
        "serial_path": serial_path.as_posix(),
        "data_identity": {"device": data_before[0], "inode": data_before[1]},
        "data_filesystem_uuid": prepared.data_uuid,
        "pre_action_frame_visibility_confirmation": visible_confirmation,
        "post_action_display_content": "NOT_VERIFIED",
        "marker_counts": marker_counts,
        "guest_memory": guest_memory_summary(serial_text),
        "qemu_arguments": argv[1:],
        "supervisor_result": summary,
    }
    if automatic_input is not None:
        result["automatic_input"] = automatic_input.summary()
    if phase == "locale-read":
        if frames.keys() != set(L9_FRAME_NAMES):
            raise RunnerError("Japanese/Dark L9 frame evidence is incomplete")
        result["rfb_frames"] = frames
        result["post_action_display_content"] = "CAPTURED_REQUIRES_REVIEW"
    return result


def _write_receipt(path: Path, receipt: dict[str, Any], *, descriptor: int | None = None) -> int:
    payload = (json.dumps(receipt, sort_keys=True, indent=2) + "\n").encode("utf-8")
    if descriptor is None:
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
        try:
            descriptor = os.open(path, flags, 0o600)
        except FileExistsError:
            raise RunnerError("refusing to overwrite an existing Task24 run receipt") from None
    else:
        try:
            current = path.lstat()
            opened = os.fstat(descriptor)
        except OSError as error:
            raise RunnerError(f"run receipt changed ({error.__class__.__name__})") from None
        if stat.S_ISLNK(current.st_mode) or _file_identity(current) != _file_identity(opened):
            raise RunnerError("run receipt path no longer matches its opened file")
        os.lseek(descriptor, 0, os.SEEK_SET)
        os.ftruncate(descriptor, 0)
    try:
        view = memoryview(payload)
        while view:
            written = os.write(descriptor, view)
            view = view[written:]
        os.fsync(descriptor)
        return descriptor
    except OSError as error:
        raise RunnerError(f"run receipt could not be written ({error.__class__.__name__})") from None


def run_authorized_pair(
    prepared: PreparedRun,
    *,
    slot: int,
    board_readback_sha256: str,
    qemu: Path,
    confirm_visible: Callable[[str, str, float], dict[str, Any]] | None = None,
    manual_trial: bool = False,
    automatic_trial: bool = False,
    fixture_manifest_path: Path | None = None,
    l9_trial: bool = False,
) -> dict[str, Any]:
    if prepared.f9_rfb_no_start:
        raise RunnerError("the F9 8e2a profile is no-start only")
    if l9_trial != (prepared.l9_pin is not None) or (l9_trial and not automatic_trial):
        raise RunnerError("L9 requires its pinned preflight candidate and automatic fixture")
    if manual_trial and automatic_trial:
        raise RunnerError("manual and automatic Task24 trial modes are mutually exclusive")
    if automatic_trial and confirm_visible is not None:
        raise RunnerError("automatic Task24 trial does not accept manual visible-frame confirmation")
    if not automatic_trial and fixture_manifest_path is not None:
        raise RunnerError("a fixture manifest requires automatic Task24 trial mode")
    if automatic_trial and fixture_manifest_path is None:
        raise RunnerError("automatic Task24 trial requires a fixture manifest")
    validate_start_gate(slot, board_readback_sha256, l9_trial=l9_trial)
    if not automatic_trial and confirm_visible is None:
        raise RunnerError("manual Task24 trial requires visible-frame confirmation")
    try:
        qemu = Path(qemu).resolve(strict=True)
    except OSError as error:
        raise RunnerError(f"QEMU is unavailable ({error.__class__.__name__})") from None
    qemu = _canonical_existing_file(qemu, "QEMU", executable=True)
    qemu_sha256, _ = sha256_file(qemu, "QEMU")
    child_runner_sha256, _ = sha256_file(QEMU_CHILD_RUNNER, "QEMU child runner")
    _verify_fixed_inputs(prepared)
    trial = manual_trial or automatic_trial
    fixture: DevelopmentFixture | None = None
    if automatic_trial:
        if not l9_trial:
            _require_c3_restart_candidate(prepared)
        assert fixture_manifest_path is not None
        fixture = load_development_fixture(
            fixture_manifest_path,
            run_directory=prepared.run_directory,
            run_id=prepared.run_id,
            preflight_manifest_sha256=prepared.manifest_sha256,
        )
    phases = MANUAL_TRIAL_PHASES if trial else PHASES
    if l9_trial:
        phases = tuple(L9_RFB_FLOW)
        plan = prepare_trial_layout(prepared, phases)
        prepared = replace(prepared, phases={
            phase: {
                **prepared.phases["read"],
                "serial_path": Path(plan["serial_destinations"][phase]),
            }
            for phase in phases
        })
    elif trial:
        plan = prepare_three_phase_trial(prepared)
        rejected = {
            **prepared.phases["read"],
            "serial_path": Path(plan["serial_destinations"]["reject-close"]),
        }
        prepared = replace(prepared, phases={**prepared.phases, "reject-close": rejected})
    receipt_path = prepared.run_directory / RECEIPT_NAME
    initial_data_identity = _data_identity(prepared.data_path)
    initial = {
        "schema": "nagi-task24-interactive-run/v1",
        "run_id": prepared.run_id,
        "manifest_candidate_source_commit": prepared.candidate_source_commit,
        "artifact_source_provenance": "UNVERIFIED_BY_PREFLIGHT",
        "manifest_sha256": prepared.manifest_sha256,
        "preflight_script_sha256": prepared.preflight_script_sha256,
        "preflight": prepared.preflight_result,
        "board_slot": slot,
        "board_readback_sha256": board_readback_sha256,
        "board_reservation_status": "operator-readback-attested; runner does not read the external board",
        "qemu_sha256": qemu_sha256,
        "qemu_child_runner_sha256": child_runner_sha256,
        "qemu_started": False,
        "display_backend": "vnc-loopback-rfb" if automatic_trial else "cocoa",
        "guest_ram_measurement": "NAGI_A_GUEST_MEM serial samples when present; guest-clock KiB",
        "desktop_input_to_present_ns": None,
        "guest_memory_host_clock_alignment": "not available; no cross-clock correlation claimed",
        "host_rss_used_as_guest_ram": False,
        "pre_action_frame_visibility": (
            "not captured; no frame visibility claim"
            if automatic_trial else "operator confirmation only; no pixel capture"
        ),
        "post_action_display_content": "NOT_VERIFIED",
        "data_freshness_evidence": "MANIFEST_DECLARED_ONLY",
        "acceptance_status": "NOT_ESTABLISHED",
        "acceptance_gates": {
            "artifact_source_provenance": "UNVERIFIED_BY_PREFLIGHT",
            "data_creation_freshness": "MANIFEST_DECLARED_ONLY",
            "post_action_display_content": "NOT_VERIFIED",
            "task24_authentication_rejection_case": "NOT_RUN",
            "desktop_input_to_present_latency": "NOT_MEASURED",
        },
        "data_filesystem_uuid": prepared.data_uuid,
        "data_identity": {
            "device": initial_data_identity[0],
            "inode": initial_data_identity[1],
        },
        "phases": [],
        "active_phase": None,
        "status": "IN_PROGRESS",
    }
    if fixture is not None:
        initial["automatic_trial"] = {
            "mode": "AUTOMATIC_L9_FIXTURE" if l9_trial else "MANIFEST_BACKED_RFB_FIXTURE",
            "fixture_manifest_sha256": fixture.sha256,
            "rfb_endpoint": f"127.0.0.1:{fixture.rfb_port}",
            "fixture_secrets_recorded": False,
            "post_action_display_content": "NOT_VERIFIED",
        }
    descriptor = _write_receipt(receipt_path, initial)
    receipt = initial
    try:
        for phase in phases:
            receipt["active_phase"] = phase
            _write_receipt(receipt_path, receipt, descriptor=descriptor)
            if _data_identity(prepared.data_path) != initial_data_identity:
                raise RunnerError("Data backing file changed between the save and read boots")
            if trial:
                previous_serial = None
                l9_args = {}
                if l9_trial and receipt["phases"]:
                    prior = receipt["phases"][-1]
                    payload = prepared.phases[prior["phase"]]["serial_path"].read_bytes()
                    if hashlib.sha256(payload).hexdigest() != prior["serial_sha256"]:
                        raise RunnerError("preceding L9 serial changed after acceptance")
                    previous_serial = payload.decode("utf-8", errors="replace")
                    try:
                        session, _ = l9_logout_session(
                            recovery._complete_lines(previous_serial, prior["phase"]),
                            L9_LOGGED_OUT, prior["phase"],
                        )
                    except ValueError as error:
                        raise RunnerError(str(error)) from None
                    l9_args["previous_l9_session"] = session

                def validate(serial: str) -> dict[str, int]:
                    try:
                        if l9_trial:
                            return validate_l9_phase(serial, phase, previous_serial)
                        if phase == "reject-close":
                            save = _read_serial(prepared.phases["save"]["serial_path"])
                            _, _, saved_receipt = recovery._validate_phase(save, "save", recovery.SAVED)
                            recovery.validate_reject_close(serial, saved_receipt)
                            return {"identity_ready_count": 0, "pid1_ready_count": 0,
                                    "confirmed_rejection_count": 1, "terminal_consumed_count": 1}
                        if phase == "read":
                            recovery.validate_save_reject_read(
                                _read_serial(prepared.phases["save"]["serial_path"]),
                                _read_serial(prepared.phases["reject-close"]["serial_path"]),
                                serial,
                            )
                    except ValueError as error:
                        raise RunnerError(str(error)) from None
                    return _phase_success(serial, phase)

                result = run_phase(
                    prepared,
                    phase,
                    qemu,
                    confirm_visible,
                    trial_validation=validate,
                    automatic_fixture=fixture if automatic_trial else None,
                    **l9_args,
                )
            else:
                result = run_phase(prepared, phase, qemu, confirm_visible)
            if result["data_identity"] != {
                "device": initial_data_identity[0],
                "inode": initial_data_identity[1],
            }:
                raise RunnerError(f"{phase} boot did not retain the original Data backing file")
            receipt["phases"].append(result)
            receipt["qemu_started"] = True
            receipt["active_phase"] = None
            _write_receipt(receipt_path, receipt, descriptor=descriptor)
        observed = (
            {"lifecycle": "L9_SERIAL_VALIDATED", "frame_review": "PENDING"}
            if l9_trial else observe_completed_pair(prepared.run_directory, include_rejection=trial)
        )
        if not trial and any(
            value != "GUEST_POWERDOWN_AND_NATURAL_EXIT" for value in observed["shutdown"].values()
        ):
            raise RunnerError("paired guest powerdown and natural exit are unverified")
        if l9_trial:
            receipt["post_action_display_content"] = "CAPTURED_REQUIRES_REVIEW"
            receipt["acceptance_gates"]["post_action_display_content"] = "CAPTURED_REQUIRES_REVIEW"
        elif trial:
            receipt["acceptance_gates"]["task24_authentication_rejection_case"] = "SERIAL_VALIDATED"
        receipt["recovery_observation"] = observed
        receipt["status"] = "RUNNER_COMPLETE"
        receipt["same_data_backing_file_across_boots"] = (
            all(item["data_identity"] == receipt["data_identity"] for item in receipt["phases"])
        )
        if not receipt["same_data_backing_file_across_boots"]:
            raise RunnerError("save/read boots did not use the same Data backing file")
        _write_receipt(receipt_path, receipt, descriptor=descriptor)
        return receipt
    except BaseException as error:
        receipt["qemu_started"] = any(
            (prepared.run_directory / phase / "supervisor.log").exists()
            and "qemu_child_started pid="
            in (prepared.run_directory / phase / "supervisor.log").read_text(
                encoding="utf-8", errors="replace"
            )
            for phase in phases
        )
        receipt["status"] = "RUNNER_FAILED"
        receipt["failed_phase"] = receipt["active_phase"]
        receipt["failure_stage"] = "phase" if receipt["active_phase"] is not None else "final_pair_validation"
        receipt["active_phase"] = None
        receipt["failure"] = f"{error.__class__.__name__}: {error}"
        _write_receipt(receipt_path, receipt, descriptor=descriptor)
        raise
    finally:
        os.close(descriptor)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--preflight-script", type=Path)
    parser.add_argument(
        "--observe-completed-pair",
        type=Path,
        help="read only the existing save/read serial and supervisor logs under this directory",
    )
    parser.add_argument("--include-reject-close", action="store_true")
    parser.add_argument("--l9-candidate-head", help="the board-fixed L9 source commit")
    parser.add_argument("--l9-initramfs-sha256", help="the board-fixed L9 signed initramfs SHA-256")
    parser.add_argument("--manual-trial", action="store_true",
                        help="select the three-boot manual trial with --authorize-start")
    parser.add_argument(
        "--automatic-development-trial", action="store_true",
        help="select the pinned three-boot loopback RFB fixture with --authorize-start",
    )
    parser.add_argument(
        "--prepare-manual-trial", action="store_true",
        help="check the save, rejection and read output layout without starting QEMU",
    )
    parser.add_argument(
        "--prepare-automatic-trial", action="store_true",
        help="validate the pinned fixture and three-phase layout without starting QEMU",
    )
    parser.add_argument(
        "--fixture-manifest", type=Path,
        help="private run-directory development-fixture.json for the automatic trial",
    )
    retest = parser.add_mutually_exclusive_group()
    retest.add_argument("--notes-save-retest", action="store_true",
                        help="use the pinned local Notes save candidate for the manual trial")
    retest.add_argument("--c3-restart-retest", action="store_true",
                        help="use the pinned local C3 restart candidate for the manual trial")
    retest.add_argument(
        "--f9-8e2a-rfb-no-start", action="store_true",
        help="prepare only the fixed 8e2a source/ELF/CPIO RFB fixture; QEMU start is forbidden",
    )
    parser.add_argument("--pid1-elf", type=Path,
                        help="separate fixed PID 1 ELF input required by the F9 no-start profile")
    parser.add_argument("--f9-fixed-input-receipt", type=Path,
                        help="fixed candidate input binding receipt required by the F9 no-start profile")
    parser.add_argument("--f9-cpio-validation-receipt", type=Path,
                        help="canonical development CPIO validation receipt for the F9 profile")
    parser.add_argument("--qemu", type=Path, help="required only with --authorize-start")
    parser.add_argument("--slot", type=int, choices=(1, 2), help="the board-reserved QEMU slot")
    parser.add_argument("--board-readback-sha256", help="full SHA-256 of a fresh official board readback")
    parser.add_argument(
        "--authorize-start",
        action="store_true",
        help="after all pins pass, start the explicitly selected bounded Task24 trial",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = _parser()
    args = parser.parse_args(argv)
    try:
        l9_trial = args.l9_candidate_head is not None or args.l9_initramfs_sha256 is not None
        if l9_trial and (args.l9_candidate_head is None or args.l9_initramfs_sha256 is None
                         or not (args.automatic_development_trial or args.prepare_automatic_trial)):
            parser.error("L9 requires both candidate pins and an automatic preparation or trial")
        if args.observe_completed_pair is not None:
            if (args.authorize_start or args.manifest is not None or args.preflight_script is not None
                    or args.qemu is not None or args.slot is not None
                    or args.board_readback_sha256 is not None or args.prepare_manual_trial
                    or args.prepare_automatic_trial or args.manual_trial
                    or args.automatic_development_trial or args.fixture_manifest is not None
                    or args.notes_save_retest or args.c3_restart_retest or args.f9_8e2a_rfb_no_start
                    or args.pid1_elf is not None or args.f9_fixed_input_receipt is not None
                    or args.f9_cpio_validation_receipt is not None):
                parser.error("--observe-completed-pair does not accept preflight or launch arguments")
            observation = observe_completed_pair(
                args.observe_completed_pair, include_rejection=args.include_reject_close,
            )
            print(json.dumps(observation, sort_keys=True))
            return 0
        if args.include_reject_close:
            parser.error("--include-reject-close requires --observe-completed-pair")
        if args.prepare_manual_trial and args.authorize_start:
            parser.error("--prepare-manual-trial does not authorize QEMU")
        if args.prepare_automatic_trial and args.authorize_start:
            parser.error("--prepare-automatic-trial does not authorize QEMU")
        f9_input_values = (
            args.pid1_elf, args.f9_fixed_input_receipt, args.f9_cpio_validation_receipt,
        )
        f9_inputs_supplied = all(value is not None for value in f9_input_values)
        if args.f9_8e2a_rfb_no_start:
            if not args.prepare_automatic_trial or args.authorize_start:
                parser.error("the F9 8e2a profile requires --prepare-automatic-trial and forbids --authorize-start")
            if not f9_inputs_supplied:
                parser.error("the F9 8e2a profile requires --pid1-elf and both fixed candidate receipts")
            if l9_trial or args.notes_save_retest or args.c3_restart_retest:
                parser.error("the F9 8e2a profile cannot be combined with another Task24 candidate pin")
        elif args.pid1_elf is not None or args.f9_fixed_input_receipt is not None or args.f9_cpio_validation_receipt is not None:
            parser.error("F9 candidate receipts require --f9-8e2a-rfb-no-start")
        if args.manual_trial and args.automatic_development_trial:
            parser.error("--manual-trial and --automatic-development-trial are mutually exclusive")
        if (args.manual_trial or args.automatic_development_trial) and not args.authorize_start:
            parser.error("trial launch modes require --authorize-start")
        if args.automatic_development_trial and args.prepare_automatic_trial:
            parser.error("choose automatic trial preparation or launch, not both")
        automatic_selected = args.automatic_development_trial or args.prepare_automatic_trial
        if automatic_selected != (args.fixture_manifest is not None):
            parser.error("automatic trial preparation or launch requires exactly one --fixture-manifest")
        if automatic_selected and not (args.c3_restart_retest or l9_trial or args.f9_8e2a_rfb_no_start):
            parser.error("automatic development trial requires a fixed candidate profile")
        if args.manifest is None or args.preflight_script is None:
            parser.error("--manifest and --preflight-script are required for preflight")
        if (args.notes_save_retest or args.c3_restart_retest or args.f9_8e2a_rfb_no_start) and not (
            args.prepare_manual_trial or args.manual_trial or automatic_selected
        ):
            parser.error("a retest requires --prepare-manual-trial or --manual-trial")
        prepared = prepare_run(
            args.manifest, args.preflight_script,
            notes_save_retest=args.notes_save_retest, c3_restart_retest=args.c3_restart_retest,
            l9_pin=(args.l9_candidate_head, args.l9_initramfs_sha256) if l9_trial else None,
            f9_rfb_no_start=args.f9_8e2a_rfb_no_start,
            f9_pid1_elf_path=args.pid1_elf,
            f9_fixed_input_receipt_path=args.f9_fixed_input_receipt,
            f9_cpio_validation_receipt_path=args.f9_cpio_validation_receipt,
        )
        if not args.authorize_start:
            print(
                json.dumps(
                    {
                        "runner": "PREPARED_NO_START",
                        "preflight": prepared.preflight_result,
                        "manifest_sha256": prepared.manifest_sha256,
                        "preflight_script_sha256": prepared.preflight_script_sha256,
                        "qemu_started": False,
                        **({"candidate_check_receipt": f9_candidate_check_receipt(prepared)}
                           if prepared.f9_rfb_no_start else {}),
                        **({"manual_trial": prepare_manual_trial(prepared)}
                           if args.prepare_manual_trial else {}),
                        **({"automatic_trial": prepare_automatic_trial(prepared, args.fixture_manifest, l9_trial=l9_trial)}
                           if args.prepare_automatic_trial else {}),
                    },
                    sort_keys=True,
                )
            )
            return 0
        validate_start_gate(args.slot, args.board_readback_sha256, l9_trial=l9_trial)
        if args.qemu is None:
            raise RunnerError("--qemu is required with --authorize-start")
        if not args.automatic_development_trial and not sys.stdin.isatty():
            raise RunnerError("interactive start requires a Terminal TTY")
        receipt = run_authorized_pair(
            prepared,
            slot=args.slot,
            board_readback_sha256=args.board_readback_sha256,
            qemu=args.qemu,
            confirm_visible=(None if args.automatic_development_trial else request_visible_confirmation),
            manual_trial=args.manual_trial,
            automatic_trial=args.automatic_development_trial,
            fixture_manifest_path=args.fixture_manifest,
            l9_trial=l9_trial,
        )
        print(json.dumps(receipt, sort_keys=True))
        return 0
    except (OSError, RunnerError, AutomationError) as error:
        print(f"task24_interactive_runner=FAIL reason={error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
