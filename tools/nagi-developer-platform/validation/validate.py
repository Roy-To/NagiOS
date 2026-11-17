#!/usr/bin/env python3
"""NDP Stage 1 adapter for the existing nagi-pkg Manifest validator."""
import argparse
import json
import os
from pathlib import Path
import re
import selectors
import signal
import stat
import subprocess
import sys
import tempfile
import time

REPO = Path(__file__).resolve().parents[3]
VALIDATOR = REPO / "out/ndp-host/target/debug/nagi-pkg"
MAX_INPUT = 1_048_576
MAX_OUTPUT = 65_536
SUCCESS = re.compile(rb"PASS nagi-pkg manifest validate: manifest\.json AppId=([0-9]+) sdk=([0-9]+)\.([0-9]+)\r?\n")


class InputError(Exception):
    pass


def read_manifest(root, relative):
    """Read one bounded regular file through no-follow directory descriptors."""
    path = Path(relative)
    if path.is_absolute() or not path.parts or ".." in path.parts:
        raise InputError("unsafe_path")
    if not all(hasattr(os, flag) for flag in ("O_DIRECTORY", "O_NOFOLLOW", "O_NONBLOCK")):
        raise InputError("unsupported_host")
    directory = None
    try:
        directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        for part in path.parts[:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
            os.close(directory)
            directory = child
        descriptor = os.open(path.parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                             dir_fd=directory)
        with os.fdopen(descriptor, "rb") as source:
            if not stat.S_ISREG(os.fstat(source.fileno()).st_mode):
                raise InputError("not_regular_file")
            data = source.read(MAX_INPUT + 1)
        if len(data) > MAX_INPUT:
            raise InputError("input_too_large")
        return data
    except (OSError, ValueError):
        raise InputError("input_unavailable") from None
    finally:
        if directory is not None:
            os.close(directory)


def _kill_group(process):
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def _run(command, cwd, timeout, cancel=None, output_limit=MAX_OUTPUT):
    """Bounded process transport; arbitrary test commands are internal test inputs only."""
    if cancel is not None and cancel.is_set():
        return {"status": "cancelled", "exit_code": None, "stdout": b"", "stderr": b""}
    try:
        process = subprocess.Popen(command, cwd=cwd, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   start_new_session=True)
    except OSError:
        return {"status": "unavailable", "exit_code": None, "stdout": b"", "stderr": b""}
    streams = {"stdout": bytearray(), "stderr": bytearray()}
    remaining = output_limit
    deadline = time.monotonic() + timeout
    status = "exited"
    selector = selectors.DefaultSelector()
    try:
        for name in streams:
            selector.register(getattr(process, name), selectors.EVENT_READ, name)
        while selector.get_map() or process.poll() is None:
            if cancel is not None and cancel.is_set():
                status = "cancelled"
                break
            if time.monotonic() >= deadline:
                status = "timeout"
                break
            for key, _ in selector.select(min(0.05, max(0, deadline - time.monotonic()))):
                chunk = os.read(key.fd, 4096)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                if len(chunk) > remaining:
                    streams[key.data].extend(chunk[:remaining])
                    status = "output_limit"
                    break
                streams[key.data].extend(chunk)
                remaining -= len(chunk)
            if status != "exited":
                break
    except OSError:
        status = "io_error"
    finally:
        _kill_group(process)
        process.wait()
        selector.close()
        process.stdout.close()
        process.stderr.close()
    return {"status": status, "exit_code": process.returncode,
            **{key: bytes(value) for key, value in streams.items()}}


def validate(root, relative, stage=1, timeout=10, cancel=None):
    result = {"stage": stage, "check": "manifest", "scope": "host_metadata_only",
              "status": "not_run", "reason": "stage_not_connected", "exit_code": None}
    if stage != 1:
        return result
    if not 0 < timeout <= 60:
        return {**result, "status": "failed", "reason": "invalid_timeout"}
    if cancel is not None and cancel.is_set():
        return {**result, "status": "cancelled", "reason": "cancelled"}
    try:
        data = read_manifest(root, relative)
    except InputError as error:
        return {**result, "status": "failed", "reason": str(error)}
    if VALIDATOR.is_symlink() or not VALIDATOR.is_file() or not os.access(VALIDATOR, os.X_OK):
        return {**result, "status": "unavailable", "reason": "validator_unavailable"}
    try:
        with tempfile.TemporaryDirectory(prefix="nagi-ndp-validate-") as directory:
            # Snapshot the selected bytes. The validator never reopens the original path.
            (Path(directory) / "manifest.json").write_bytes(data)
            execution = _run([str(VALIDATOR), "manifest", "validate", "manifest.json"],
                             directory, timeout, cancel)
    except OSError:
        return {**result, "status": "unavailable", "reason": "scratch_unavailable"}
    result.update(exit_code=execution["exit_code"],
                  captured_output_bytes=len(execution["stdout"]) + len(execution["stderr"]))
    if execution["status"] != "exited":
        return {**result, "status": execution["status"], "reason": execution["status"]}
    if execution["exit_code"] != 0:
        reason = "validator_rejected" if execution["exit_code"] == 2 else "validator_failed"
        return {**result, "status": "failed", "reason": reason}
    match = SUCCESS.fullmatch(execution["stdout"])
    if match is None or execution["stderr"]:
        return {**result, "status": "failed", "reason": "invalid_tool_output"}
    return {**result, "status": "passed", "reason": "metadata_valid",
            "app_id": match[1].decode(), "sdk_contract": [int(match[2]), int(match[3])]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, help="Explicit allowed input directory")
    parser.add_argument("--stage", type=int, choices=(1, 2, 3), default=1)
    parser.add_argument("--timeout", type=float, default=10)
    parser.add_argument("manifest", help="One relative Manifest path")
    args = parser.parse_args(argv)
    try:
        result = validate(args.root, args.manifest, args.stage, args.timeout)
    except KeyboardInterrupt:
        result = {"status": "cancelled", "reason": "cancelled"}
    print(json.dumps(result, sort_keys=True))
    return {"passed": 0, "unavailable": 2, "not_run": 3, "timeout": 124, "cancelled": 130}.get(result["status"], 1)


if __name__ == "__main__":
    sys.exit(main())
