#!/usr/bin/env python3
"""Check ordered WS-10 markers from dedicated Task24 development boot logs."""

import argparse
from pathlib import Path

import task24_recovery_serials as recovery

READY = "nagi-init: state=ready"
WAITING = "NAGI_DEV_TASK24_UNLOCK_WAITING_FOR_INPUT"
ATTACHED = "NAGI_DEV_TASK24_SESSION_ATTACHED"
NOTES_READY = "NAGI_DEV_NOTES_INPUT_READY"
CLOSED = "NAGI_DEV_NOTES_CLOSED"
LOGOUT = "NAGI_DEV_NOTES_SESSION_ENDED reason=UserLogout"
PREVIOUS_LOGOUT = "NAGI_DEV_WS10_PREVIOUS_SESSION_ENDED reason=UserLogout"
PREVIOUS_TERMINAL = "NAGI_DEV_TASK24_PREVIOUS_IDENTITY_TERMINAL_CONSUMED "
NORMAL_REAP = "NAGI_DEV_TASK24_C3_IDENTITY_NORMAL_EXIT_REAPED "
NORMAL_TERMINAL = "NAGI_DEV_TASK24_IDENTITY_TERMINAL_CONSUMED "
ABNORMAL_RECOVERY = "NAGI_DEV_WS10_PRIOR_ABNORMAL_RECOVERY_CONFIRMED"
RETIRED = "NAGI_DEV_TASK24_CHILD_REAPED_AND_RETIRED "
STOPPED = "NAGI_DEV_WS10_IDENTITY_REAPED signal=SIGKILL"
REVOKED = "NAGI_DEV_WS10_SESSION_REVOKED reason=Revoked"
ENDED_REVOKED = "NAGI_DEV_NOTES_SESSION_ENDED reason=Revoked"
SAVE_FAILED = "NAGI_DEV_NOTES_SAVE_FAILED event=28 "
UNSAVED = "NAGI_DEV_NOTES_CLOSED_UNSAVED"
SAVED = "NAGI_DEV_NOTES_SAVED "
LIMIT = 4 * 1024 * 1024


def positions(lines, marker):
    return [
        i for i, line in enumerate(lines)
        if (line.startswith(marker) if marker.endswith(" ") else line == marker)
    ]


def ordered(lines, *markers):
    last = -1
    for marker in markers:
        found = positions(lines, marker)
        if len(found) != 1 or found[0] <= last:
            raise ValueError(f"missing, repeated or out-of-order marker: {marker.strip()}")
        last = found[0]


def require_absent(lines, *markers):
    for marker in markers:
        if positions(lines, marker):
            raise ValueError(f"unexpected marker: {marker.strip()}")


def notes_entry(lines):
    launcher = (recovery.DESKTOP_READY, recovery.DESKTOP_LAUNCH) if recovery.uses_desktop(lines) else ()
    return (ATTACHED, READY, *launcher, NOTES_READY)


def validate(case, first, reopened=None):
    entry = notes_entry(first)
    if reopened is not None and entry != notes_entry(reopened):
        raise ValueError("WS-10 boots used different Notes entry points")
    if case == "logout":
        desktop_close = (recovery.DESKTOP_CLOSED, recovery.DESKTOP_LOGOUT) if recovery.uses_desktop(first) else ()
        ordered(first, *entry, CLOSED, *desktop_close, LOGOUT, RETIRED)
        ordered(reopened, PREVIOUS_LOGOUT, WAITING, *entry)
    elif case == "stop":
        ordered(first, *entry, STOPPED, RETIRED)
        require_absent(first, LOGOUT, recovery.DESKTOP_LOGOUT, NORMAL_REAP, NORMAL_TERMINAL)
        require_absent(reopened, PREVIOUS_TERMINAL)
        ordered(reopened, ABNORMAL_RECOVERY, WAITING, *entry)
    elif case == "revoke":
        ordered(first, *entry, REVOKED, SAVE_FAILED, UNSAVED, ENDED_REVOKED, RETIRED)
        require_absent(first, CLOSED, LOGOUT, recovery.DESKTOP_LOGOUT)
        revoked_at, = positions(first, REVOKED)
        require_absent(first[revoked_at:], SAVED)
    else:
        raise ValueError("unknown WS-10 case")


def serial(path):
    with path.open("rb") as stream:
        data = stream.read(LIMIT + 1)
    if len(data) > LIMIT:
        raise ValueError("serial log exceeds the WS-10 size limit")
    return data.decode("utf-8", errors="replace").splitlines()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("case", choices=("logout", "stop", "revoke"))
    parser.add_argument("first", type=Path)
    parser.add_argument("reopened", type=Path, nargs="?")
    args = parser.parse_args()
    if (args.reopened is None) != (args.case == "revoke"):
        parser.error("logout and stop require a distinct reboot serial; revoke requires one serial")
    if args.reopened is not None and args.first.samefile(args.reopened):
        parser.error("first and reboot serials must be distinct")
    try:
        validate(args.case, serial(args.first), serial(args.reopened) if args.reopened else None)
    except (OSError, ValueError) as error:
        parser.exit(1, f"FAIL WS-10 serial markers: {error}\n")
    print(f"PASS WS-10 {args.case} serial markers")


if __name__ == "__main__":
    main()
