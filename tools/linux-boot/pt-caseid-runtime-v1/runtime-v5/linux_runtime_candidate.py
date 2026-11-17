#!/usr/bin/env python3
"""Build and verify task-local Linux userspace smoke fixtures.

Host input paths are explicit absolute CLI arguments. Outputs are restricted to
the supplied task root. QEMU process supervision is delegated to the existing
linux-boot/run_qemu_child.py contract.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
import re
import stat
import subprocess
import sys
import time
from pathlib import Path


class CandidateError(Exception):
    def __init__(self, message: str, status: int = 1, result: str = "FAIL"):
        self.status = status
        self.result = result
        super().__init__(message)


DEFAULT_ENTRY_NAMES = (
    "linux_model_clock_consumer",
    "notes-host-preview",
    "in_process_echo",
    "sdk-development-smoke",
    "installed_service_clock_smoke",
    "notes-event-bridge-smoke",
    "desktop-host-preview-smoke",
    "desktop-evdev-stream-smoke",
    "desktop-evdev-integrity-smoke",
)
DEFAULT_ENTRY_CASE_COUNTS = {
    "linux_model_clock_consumer": 1,
    "notes-host-preview": 2,
    "in_process_echo": 1,
    "sdk-development-smoke": 1,
    "installed_service_clock_smoke": 1,
    "notes-event-bridge-smoke": 2,
    "desktop-host-preview-smoke": 1,
    "desktop-evdev-stream-smoke": 4,
    "desktop-evdev-integrity-smoke": 3,
}
DEFAULT_CASE_IDS_BY_ENTRY = {
    "linux_model_clock_consumer": ("linux_model_clock_consumer",),
    "notes-host-preview": ("notes-host-preview.save", "notes-host-preview.read"),
    "in_process_echo": ("in_process_echo",),
    "sdk-development-smoke": ("sdk-development-smoke",),
    "installed_service_clock_smoke": ("installed_service_clock_smoke",),
    "notes-event-bridge-smoke": ("notes-event-bridge-smoke.save", "notes-event-bridge-smoke.read"),
    "desktop-host-preview-smoke": ("desktop-host-preview-smoke",),
    "desktop-evdev-stream-smoke": (
        "desktop-evdev-stream-smoke.create-empty",
        "desktop-evdev-stream-smoke.producer",
        "desktop-evdev-stream-smoke.stdin",
        "desktop-evdev-stream-smoke.read",
    ),
    "desktop-evdev-integrity-smoke": (
        "desktop-evdev-integrity-smoke.integrity-seed-v1",
        "desktop-evdev-integrity-smoke.integrity-save",
        "desktop-evdev-integrity-smoke.integrity-check",
    ),
}
EXPECTED_NEGATIVE_ENTRY = "in_process_echo"
EXPECTED_NEGATIVE_CASE = (23, 0)
EXPECTED_NEGATIVE_CASE_ID = "in_process_echo"
INIT_UNAVAILABLE_LINE = "nagi-init: startup failed (Unavailable); no service is ready"


def _absolute(raw: str, label: str) -> Path:
    path = Path(raw)
    if not path.is_absolute() or any(part in (".", "..") for part in path.parts):
        raise CandidateError(f"{label} must be an absolute normalized path", 2)
    return path


def _real_path(path: Path, label: str, *, final_directory: bool | None = None,
               executable: bool = False) -> os.stat_result:
    current = Path(path.anchor)
    info = None
    parts = path.parts[1:]
    for index, part in enumerate(parts):
        current = current / part
        try:
            info = current.lstat()
        except FileNotFoundError:
            raise CandidateError(f"{label} is missing: {current}", 2) from None
        if stat.S_ISLNK(info.st_mode):
            raise CandidateError(f"{label} contains a symlink: {current}", 2)
        if index < len(parts) - 1 and not stat.S_ISDIR(info.st_mode):
            raise CandidateError(f"{label} parent is not a directory: {current}", 2)
    if info is None:
        raise CandidateError(f"{label} is not a file or directory", 2)
    if final_directory is True and not stat.S_ISDIR(info.st_mode):
        raise CandidateError(f"{label} is not a directory: {path}", 2)
    if final_directory is False and not stat.S_ISREG(info.st_mode):
        raise CandidateError(f"{label} is not a regular file: {path}", 2)
    if executable and not info.st_mode & 0o111:
        raise CandidateError(f"{label} has no executable mode bits: {path}", 2)
    return info


def _task_and_output_dirs(task_raw: str, output_raw: str, *, create: bool) -> tuple[Path, Path]:
    task = _absolute(task_raw, "task-root")
    _real_path(task, "task-root", final_directory=True)
    output = _absolute(output_raw, "output-dir")
    if output == task or os.path.commonpath((str(task), str(output))) != str(task):
        raise CandidateError("output-dir must be a child of task-root", 2)
    relative = output.relative_to(task)
    current = task
    for index, part in enumerate(relative.parts):
        current = current / part
        try:
            info = current.lstat()
        except FileNotFoundError:
            if index != len(relative.parts) - 1:
                raise CandidateError(f"output-dir parent is missing: {current}", 2) from None
            if not create:
                break
            try:
                current.mkdir(mode=0o700)
            except OSError as error:
                raise CandidateError(f"cannot create output-dir: {error}", 1) from error
            info = current.lstat()
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            raise CandidateError(f"output-dir component is not a real directory: {current}", 2)
    return task, output


def _input_file(raw: str, label: str, *, executable: bool = False,
                allow_symlink: bool = False) -> Path:
    path = _absolute(raw, label)
    if allow_symlink:
        try:
            path = path.resolve(strict=True)
        except OSError as error:
            raise CandidateError(f"{label} cannot be resolved: {error}", 2) from error
    _real_path(path, label, final_directory=False, executable=executable)
    return path


def _leaf(raw: str, label: str) -> str:
    if (not raw or raw in (".", "..") or "/" in raw or "\\" in raw
            or "\0" in raw or "\r" in raw or "\n" in raw):
        raise CandidateError(f"{label} must be a single safe filename", 2)
    return raw


def _read_regular(path: Path, label: str) -> bytes:
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(path, flags)
    except OSError as error:
        raise CandidateError(f"cannot open {label}: {error}", 2) from error
    try:
        before = os.fstat(fd)
        if not stat.S_ISREG(before.st_mode):
            raise CandidateError(f"{label} is not a regular file", 2)
        chunks = []
        while True:
            try:
                chunk = os.read(fd, 1024 * 1024)
            except InterruptedError:
                continue
            except OSError as error:
                raise CandidateError(f"I/O error reading {label}: {error}", 1) from error
            if not chunk:
                break
            chunks.append(chunk)
        after = os.fstat(fd)
        if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) != (
            after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns
        ):
            raise CandidateError(f"{label} changed while being read", 1)
        return b"".join(chunks)
    finally:
        os.close(fd)


def _load_builder():
    builder_path = Path(__file__).resolve().parent.parent / "build_initramfs.py"
    spec = importlib.util.spec_from_file_location("nagi_dev_smoke_builder", builder_path)
    if spec is None or spec.loader is None:
        raise CandidateError("cannot load task-local CPIO builder", 2)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _append_runner(builder, archive: bytes, entry_name: str, payload: bytes) -> bytes:
    builder.safe_name(entry_name)
    records = builder.parse_newc(archive)
    names = [item["name"] for item in records[:-1]]
    if records[-1]["name"] != builder.TRAILER or entry_name in names:
        raise CandidateError("base CPIO has a duplicate entry or invalid trailer", 2)
    trailer_offset = records[-1]["offset"]
    inode = int(archive[trailer_offset + 6:trailer_offset + 14], 16)
    output = bytearray(archive[:trailer_offset])
    output.extend(builder.emit(entry_name, stat.S_IFREG | 0o755, payload, inode, len(output)))
    output.extend(builder.emit(builder.TRAILER, 0, b"", inode + 1, len(output)))
    result = bytes(output)
    parsed = builder.parse_newc(result)
    if [item["name"] for item in parsed[:-1]] != names + [entry_name]:
        raise CandidateError("positive CPIO path readback mismatch", 1)
    if result[:trailer_offset] != archive[:trailer_offset]:
        raise CandidateError("positive CPIO changed original records", 1)
    selected = parsed[-2]
    if selected["mode"] != stat.S_IFREG | 0o755 or selected["data"] != payload:
        raise CandidateError("positive CPIO runner readback mismatch", 1)
    return result


def _replace_entry(builder, archive: bytes, target: str, payload: bytes) -> bytes:
    records = builder.parse_newc(archive)
    builder.safe_name(target)
    selected = [item for item in records[:-1] if item["name"] == target]
    if len(selected) != 1 or not stat.S_ISREG(selected[0]["mode"]):
        raise CandidateError(f"replacement target must be one regular CPIO entry: {target}", 2)
    output = bytearray()
    replaced = 0
    for item in records:
        start, end = item["offset"], item["next"]
        if item["name"] != target:
            output.extend(archive[start:end])
            continue
        header = bytearray(archive[start:start + 110])
        header[54:62] = f"{len(payload):08x}".encode("ascii")
        name_and_padding = archive[start + 110:item["data_offset"]]
        record = bytearray(header + name_and_padding + payload)
        record.extend(b"\0" * ((-(len(output) + len(record))) & 3))
        output.extend(record)
        replaced += 1
    result = bytes(output)
    after_records = builder.parse_newc(result)
    if replaced != 1 or [r["name"] for r in records] != [r["name"] for r in after_records]:
        raise CandidateError("negative CPIO path readback mismatch", 1)
    for before, after in zip(records, after_records, strict=True):
        if before["name"] == target:
            if after["data"] != payload or after["mode"] != before["mode"]:
                raise CandidateError("negative CPIO replacement readback mismatch", 1)
        elif before["name"] != builder.TRAILER:
            if archive[before["offset"]:before["next"]] != result[after["offset"]:after["next"]]:
                raise CandidateError(f"negative CPIO changed non-target entry: {before['name']}", 1)
    return result


def _write_new(path: Path, data: bytes) -> None:
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(path, flags, 0o600)
    except OSError as error:
        raise CandidateError(f"cannot create output {path.name}: {error}", 1) from error
    try:
        offset = 0
        while offset < len(data):
            try:
                count = os.write(fd, data[offset:])
            except InterruptedError:
                continue
            except OSError as error:
                raise CandidateError(f"I/O error writing {path.name}: {error}", 1) from error
            if count <= 0:
                raise CandidateError(f"short/zero write for output {path.name}", 1)
            offset += count
        try:
            os.fsync(fd)
        except OSError as error:
            raise CandidateError(f"I/O error syncing {path.name}: {error}", 1) from error
    finally:
        os.close(fd)


def build_fixtures(args) -> dict[str, object]:
    task, output_dir = _task_and_output_dirs(args.task_root, args.output_dir, create=False)
    bundle_path = _input_file(args.bundle, "bundle")
    runner_path = _input_file(args.runner_elf, "runner-elf", executable=True)
    fail_path = _input_file(args.fail_child_elf, "fail-child-elf", executable=True)
    positive_name = _leaf(args.positive_name, "positive-name")
    negative_name = _leaf(args.negative_name, "negative-name")
    if positive_name == negative_name:
        raise CandidateError("positive-name and negative-name must differ", 2)
    positive_path = output_dir / positive_name
    negative_path = output_dir / negative_name
    for path in (positive_path, negative_path):
        if (path.name in ("", ".", "..") or "/" in path.name or "\\" in path.name
                or path.exists() or path.is_symlink()):
            raise CandidateError(f"output file must be a new leaf: {path}", 2)
    builder = _load_builder()
    bundle = _read_regular(bundle_path, "bundle")
    runner = _read_regular(runner_path, "runner-elf")
    fail_child = _read_regular(fail_path, "fail-child-elf")
    try:
        builder.validate_elf(runner, "runner-elf", builder.ET_EXEC)
        builder.validate_elf(fail_child, "fail-child-elf", builder.ET_EXEC)
        positive = _append_runner(builder, bundle, args.runner_entry, runner)
        negative = _replace_entry(builder, positive, args.replace_entry, fail_child)
    except builder.BuildError as error:
        raise CandidateError(f"fixture validation failed: {error}", 2) from error
    # Create the one missing output leaf only after all inputs and CPIOs validate.
    task, output_dir = _task_and_output_dirs(args.task_root, args.output_dir, create=True)
    _write_new(output_dir / positive_name, positive)
    _write_new(output_dir / negative_name, negative)
    positive_readback = _read_regular(output_dir / positive_name, "positive fixture output")
    negative_readback = _read_regular(output_dir / negative_name, "negative fixture output")
    if positive_readback != positive or negative_readback != negative:
        raise CandidateError("fixture output readback mismatch", 1)
    return {
        "result": "PASS",
        "task_root": str(task),
        "positive_fixture": str(output_dir / positive_name),
        "positive_sha256": hashlib.sha256(positive_readback).hexdigest(),
        "negative_fixture": str(output_dir / negative_name),
        "negative_sha256": hashlib.sha256(negative_readback).hexdigest(),
        "replacement": args.replace_entry,
    }


def _guest_runner_record(builder, archive: bytes, rdinit: str) -> None:
    if (not rdinit.startswith("/") or any(part in ("", ".", "..") for part in rdinit[1:].split("/"))
            or any(ch in rdinit for ch in "\r\n\0")):
        raise CandidateError("rdinit must be a normalized absolute guest path", 2)
    entry_name = rdinit[1:]
    records = builder.parse_newc(archive)
    selected = [item for item in records[:-1] if item["name"] == entry_name]
    if len(selected) != 1 or not stat.S_ISREG(selected[0]["mode"]) or not selected[0]["mode"] & 0o111:
        raise CandidateError("rdinit does not identify one executable CPIO file", 2)
    try:
        builder.validate_elf(selected[0]["data"], entry_name, builder.ET_EXEC)
    except builder.BuildError as error:
        raise CandidateError(f"rdinit ELF validation failed: {error}", 2) from error


def _read_log(path: Path) -> bytes:
    flags = os.O_RDWR | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(path, flags)
    except OSError as error:
        raise CandidateError(f"I/O error opening QEMU log: {error}", 1) from error
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode):
            raise CandidateError("QEMU log is not a regular file", 1)
        try:
            os.fsync(fd)
            os.lseek(fd, 0, os.SEEK_SET)
        except OSError as error:
            raise CandidateError(f"I/O error syncing QEMU log: {error}", 1) from error
        chunks = []
        while True:
            try:
                chunk = os.read(fd, 1024 * 1024)
            except InterruptedError:
                continue
            except OSError as error:
                raise CandidateError(f"I/O error reading QEMU log: {error}", 1) from error
            if not chunk:
                return b"".join(chunks)
            chunks.append(chunk)
    finally:
        os.close(fd)


def _supervisor_summary(output: bytes) -> dict[str, object]:
    summaries = []
    for line in output.decode("utf-8", errors="replace").splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and "qemu_exit" in value and "timed_out" in value:
            summaries.append(value)
    if len(summaries) != 1:
        raise CandidateError(
            f"QEMU child supervisor returned {len(summaries)} machine summaries; expected exactly one", 1
        )
    summary = summaries[0]
    qemu_exit = summary.get("qemu_exit")
    timed_out = summary.get("timed_out")
    cancelled = summary.get("cancelled")
    cancel_signal = summary.get("cancel_signal")
    elapsed = summary.get("elapsed_seconds")
    if (type(qemu_exit) is not int or type(timed_out) is not bool
            or type(cancelled) is not bool
            or (cancel_signal is not None and type(cancel_signal) is not int)
            or type(elapsed) not in (int, float) or not math.isfinite(elapsed) or elapsed < 0):
        raise CandidateError(f"QEMU child supervisor returned malformed status: {summary}", 1)
    if cancelled != (cancel_signal is not None):
        raise CandidateError(f"QEMU child supervisor returned inconsistent cancellation status: {summary}", 1)
    return summary


def _validate_supervisor_result(child_exit: int | None, output: bytes, serial: bytes) -> dict[str, object]:
    linux_started = b"Linux version " in serial or b"Booting the kernel" in serial
    try:
        summary = _supervisor_summary(output)
    except CandidateError as error:
        classification = "SUPERVISOR_ABNORMAL" if linux_started else "STARTUP_FAILED"
        raise CandidateError(str(error), error.status, classification) from error
    if summary["timed_out"] is True:
        raise CandidateError(f"QEMU child supervisor timed out: {summary}", 1, "TIMEOUT")
    if summary["cancelled"] is True:
        raise CandidateError(f"QEMU child supervisor was cancelled: {summary}", 1, "CANCELLED")
    if summary["qemu_exit"] != 0:
        classification = "CHILD_ABNORMAL" if linux_started else "STARTUP_FAILED"
        raise CandidateError(
            f"QEMU child exited abnormally before accepted guest result: {summary}", 1, classification
        )
    if child_exit != 0:
        raise CandidateError(
            f"QEMU child supervisor exited abnormally: helper_exit={child_exit}, summary={summary}",
            1,
            "SUPERVISOR_ABNORMAL",
        )
    return summary


def _normal_init_result(text: str, ready_marker: str) -> dict[str, object]:
    lines = text.splitlines()
    if INIT_UNAVAILABLE_LINE in lines:
        raise CandidateError(
            "normal init reported Unavailable; no service is ready, and this is not a fixture PASS",
            1,
            "UNAVAILABLE",
        )
    marker_count = lines.count(ready_marker)
    if marker_count == 0:
        raise CandidateError("normal init READY marker is missing", 1, "READY_MISSING")
    if marker_count != 1:
        raise CandidateError(f"normal init READY marker appeared {marker_count} times", 1, "READY_MALFORMED")
    return {"result": "READY", "readiness": "READY", "fixture_pass": False}


def run_guest(args) -> dict[str, object]:
    task, output_dir = _task_and_output_dirs(args.task_root, args.output_dir, create=False)
    qemu = _input_file(args.qemu, "qemu", executable=True, allow_symlink=True)
    kernel = _input_file(args.kernel, "kernel")
    initrd = _input_file(args.initrd, "initrd")
    child_runner = _input_file(args.qemu_child_runner, "qemu-child-runner")
    if not 1 <= args.timeout_seconds <= 600:
        raise CandidateError("timeout-seconds must be between 1 and 600", 2)
    if args.contract == "fixture":
        if args.expected_runner_status is None:
            raise CandidateError("fixture contract requires --expected-runner-status", 2)
        if args.ready_marker is not None:
            raise CandidateError("--ready-marker is only valid with --contract normal-init", 2)
        if not 1 <= args.expected_entries <= 256:
            raise CandidateError("expected-entries must be between 1 and 256", 2)
        expected_names = args.expected_entry or list(DEFAULT_ENTRY_NAMES)
        if len(expected_names) != args.expected_entries or len(set(expected_names)) != len(expected_names):
            raise CandidateError("expected-entry names must be unique and match expected-entries", 2)
        if any(name not in DEFAULT_ENTRY_CASE_COUNTS for name in expected_names):
            raise CandidateError("expected-entry has no fixed case-count contract", 2)
        expected_case_ids = tuple(
            case_id for name in expected_names for case_id in DEFAULT_CASE_IDS_BY_ENTRY[name]
        )
        if len(expected_case_ids) != len(set(expected_case_ids)):
            raise CandidateError("fixed entry contracts contain duplicate case IDs", 2)
        expected_case_id_set = set(expected_case_ids)
    elif args.contract == "normal-init":
        if args.rdinit != "/init":
            raise CandidateError("normal-init contract requires --rdinit /init", 2)
        if args.expected_runner_status is not None or args.expected_entry is not None:
            raise CandidateError("fixture runner expectations cannot be used with normal-init", 2)
        if (not args.ready_marker or args.ready_marker.strip() == ""
                or any(char in args.ready_marker for char in "\r\n\0")):
            raise CandidateError("normal-init contract requires one exact --ready-marker line", 2)
        expected_names = []
        expected_case_ids = ()
        expected_case_id_set = set()
    else:
        raise CandidateError(f"unsupported runtime contract: {args.contract}", 2)
    log_name = _leaf(args.log_name, "log-name")
    log_path = output_dir / log_name
    qmp_path = output_dir / (log_name + ".qmp.sock")
    for path in (log_path, qmp_path):
        if path.exists() or path.is_symlink():
            raise CandidateError(f"refusing existing or invalid output path: {path}", 2)

    builder = _load_builder()
    initrd_bytes = _read_regular(initrd, "initrd")
    _guest_runner_record(builder, initrd_bytes, args.rdinit)
    task, output_dir = _task_and_output_dirs(args.task_root, args.output_dir, create=True)
    log_path = output_dir / log_name
    qmp_path = output_dir / (log_name + ".qmp.sock")
    for path in (log_path, qmp_path):
        if path.exists() or path.is_symlink():
            raise CandidateError(f"refusing existing output path: {path}", 2)

    qmp_arg = f"unix:{qmp_path},server=on,wait=off"
    qemu_argv = [
        str(qemu),
        "-machine", "pc,accel=tcg",
        "-cpu", "qemu64",
        "-m", "1024",
        "-smp", "2",
        "-display", "none",
        "-serial", "stdio",
        "-monitor", "none",
        "-nic", "none",
        "-no-reboot",
        "-qmp", qmp_arg,
        "-kernel", str(kernel),
        "-initrd", str(initrd),
        "-append", f"console=ttyS0,115200n8 rdinit={args.rdinit} panic=1 oops=panic",
    ]
    supervisor_argv = [
        sys.executable, str(child_runner),
        "--timeout-seconds", str(args.timeout_seconds),
        "--qmp-socket", str(qmp_path),
        "--qemu-log", str(log_path),
        "--", *qemu_argv,
    ]
    started = time.monotonic()
    try:
        child = subprocess.Popen(
            supervisor_argv,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    except OSError as error:
        raise CandidateError(f"cannot start QEMU child supervisor: {error}", 1, "STARTUP_FAILED") from error
    try:
        supervisor_output, _ = child.communicate(timeout=args.timeout_seconds + 20)
    except subprocess.TimeoutExpired:
        # The mainline supervisor handles its QEMU child on SIGTERM; wait for it
        # before escalating against this directly-owned supervisor process.
        child.terminate()
        try:
            supervisor_output, _ = child.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            child.kill()
            supervisor_output, _ = child.communicate()
        raise CandidateError("QEMU child supervisor exceeded its deadline", 1, "TIMEOUT")
    if log_path.is_symlink():
        raise CandidateError("QEMU log was replaced by a symlink", 1)
    log = _read_log(log_path) if log_path.exists() else b""
    supervisor = _validate_supervisor_result(child.returncode, supervisor_output, log)
    text = log.decode("utf-8", errors="replace").replace("\r\n", "\n")
    if args.contract == "normal-init":
        init_result = _normal_init_result(text, args.ready_marker)
        return {
            **init_result,
            "task_root": str(task),
            "qemu_exit": supervisor["qemu_exit"],
            "timed_out": False,
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "log": str(log_path),
            "log_sha256": hashlib.sha256(log).hexdigest(),
        }
    if INIT_UNAVAILABLE_LINE in text.splitlines():
        raise CandidateError(
            "normal init reported Unavailable; fixture output cannot be accepted as PASS",
            1,
            "UNAVAILABLE",
        )
    done = re.findall(
        r"^RUNNER_DONE entries=(\d+) status=(\d+) output_error=(\d+)\b",
        text, re.MULTILINE,
    )
    entries = re.findall(
        r"^ENTRY_RESULT name=([A-Za-z0-9_.-]+) cases=([-0-9,/expected]+) outcome=(PASS|FAIL)$",
        text, re.MULTILINE,
    )
    case_results = re.findall(
        r"^CASE_RESULT id=([A-Za-z0-9_.-]+) exit=(-?[0-9]+) expected=(-?[0-9]+) outcome=(PASS|FAIL)$",
        text, re.MULTILINE,
    )
    case_result_lines = [line for line in text.splitlines() if line.startswith("CASE_RESULT")]
    panics = re.findall(
        r"Attempted to kill init! exitcode=0x([0-9a-fA-F]+)", text
    )
    if (len(done) != 1 or len(entries) != args.expected_entries
            or len(case_result_lines) != len(case_results)
            or len(case_results) != len(expected_case_ids) or len(panics) != 1):
        raise CandidateError("QEMU log has missing, malformed, mixed, or duplicated runner/guest result markers", 1)
    if len({record[0] for record in entries}) != len(entries):
        raise CandidateError("QEMU log repeats an entry result name", 1)
    if {record[0] for record in entries} != set(expected_names):
        raise CandidateError("QEMU log entry names do not match the expected set", 1)
    case_ids = [record[0] for record in case_results]
    if len(set(case_ids)) != len(case_ids):
        raise CandidateError("QEMU log repeats a case ID", 1)
    if set(case_ids) != expected_case_id_set:
        missing = sorted(expected_case_id_set - set(case_ids))
        unknown = sorted(set(case_ids) - expected_case_id_set)
        raise CandidateError(f"QEMU log case ID set mismatch: missing={missing}, unknown={unknown}", 1)
    parsed_case_results = {}
    for case_id, exit_text, expected_text, outcome in case_results:
        actual_exit, expected_exit = int(exit_text), int(expected_text)
        if expected_exit != 0:
            raise CandidateError(f"unexpected expected exit for case {case_id}: {expected_exit}", 1)
        if (actual_exit == expected_exit) != (outcome == "PASS"):
            raise CandidateError(f"case result/outcome mismatch for {case_id}", 1)
        parsed_case_results[case_id] = (actual_exit, expected_exit, outcome)
    runner_entries, runner_status, output_error = map(int, done[0])
    parsed_entries = {}
    for name, case_text, outcome in entries:
        cases = []
        for case_text_item in case_text.split(","):
            actual_expected = case_text_item.split("/expected")
            if len(actual_expected) != 2:
                raise CandidateError(f"malformed case result for {name}", 1)
            try:
                cases.append((int(actual_expected[0]), int(actual_expected[1])))
            except ValueError as error:
                raise CandidateError(f"malformed case result for {name}", 1) from error
        if len(cases) != DEFAULT_ENTRY_CASE_COUNTS[name]:
            raise CandidateError(f"entry case count mismatch for {name}: {len(cases)}", 1)
        parsed_entries[name] = (cases, outcome)
    for name, (entry_cases, entry_outcome) in parsed_entries.items():
        case_ids_for_entry = DEFAULT_CASE_IDS_BY_ENTRY[name]
        case_records = [parsed_case_results[case_id] for case_id in case_ids_for_entry]
        if entry_cases != [(actual, expected) for actual, expected, _ in case_records]:
            raise CandidateError(f"entry/case marker disagreement for {name}", 1)
        group_outcome = "PASS" if all(outcome == "PASS" for _, _, outcome in case_records) else "FAIL"
        if entry_outcome != group_outcome:
            raise CandidateError(f"entry/case outcome disagreement for {name}", 1)
    passed = sum(outcome == "PASS" for _, outcome in parsed_entries.values())
    failed = sum(outcome == "FAIL" for _, outcome in parsed_entries.values())
    case_passed = sum(outcome == "PASS" for _, _, outcome in parsed_case_results.values())
    case_failed = sum(outcome == "FAIL" for _, _, outcome in parsed_case_results.values())
    guest_status = (int(panics[0], 16) >> 8) & 0xFF
    if runner_entries != args.expected_entries:
        raise CandidateError("runner entry count mismatch", 1)
    if runner_status != args.expected_runner_status or guest_status != args.expected_runner_status:
        raise CandidateError(
            f"runner/guest exit mismatch: expected={args.expected_runner_status}, "
            f"runner={runner_status}, guest={guest_status}",
            1,
        )
    if output_error != 0:
        raise CandidateError(f"guest runner output_error={output_error}", 1)
    if args.expected_runner_status == 0:
        if case_failed != 0 or case_passed != len(expected_case_ids):
            raise CandidateError("expected success but one or more named cases failed", 1)
        for name, (cases, outcome) in parsed_entries.items():
            if outcome != "PASS" or any(actual != expected for actual, expected in cases):
                raise CandidateError(f"unexpected entry failure: {name}", 1)
    else:
        failed_case_ids = [
            case_id for case_id in expected_case_ids
            if parsed_case_results[case_id][2] == "FAIL"
        ]
        if failed_case_ids != [EXPECTED_NEGATIVE_CASE_ID]:
            raise CandidateError(
                f"expected only case {EXPECTED_NEGATIVE_CASE_ID} to fail, got {failed_case_ids}", 1
            )
        if parsed_case_results[EXPECTED_NEGATIVE_CASE_ID] != (23, 0, "FAIL"):
            raise CandidateError("expected case in_process_echo to report exit 23/expected0 FAIL", 1)
        failed_names = [name for name, (_, outcome) in parsed_entries.items() if outcome == "FAIL"]
        if failed_names != [EXPECTED_NEGATIVE_ENTRY]:
            raise CandidateError(
                f"expected only {EXPECTED_NEGATIVE_ENTRY} to fail, got {failed_names}", 1
            )
        target_cases, target_outcome = parsed_entries[EXPECTED_NEGATIVE_ENTRY]
        if target_outcome != "FAIL" or target_cases != [EXPECTED_NEGATIVE_CASE]:
            raise CandidateError(
                f"expected {EXPECTED_NEGATIVE_ENTRY} exit {EXPECTED_NEGATIVE_CASE[0]}, got {target_cases}",
                1,
            )
        for name, (cases, outcome) in parsed_entries.items():
            if name == EXPECTED_NEGATIVE_ENTRY:
                continue
            if outcome != "PASS" or any(actual != expected for actual, expected in cases):
                raise CandidateError(f"unexpected additional failure: {name}", 1)
    return {
        "result": "PASS",
        "task_root": str(task),
        "qemu_exit": supervisor.get("qemu_exit"),
        "runner_status": runner_status,
        "guest_pid1_status": guest_status,
        "entry_groups": {"pass": passed, "fail": failed},
        "cases": {"pass": case_passed, "fail": case_failed},
        "timed_out": False,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "log": str(log_path),
        "log_sha256": hashlib.sha256(log).hexdigest(),
    }


def make_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    build = commands.add_parser("build-fixtures")
    build.add_argument("--task-root", required=True)
    build.add_argument("--output-dir", required=True)
    build.add_argument("--bundle", required=True)
    build.add_argument("--runner-elf", required=True)
    build.add_argument("--fail-child-elf", required=True)
    build.add_argument("--runner-entry", default="bin/entry-runtime-runner-io-fixed")
    build.add_argument("--replace-entry", default="bin/in_process_echo")
    build.add_argument("--positive-name", default="runner-positive.cpio")
    build.add_argument("--negative-name", default="runner-exit23.cpio")

    run = commands.add_parser("run")
    run.add_argument("--task-root", required=True)
    run.add_argument("--output-dir", required=True)
    run.add_argument("--qemu", required=True)
    run.add_argument("--qemu-child-runner", required=True)
    run.add_argument("--kernel", required=True)
    run.add_argument("--initrd", required=True)
    run.add_argument("--rdinit", default="/bin/entry-runtime-runner-io-fixed")
    run.add_argument("--contract", choices=("fixture", "normal-init"), default="fixture")
    run.add_argument("--timeout-seconds", type=int, default=600)
    run.add_argument("--expected-runner-status", type=int, choices=(0, 1), default=None)
    run.add_argument("--expected-entries", type=int, default=9)
    run.add_argument("--expected-entry", action="append", default=None)
    run.add_argument("--ready-marker")
    run.add_argument("--log-name", default="qemu.log")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = make_parser()
    try:
        args = parser.parse_args(argv)
        if args.command == "build-fixtures":
            result = build_fixtures(args)
        else:
            result = run_guest(args)
        print(json.dumps(result, sort_keys=True))
        return 0
    except CandidateError as error:
        print(json.dumps({"result": error.result, "error": str(error)}, sort_keys=True), file=sys.stderr)
        return error.status
    except OSError as error:
        print(json.dumps({"result": "FAIL", "error": f"I/O error: {error}"}, sort_keys=True), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
