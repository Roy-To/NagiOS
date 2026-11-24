"""Validate the Calendar portion of a two-boot L10 acceptance transcript."""

import argparse
import re
from pathlib import Path


class CalendarAcceptanceError(ValueError):
    """The transcript does not establish the scripted Calendar scenario."""


REQUIRED_DATES = ("2026-11-23", "2026-11-24")
ID = re.compile(r"event_[a-z0-9_-]+")
MARKER = re.compile(r"\b(NAGI_DEV_(?:CALENDAR|DESKTOP)_[A-Z_]+)\b(.*)$")


def _require(condition, message):
    if not condition:
        raise CalendarAcceptanceError(message)


def _fields(tail):
    fields = {}
    for token in tail.split():
        key, separator, value = token.partition("=")
        _require(separator and key and key not in fields, "invalid marker fields")
        fields[key] = value
    return fields


def _boot(transcript: str, initial_count: int):
    stage = "desktop"
    writes = []
    days = {}
    for line in transcript.splitlines():
        marker = MARKER.search(line)
        if marker is None:
            continue
        tag, tail = marker.groups()
        if tag in {"NAGI_DEV_DESKTOP_APP_LAUNCH", "NAGI_DEV_DESKTOP_APP_CLOSED"}:
            if _fields(tail).get("app") != "calendar":
                continue
        if tag == "NAGI_DEV_DESKTOP_READY":
            _require(stage == "desktop", "unexpected Desktop READY")
            stage = "launch"
        elif tag == "NAGI_DEV_DESKTOP_APP_LAUNCH":
            _require(stage == "launch", "Calendar launch before Desktop READY")
            stage = "ready"
        elif tag == "NAGI_DEV_CALENDAR_READY":
            fields = _fields(tail)
            _require(stage == "ready", "unexpected Calendar READY")
            _require(fields.get("events") == str(initial_count), "unexpected initial event count")
            stage = "calendar"
        elif tag == "NAGI_DEV_CALENDAR_DAY":
            fields = _fields(tail)
            _require(stage == "calendar", "day marker outside Calendar")
            ids = fields.get("ids")
            _require(ids is not None, "day marker missing ids")
            rows = ids.split(",") if ids else []
            _require(all(ID.fullmatch(i) for i in rows) and len(set(rows)) == len(rows), "invalid day ids")
            _require(fields.get("rows") == str(len(rows)), "incorrect row count")
            days[fields.get("date")] = set(rows)
        elif tag == "NAGI_DEV_CALENDAR_SAVED":
            fields = _fields(tail)
            _require(stage == "calendar", "save marker outside Calendar")
            _require(ID.fullmatch(fields.get("id", "")), "invalid saved event id")
            _require(re.fullmatch(r"[1-9][0-9]*", fields.get("bytes", "")), "missing saved byte count")
            _require(re.fullmatch(r"[a-f0-9]{12,64}", fields.get("sha256", "")), "missing saved digest")
            writes.append((fields.get("op"), fields["id"]))
            days.clear()
        elif tag.startswith("NAGI_DEV_CALENDAR_"):
            raise CalendarAcceptanceError(f"unexpected {tag}")
        elif tag == "NAGI_DEV_DESKTOP_APP_CLOSED":
            _require(stage == "calendar" and _fields(tail).get("exit") == "ClosedByUser", "Calendar did not close normally")
            stage = "logout"
        elif tag == "NAGI_DEV_DESKTOP_LOGOUT_REQUESTED":
            _require(stage == "logout", "logout before Calendar closed")
            stage = "done"
    _require(stage == "done", "missing Calendar close or Desktop logout")
    return writes, days


def validate(before: str, after: str):
    writes, initial = _boot(before, 0)
    restored_writes, restored = _boot(after, 3)
    _require(not restored_writes, "reboot readback attempted a write")
    _require([op for op, _ in writes] == ["create", "create", "create", "update", "create", "delete"],
             "unexpected sequence of Calendar commits")
    a, b, c, updated, d, deleted = (event_id for _, event_id in writes)
    _require(len({a, b, c, d}) == 4 and b == updated and d == deleted, "Calendar identities did not survive edit/delete")
    for label, observed in (("before reboot", initial), ("after reboot", restored)):
        for date, expected in zip(REQUIRED_DATES, ({a, c}, {b, c}), strict=True):
            _require(observed.get(date) == expected, f"{label}: wrong Calendar rows on {date}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path, help="first boot serial")
    parser.add_argument("after", type=Path, help="same-Data reboot serial")
    args = parser.parse_args()
    try:
        validate(args.before.read_text(), args.after.read_text())
    except (OSError, CalendarAcceptanceError) as error:
        parser.exit(1, f"Calendar transcript rejected: {error}\n")
    print("calendar_l10_serial=PASS")


if __name__ == "__main__":
    main()
