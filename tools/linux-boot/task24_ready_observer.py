#!/usr/bin/env python3
"""Report Task24 Identity/PID 1 READY counts without echoing serial contents."""

from __future__ import annotations

import argparse
import json
import re
import stat
import sys
from pathlib import Path


IDENTITY_READY = "NAGI_DEV_TASK24_IDENTITY_HANDSHAKE_COMPLETED"
PID1_READY = "nagi-init: state=ready"
LOGIN_WAIT = "NAGI_DEV_TASK24_UNLOCK_WAITING_FOR_INPUT"
AUTH_DENIED_PREFIX = "NAGI_DEV_TASK24_ACCOUNT_AUTHENTICATION_DENIED"
AUTHENTICATED_PREFIX = "NAGI_DEV_TASK24_ACCOUNT_AUTHENTICATED"
MAX_SERIAL_BYTES = 16 * 1024 * 1024


class ObservationError(ValueError):
    """Serial evidence does not satisfy the requested READY case."""


def _complete_lines(serial: str, case: str) -> list[str]:
    if len(serial.encode("utf-8")) > MAX_SERIAL_BYTES:
        raise ObservationError(f"{case} serial exceeds the 16 MiB limit")
    if not serial.endswith(("\n", "\r")):
        raise ObservationError(f"{case} serial ends with a partial line")
    return serial.splitlines()


def _line_numbers(lines: list[str], marker: str) -> list[int]:
    return [number for number, line in enumerate(lines, start=1) if line == marker]


def validate_success_serial(serial: str, case: str) -> dict[str, int | str]:
    if not re.fullmatch(r"[a-z][a-z0-9_-]{0,31}", case):
        raise ObservationError("success case label is not canonical")
    lines = _complete_lines(serial, case)
    identity_lines = _line_numbers(lines, IDENTITY_READY)
    pid1_lines = _line_numbers(lines, PID1_READY)
    if len(identity_lines) != 1 or len(pid1_lines) != 1:
        raise ObservationError(f"{case} requires exactly one Identity READY and PID 1 READY marker")
    if identity_lines[0] >= pid1_lines[0]:
        raise ObservationError(f"{case} PID 1 READY does not follow Identity READY")
    return {
        "case": case,
        "identity_ready_count": len(identity_lines),
        "identity_ready_line": identity_lines[0],
        "pid1_ready_count": len(pid1_lines),
        "pid1_ready_line": pid1_lines[0],
    }


def validate_rejection_serial(serial: str) -> dict[str, int | str]:
    lines = _complete_lines(serial, "rejection")
    identity_lines = _line_numbers(lines, IDENTITY_READY)
    pid1_lines = _line_numbers(lines, PID1_READY)
    login_wait_lines = _line_numbers(lines, LOGIN_WAIT)
    denial_lines = [
        number
        for number, line in enumerate(lines, start=1)
        if line.startswith(AUTH_DENIED_PREFIX + " ")
    ]
    authenticated_lines = [
        number
        for number, line in enumerate(lines, start=1)
        if line.startswith(AUTHENTICATED_PREFIX)
    ]
    if len(denial_lines) != 1:
        raise ObservationError("rejection requires exactly one account-denied marker")
    if not login_wait_lines or login_wait_lines[0] >= denial_lines[0]:
        raise ObservationError("rejection marker does not follow the login input-wait marker")
    if identity_lines or pid1_lines:
        raise ObservationError("rejected authentication must have zero Identity/PID 1 READY markers")
    if authenticated_lines:
        raise ObservationError("rejected authentication also emitted an authenticated marker")
    return {
        "case": "rejection",
        "login_wait_count": len(login_wait_lines),
        "account_denied_count": len(denial_lines),
        "account_denied_line": denial_lines[0],
        "authenticated_count": len(authenticated_lines),
        "identity_ready_count": len(identity_lines),
        "pid1_ready_count": len(pid1_lines),
    }


def _read_serial(path: Path, label: str) -> str:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise ObservationError(f"{label} serial is unavailable ({error.__class__.__name__})") from None
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ObservationError(f"{label} serial must be a regular non-symlink file")
    if metadata.st_size > MAX_SERIAL_BYTES:
        raise ObservationError(f"{label} serial exceeds the 16 MiB limit")
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise ObservationError(f"{label} serial could not be read ({error.__class__.__name__})") from None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--success",
        action="append",
        default=[],
        metavar="LABEL=PATH",
        help="successful save/read serial to count both READY markers",
    )
    parser.add_argument("--rejection", type=Path, help="authentication-rejection serial")
    args = parser.parse_args(argv)
    if not args.success and args.rejection is None:
        parser.error("at least one --success or --rejection input is required")
    results = []
    try:
        for item in args.success:
            if "=" not in item:
                raise ObservationError("--success must use LABEL=PATH")
            label, raw_path = item.split("=", 1)
            results.append(validate_success_serial(_read_serial(Path(raw_path), label), label))
        if args.rejection is not None:
            results.append(validate_rejection_serial(_read_serial(args.rejection, "rejection")))
    except ObservationError as error:
        print(f"FAIL {error}", file=sys.stderr)
        return 1
    print(json.dumps({"schema": "nagi-task24-ready-observation/v1", "cases": results}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
