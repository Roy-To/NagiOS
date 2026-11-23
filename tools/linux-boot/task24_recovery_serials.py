#!/usr/bin/env python3
"""Validate Task24 save/read serial evidence for previous-launch recovery.

This pure host validator reads only the two supplied serial files. It does not
open a Data image, start QEMU, or inspect Notes body text. The recovery runner
must still validate Data identity, child exit status, and its C3 journal record.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

MOUNT = "NAGI_DEV_IDENTITY_DATA_MOUNTED retained_fd=true"
SESSION = "NAGI_DEV_TASK24_SESSION_ATTACHED"
HANDSHAKE = "NAGI_DEV_TASK24_IDENTITY_HANDSHAKE_COMPLETED"
READY = "nagi-init: state=ready"
INPUT = "NAGI_DEV_NOTES_INPUT_READY"
SAVED = "NAGI_DEV_NOTES_SAVED"
REOPENED = "NAGI_DEV_NOTES_REOPENED"
CLOSED = "NAGI_DEV_NOTES_CLOSED"
DESKTOP_READY = "NAGI_DEV_DESKTOP_READY"
DESKTOP_LAUNCH = "NAGI_DEV_DESKTOP_APP_LAUNCH app=notes"
DESKTOP_CLOSED = "NAGI_DEV_DESKTOP_APP_CLOSED app=notes exit=ClosedByUser"
DESKTOP_REISSUED = "NAGI_DEV_DESKTOP_NOTES_ATTACHMENT_REISSUED"
DESKTOP_LOGOUT = "NAGI_DEV_DESKTOP_LOGOUT_REQUESTED"
SESSION_LOGOUT = "NAGI_DEV_NOTES_SESSION_ENDED reason=UserLogout"
DESKTOP_NOTES_FLOW = {
    "save": (
        DESKTOP_READY, DESKTOP_LAUNCH, INPUT, SAVED, CLOSED, DESKTOP_CLOSED,
        DESKTOP_LAUNCH, DESKTOP_REISSUED, REOPENED, INPUT, CLOSED, DESKTOP_CLOSED,
        DESKTOP_LOGOUT, SESSION_LOGOUT,
    ),
    "read": (
        DESKTOP_READY, DESKTOP_LAUNCH, REOPENED, INPUT, CLOSED, DESKTOP_CLOSED,
        DESKTOP_LOGOUT, SESSION_LOGOUT,
    ),
}
REAPED = "NAGI_DEV_TASK24_CHILD_REAPED_AND_RETIRED"
REAPED_NORMAL = (
    "NAGI_DEV_TASK24_CHILD_REAPED_AND_RETIRED "
    "status=ServiceExitStatus { exit_code: Some(0), signal: None }"
)
IDENTITY_EXIT_REQUESTED = "NAGI_DEV_TASK24_IDENTITY_EXIT_REQUESTED"
C3_IDENTITY_NORMAL_EXIT_REAPED = "NAGI_DEV_TASK24_C3_IDENTITY_NORMAL_EXIT_REAPED"
CURRENT_TERMINAL = "NAGI_DEV_TASK24_IDENTITY_TERMINAL_CONSUMED"
PREVIOUS_TERMINAL = "NAGI_DEV_TASK24_PREVIOUS_IDENTITY_TERMINAL_CONSUMED"
RECEIPT_FIELDS = (
    "service",
    "catalog_generation",
    "c3_launch_generation",
    "launch_instance_id",
    "process_id",
    "c1_operation_id",
    "terminal_revision",
    "terminal_operation_id",
)
RECEIPT_VALUES = r" service=([A-Za-z0-9._-]+)" + "".join(
    rf" {field}=([1-9][0-9]*)" for field in RECEIPT_FIELDS[1:]
)
LOGIN_WAIT = "NAGI_DEV_TASK24_UNLOCK_WAITING_FOR_INPUT"
LOGIN_CONNECTED = "NAGI_DEV_TASK24_CHILD_BROKER_CONNECTED"
LOGIN_DENIED = "NAGI_DEV_TASK24_ACCOUNT_AUTHENTICATION_DENIED"
LOGIN_REJECTED = f"{LOGIN_DENIED} (Rejected)"
LOGIN_CANCELLED = "NAGI_DEV_TASK24_LOGIN_CANCELLED_NORMALLY"
FAILURES = (
    "NAGI_C1_BOOT_RECOVERY_UNAVAILABLE",
    "C1_BOOT_RECOVERY_UNAVAILABLE",
    "NAGI_DEV_TASK24_LOGIN_UNAVAILABLE",
    "NAGI_DEV_TASK24_LOGIN_STOPPED",
    "NAGI_DEV_TASK24_IDENTITY_TERMINAL_UNAVAILABLE",
    "NAGI_DEV_TASK24_REQUIRED_IDENTITY_MONITOR_UNAVAILABLE",
    "NAGI_DEV_TASK24_CLEANUP_PENDING",
)
NOTE_PATTERNS = {
    SAVED: re.compile(r"^NAGI_DEV_NOTES_SAVED object=([0-9a-f]{16}) revision=([1-9][0-9]*)$"),
    REOPENED: re.compile(r"^NAGI_DEV_NOTES_REOPENED object=([0-9a-f]{16}) revision=([1-9][0-9]*)$"),
}


def _complete_lines(serial: str, phase: str) -> list[str]:
    if not serial.endswith(("\n", "\r")):
        raise ValueError(f"{phase} serial ends with a partial line")
    lines = serial.splitlines()
    if any(line.startswith(FAILURES) for line in lines):
        raise ValueError(f"{phase} serial contains a recovery or lifecycle failure")
    return lines


def _unique_index(lines: list[str], marker: str, phase: str, *, prefix: bool = False) -> int:
    matches = [
        index
        for index, line in enumerate(lines)
        if (line.startswith(marker) if prefix else line == marker)
    ]
    if len(matches) != 1:
        raise ValueError(f"{phase} requires exactly one {marker!r} line")
    return matches[0]


def _receipt_identity(
    lines: list[str], marker: str, phase: str, *, required: bool = True
) -> tuple[tuple[str | int, ...], int] | None:
    matches = [
        (index, line)
        for index, line in enumerate(lines)
        if line == marker or line.startswith(f"{marker} ")
    ]
    if len(matches) > 1 or (required and not matches):
        raise ValueError(f"{phase} requires exactly one {marker!r} receipt line")
    if not matches:
        if any(line == marker for line in lines):
            raise ValueError(f"{phase} has a receipt marker without its identity fields")
        return None
    index, line = matches[0]
    match = re.fullmatch(re.escape(marker) + RECEIPT_VALUES, line)
    if match is None:
        raise ValueError(f"{phase} has a malformed {marker!r} receipt line")
    values: tuple[str | int, ...] = (match.group(1),) + tuple(
        int(value) for value in match.groups()[1:]
    )
    return values, index


def _unique_normal_reap_index(lines: list[str], phase: str) -> int:
    matches = [
        (index, line)
        for index, line in enumerate(lines)
        if line.startswith(REAPED)
    ]
    if len(matches) != 1 or matches[0][1] != REAPED_NORMAL:
        raise ValueError(f"{phase} requires exactly one normal child reap line")
    return matches[0][0]


def _note_identity(lines: list[str], marker: str, phase: str) -> tuple[str, int, int]:
    pattern = NOTE_PATTERNS[marker]
    candidates = [
        (index, line)
        for index, line in enumerate(lines)
        if line.startswith(f"{marker} ")
    ]
    if len(candidates) != 1:
        raise ValueError(f"{phase} requires exactly one well-formed {marker} line")
    index, line = candidates[0]
    match = pattern.fullmatch(line)
    if match is None:
        raise ValueError(f"{phase} requires exactly one well-formed {marker} line")
    return match.group(1), int(match.group(2)), index


def uses_desktop(lines: list[str]) -> bool:
    return any(line.startswith("NAGI_DEV_DESKTOP_") for line in lines)


def desktop_notes_progress(lines: list[str], phase: str) -> int:
    """Check the observed prefix of the same-session launcher and Notes flow."""
    expected = DESKTOP_NOTES_FLOW[phase]
    progress = 0
    saved = None
    for line in lines:
        marker, _, _ = line.partition(" ")
        if marker in NOTE_PATTERNS:
            match = NOTE_PATTERNS[marker].fullmatch(line)
            if match is None:
                raise ValueError(f"{phase} has a malformed {marker} line")
            if marker == SAVED:
                saved = match.groups()
            elif phase == "save" and saved != match.groups():
                raise ValueError("same-session reopen differs from saved Notes ObjectId/revision")
        elif line in (INPUT, CLOSED, SESSION_LOGOUT):
            marker = line
        elif line.startswith("NAGI_DEV_DESKTOP_"):
            marker = line
        else:
            continue
        if progress == len(expected) or marker != expected[progress]:
            raise ValueError(f"{phase} has an unexpected or out-of-order desktop marker: {marker}")
        progress += 1
    return progress


def _validate_phase(
    serial: str, phase: str, note_marker: str
) -> tuple[str, int, tuple[str | int, ...]]:
    lines = _complete_lines(serial, phase)
    mount = _unique_index(lines, MOUNT, phase)
    session = _unique_index(lines, SESSION, phase)
    handshake = _unique_index(lines, HANDSHAKE, phase)
    ready = _unique_index(lines, READY, phase)
    desktop = uses_desktop(lines)
    note_id, revision, note_index = _note_identity(lines, note_marker, phase)
    if desktop:
        if desktop_notes_progress(lines, phase) != len(DESKTOP_NOTES_FLOW[phase]):
            raise ValueError(f"{phase} desktop Notes flow did not finish")
        first_action = _unique_index(lines, DESKTOP_READY, phase)
        last_action = _unique_index(lines, SESSION_LOGOUT, phase)
    else:
        input_ready = _unique_index(lines, INPUT, phase)
        last_action = _unique_index(lines, CLOSED, phase)
        first_action = input_ready if note_marker == SAVED else note_index
        if note_marker == SAVED:
            legacy_order = input_ready < note_index < last_action
        else:
            legacy_order = note_index < input_ready < last_action
        if not legacy_order:
            raise ValueError(f"{phase} Notes input/save/read/close order is invalid")
    reaped = _unique_normal_reap_index(lines, phase)
    identity_exit_requested = _unique_index(lines, IDENTITY_EXIT_REQUESTED, phase)
    c3_identity = _receipt_identity(lines, C3_IDENTITY_NORMAL_EXIT_REAPED, phase)
    terminal_identity = _receipt_identity(lines, CURRENT_TERMINAL, phase)
    assert c3_identity is not None and terminal_identity is not None
    c3_receipt, c3_identity_reaped = c3_identity
    consumed_receipt, terminal = terminal_identity
    if c3_receipt != consumed_receipt:
        raise ValueError(f"{phase} C3 normal-exit and C1-consumed receipts differ")

    if not mount < session < handshake < ready:
        raise ValueError(f"{phase} mount/session/handshake/READY order is invalid")
    if not ready < first_action < last_action < reaped < identity_exit_requested < c3_identity_reaped < terminal:
        raise ValueError(f"{phase} Notes action/reap/Identity/C1 terminal order is invalid")
    if note_marker == SAVED:
        previous_terminal = _receipt_identity(
            lines, PREVIOUS_TERMINAL, phase, required=False
        )
        if not desktop and any(line.startswith(f"{REOPENED} ") for line in lines):
            raise ValueError("save phase must start from a fresh Notes volume")
        if previous_terminal is not None and not mount < previous_terminal[1] < session:
            raise ValueError("save previous-terminal marker is out of order")
    elif any(line.startswith(f"{SAVED} ") for line in lines):
        raise ValueError("read phase must not emit a saved marker")
    return note_id, revision, c3_receipt


def _validate_read_link(
    read_serial: str, expected_receipt: tuple[str | int, ...]
) -> tuple[str, int]:
    read_lines = _complete_lines(read_serial, "read")
    mount = _unique_index(read_lines, MOUNT, "read")
    previous_terminal_identity = _receipt_identity(read_lines, PREVIOUS_TERMINAL, "read")
    assert previous_terminal_identity is not None
    previous_receipt, previous_terminal = previous_terminal_identity
    if previous_receipt != expected_receipt:
        raise ValueError("read boot consumed a different C3 terminal receipt than the preceding boot")
    login_wait = _unique_index(read_lines, LOGIN_WAIT, "read")
    session = _unique_index(read_lines, SESSION, "read")
    handshake = _unique_index(read_lines, HANDSHAKE, "read")
    ready = _unique_index(read_lines, READY, "read")
    _, _, reopened_index = _note_identity(read_lines, REOPENED, "read")
    input_ready = _unique_index(read_lines, INPUT, "read")
    if not mount < previous_terminal < login_wait < session < handshake < ready:
        raise ValueError("read previous-receipt/login/READY order is invalid")
    if not ready < reopened_index < input_ready:
        raise ValueError("read reopen/input order is invalid")

    reopened_id, reopened_revision, _ = _validate_phase(read_serial, "read", REOPENED)
    return reopened_id, reopened_revision


def _require_matching_entrypoints(save_serial: str, read_serial: str) -> None:
    if uses_desktop(save_serial.splitlines()) != uses_desktop(read_serial.splitlines()):
        raise ValueError("save and read boots used different Notes entry points")


def validate_save_read(save_serial: str, read_serial: str) -> dict[str, int | str]:
    """Require prior terminal consumption before login, then exact note identity."""
    _require_matching_entrypoints(save_serial, read_serial)
    saved_id, saved_revision, saved_receipt = _validate_phase(save_serial, "save", SAVED)
    reopened_id, reopened_revision = _validate_read_link(read_serial, saved_receipt)
    if (saved_id, saved_revision) != (reopened_id, reopened_revision):
        raise ValueError("saved and reopened Notes ObjectId/revision differ")
    return {"object": saved_id, "revision": saved_revision, "ready_count": 1}


def validate_reject_close(
    serial: str, expected_receipt: tuple[str | int, ...],
) -> tuple[str | int, ...]:
    phase = "reject-close"
    lines = _complete_lines(serial, phase)
    forbidden = (
        "NAGI_DEV_TASK24_ACCOUNT_AUTHENTICATED", "NAGI_DEV_TASK24_C1_SESSION_COMMITTED",
        "NAGI_DEV_TASK24_LOGIN_ATTEMPT_EXPIRED", "NAGI_DEV_TASK24_INITIAL_ADMISSION_FAILED",
        "NAGI_DEV_TASK24_LOGIN_THROTTLE_PERSISTENCE_FAILED",
        "NAGI_LINUX_OWNER_REGISTRATION_WAITING_FOR_INPUT", "NAGI_DEV_NOTES_",
        SESSION, HANDSHAKE, READY, "NAGI_DEV_DESKTOP_",
    )
    if any(line.startswith(forbidden) for line in lines):
        raise ValueError("reject-close boot entered a forbidden state")
    waits = [i for i, line in enumerate(lines) if line == LOGIN_WAIT]
    denials = [(i, line) for i, line in enumerate(lines) if line.startswith(LOGIN_DENIED)]
    if len(waits) != 2 or len(denials) != 1 or denials[0][1] != LOGIN_REJECTED:
        raise ValueError("reject-close requires one confirmed rejection and two login waits")
    mount = _unique_index(lines, MOUNT, phase)
    connected = _unique_index(lines, LOGIN_CONNECTED, phase)
    cancelled = _unique_index(lines, LOGIN_CANCELLED, phase)
    exit_requested = _unique_index(lines, IDENTITY_EXIT_REQUESTED, phase)
    previous = _receipt_identity(lines, PREVIOUS_TERMINAL, phase)
    normal_exit = _receipt_identity(lines, C3_IDENTITY_NORMAL_EXIT_REAPED, phase)
    terminal = _receipt_identity(lines, CURRENT_TERMINAL, phase)
    assert previous is not None and normal_exit is not None and terminal is not None
    if previous[0] != expected_receipt:
        raise ValueError("reject-close boot consumed a different C3 terminal receipt than save boot")
    if normal_exit[0] != terminal[0] or terminal[0] == previous[0]:
        raise ValueError("reject-close did not consume its own matching C3 terminal receipt")
    if not (
        mount < previous[1] < waits[0] < connected < denials[0][0] < waits[1]
        < cancelled < exit_requested < normal_exit[1] < terminal[1]
    ):
        raise ValueError("reject-close login, retry, cancellation or receipt order is invalid")
    return terminal[0]


def validate_save_reject_read(
    save_serial: str, reject_serial: str, read_serial: str
) -> dict[str, int | str]:
    _require_matching_entrypoints(save_serial, read_serial)
    saved_id, saved_revision, saved_receipt = _validate_phase(save_serial, "save", SAVED)
    rejected_receipt = validate_reject_close(reject_serial, saved_receipt)
    reopened_id, reopened_revision = _validate_read_link(read_serial, rejected_receipt)
    if (saved_id, saved_revision) != (reopened_id, reopened_revision):
        raise ValueError("saved and reopened Notes ObjectId/revision differ")
    return {"object": saved_id, "revision": saved_revision, "ready_count": 1, "reject_ready_count": 0}


def _serial(lines: list[str]) -> str:
    return "\n".join(lines) + "\n"


def _receipt_line(marker: str, *, generation: int, launch: int, pid: int, c1: int, revision: int, terminal: int) -> str:
    return (
        f"{marker} service=org.nagi.identity catalog_generation=4 "
        f"c3_launch_generation={generation} launch_instance_id={launch} process_id={pid} "
        f"c1_operation_id={c1} terminal_revision={revision} terminal_operation_id={terminal}"
    )


def self_check() -> None:
    note_id = "1234567890abcdef"
    save = _serial(
        [
            MOUNT,
            SESSION,
            HANDSHAKE,
            READY,
            INPUT,
            f"{SAVED} object={note_id} revision=2",
            CLOSED,
            REAPED_NORMAL,
            IDENTITY_EXIT_REQUESTED,
            _receipt_line(C3_IDENTITY_NORMAL_EXIT_REAPED, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61),
            _receipt_line(CURRENT_TERMINAL, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61),
        ]
    )
    read = _serial(
        [
            MOUNT,
            _receipt_line(PREVIOUS_TERMINAL, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61),
            LOGIN_WAIT,
            SESSION,
            HANDSHAKE,
            READY,
            f"{REOPENED} object={note_id} revision=2",
            INPUT,
            CLOSED,
            REAPED_NORMAL,
            IDENTITY_EXIT_REQUESTED,
            _receipt_line(C3_IDENTITY_NORMAL_EXIT_REAPED, generation=4, launch=62, pid=502, c1=73, revision=9, terminal=74),
            _receipt_line(CURRENT_TERMINAL, generation=4, launch=62, pid=502, c1=73, revision=9, terminal=74),
        ]
    )
    expected = {"object": note_id, "revision": 2, "ready_count": 1}
    assert validate_save_read(save, read) == expected

    read_lines = read.splitlines()
    previous_line = read_lines.pop(1)
    late_previous = _serial(read_lines[:2] + [previous_line] + read_lines[2:])
    duplicate_previous = _serial(
        read.splitlines()[:2] + [previous_line] + read.splitlines()[2:]
    )
    wrong_exit = read.replace(REAPED_NORMAL, f"{REAPED} status=ServiceExitStatus {{ exit_code: Some(1), signal: None }}")
    duplicate_note = read.replace(
        f"{REOPENED} object={note_id} revision=2",
        f"{REOPENED} object={note_id} revision=2\n{REOPENED} object={note_id} revision=2",
    )
    read_saved = read.replace(
        f"{REOPENED} object={note_id} revision=2",
        f"{SAVED} object={note_id} revision=2\n{REOPENED} object={note_id} revision=2",
    )
    duplicate_reap = read.replace(
        REAPED_NORMAL,
        f"{REAPED_NORMAL}\n{REAPED} status=ServiceExitStatus {{ exit_code: Some(1), signal: None }}",
    )
    duplicate_terminal = read.replace(
        CURRENT_TERMINAL,
        f"{CURRENT_TERMINAL}\n{CURRENT_TERMINAL} unexpected",
    )
    unordered_login = read.replace(
        f"{LOGIN_WAIT}\n{SESSION}\n{HANDSHAKE}\n{READY}",
        f"{SESSION}\n{LOGIN_WAIT}\n{HANDSHAKE}\n{READY}",
    )
    missing_c3_normal_reap = _serial(
        [line for line in save.splitlines() if not line.startswith(C3_IDENTITY_NORMAL_EXIT_REAPED)]
    )
    c1_before_c3_reap = save.replace(
        f"{_receipt_line(C3_IDENTITY_NORMAL_EXIT_REAPED, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61)}\n{_receipt_line(CURRENT_TERMINAL, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61)}",
        f"{_receipt_line(CURRENT_TERMINAL, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61)}\n{_receipt_line(C3_IDENTITY_NORMAL_EXIT_REAPED, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61)}",
    )
    duplicate_identity_exit = save.replace(
        IDENTITY_EXIT_REQUESTED,
        f"{IDENTITY_EXIT_REQUESTED}\n{IDENTITY_EXIT_REQUESTED}",
    )
    duplicate_c3_reap = save.replace(
        _receipt_line(C3_IDENTITY_NORMAL_EXIT_REAPED, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61),
        f"{_receipt_line(C3_IDENTITY_NORMAL_EXIT_REAPED, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61)}\n{_receipt_line(C3_IDENTITY_NORMAL_EXIT_REAPED, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61)}",
    )
    c3_reap_before_exit_request = save.replace(
        f"{IDENTITY_EXIT_REQUESTED}\n{_receipt_line(C3_IDENTITY_NORMAL_EXIT_REAPED, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61)}",
        f"{_receipt_line(C3_IDENTITY_NORMAL_EXIT_REAPED, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61)}\n{IDENTITY_EXIT_REQUESTED}",
    )
    saved_line = f"{SAVED} object={note_id} revision=2"
    save_missing_note = _serial(
        [line for line in save.splitlines() if line != saved_line]
    )
    save_malformed_note_id = save.replace(
        saved_line, f"{SAVED} object=not-an-object-id revision=2"
    )
    save_zero_note_revision = save.replace(
        saved_line, f"{SAVED} object={note_id} revision=0"
    )
    save_duplicate_note = save.replace(
        saved_line, f"{saved_line}\n{saved_line}"
    )
    save_note_before_input = save.replace(
        f"{INPUT}\n{saved_line}", f"{saved_line}\n{INPUT}"
    )
    mismatched_previous_receipt = read.replace(
        _receipt_line(PREVIOUS_TERMINAL, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61),
        _receipt_line(PREVIOUS_TERMINAL, generation=3, launch=42, pid=501, c1=52, revision=8, terminal=61),
    )
    save_c1_receipt_mismatch = save.replace(
        _receipt_line(CURRENT_TERMINAL, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=61),
        _receipt_line(CURRENT_TERMINAL, generation=3, launch=41, pid=501, c1=52, revision=8, terminal=62),
    )
    cases = (
        _serial([line for line in read.splitlines() if not line.startswith(PREVIOUS_TERMINAL)]),
        duplicate_previous,
        late_previous,
        read.replace(f"object={note_id} revision=2", "object=fedcba0987654321 revision=2"),
        read.replace(f"object={note_id} revision=2", f"object={note_id} revision=3"),
        duplicate_note,
        read.replace(_receipt_line(CURRENT_TERMINAL, generation=4, launch=62, pid=502, c1=73, revision=9, terminal=74), "NAGI_C1_BOOT_RECOVERY_UNAVAILABLE (StoreUnavailable)"),
        wrong_exit,
        read_saved,
        duplicate_reap,
        duplicate_terminal,
        unordered_login,
        mismatched_previous_receipt,
    )
    for case_number, broken in enumerate(cases, start=1):
        try:
            validate_save_read(save, broken)
        except ValueError:
            continue
        raise AssertionError(f"synthetic recovery serial false positive in read case {case_number}")

    save_cases = (
        missing_c3_normal_reap,
        c1_before_c3_reap,
        duplicate_identity_exit,
        duplicate_c3_reap,
        c3_reap_before_exit_request,
        save_c1_receipt_mismatch,
        save_missing_note,
        save_malformed_note_id,
        save_zero_note_revision,
        save_duplicate_note,
        save_note_before_input,
    )
    for case_number, broken_save in enumerate(save_cases, start=1):
        try:
            _validate_phase(broken_save, "save", SAVED)
        except ValueError:
            continue
        raise AssertionError(f"synthetic save serial false positive in save case {case_number}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--save-serial", type=Path)
    parser.add_argument("--read-serial", type=Path)
    parser.add_argument("--self-check", action="store_true")
    args = parser.parse_args()
    if args.self_check:
        if args.save_serial is not None or args.read_serial is not None:
            parser.error("--self-check cannot be combined with serial paths")
        self_check()
        print("PASS previous-receipt order and exact save/read ObjectId/revision")
        return
    if args.save_serial is None or args.read_serial is None:
        parser.error("provide --save-serial and --read-serial, or --self-check")
    result = validate_save_read(
        args.save_serial.read_text(encoding="utf-8", errors="replace"),
        args.read_serial.read_text(encoding="utf-8", errors="replace"),
    )
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
