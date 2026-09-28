"""Versioned, deterministic inputs for browser-state recovery acceptance.

These are M18-C test envelopes, not the Nagi browser's storage schema. The
guest acceptance adapter must translate each scenario into the integrated
M18 storage boundary before asserting product behavior.
"""

from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path, PurePosixPath


FIXTURE_FORMAT = "m18c-test-fixture-v1"


def _record(kind: str, payload: dict) -> bytes:
    return (
        json.dumps(
            {"fixture_format": FIXTURE_FORMAT, "kind": kind, "payload": payload},
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        )
        + "\n"
    ).encode("utf-8")


@dataclass(frozen=True)
class StateFixture:
    """Raw files for one storage/recovery scenario; no restore behavior implied."""

    scenario_id: str
    files: tuple[tuple[str, bytes], ...]


_CLEAN_SESSION = _record(
    "session",
    {
        "active_tab": "tab-02",
        "tabs": [
            {"tab_id": "tab-01", "url": "http://fixture.invalid/one"},
            {"tab_id": "tab-02", "url": "http://fixture.invalid/two"},
        ],
    },
)
_COMMITTED_STATE = _record(
    "session",
    {"checkpoint": "before-interrupted-write", "tabs": [{"tab_id": "tab-01"}]},
)

STATE_FIXTURES = (
    StateFixture("session.missing", ()),
    StateFixture("session.empty", (("session.json", b""),)),
    StateFixture(
        "session.clean-multitab",
        (("session.json", _CLEAN_SESSION),),
    ),
    StateFixture(
        "session.truncated",
        (("session.json", b'{"fixture_format":"m18c-test-fixture-v1","kind":"session","payload":'),),
    ),
    StateFixture("session.corrupt", (("session.json", b"\xff\x00not-json\n"),)),
    StateFixture(
        "session.incompatible-version",
        ((
            "session.json",
            json.dumps(
                {"fixture_format": "m18c-test-fixture-v99", "kind": "session", "payload": {}},
                sort_keys=True,
                separators=(",", ":"),
            ).encode("utf-8") + b"\n",
        ),),
    ),
    StateFixture(
        "session.partial-records",
        (
            ("history/entry-01.json", _record("history-entry", {"title": "known good"})),
            ("history/entry-02.json", b'{"fixture_format":"m18c-test-fixture-v1","kind":"history-entry"'),
            ("bookmarks.json", _record("bookmarks", {"items": [{"title": "kept"}]})),
        ),
    ),
    StateFixture(
        "reliability.interrupted-write",
        (
            ("records/committed.json", _COMMITTED_STATE),
            ("records/pending.json.tmp", b'{"fixture_format":"m18c-test-fixture-v1","kind":"session","payload":{"checkpoint":"half'),
        ),
    ),
    StateFixture("reliability.repeated-restart", (("session.json", _CLEAN_SESSION),)),
)


def materialize_state_fixture(scenario_id: str, root: Path) -> Path:
    """Write one fixture to a fresh directory and return that directory."""
    fixture = next((item for item in STATE_FIXTURES if item.scenario_id == scenario_id), None)
    if fixture is None:
        raise KeyError(f"unknown M18-C storage fixture {scenario_id!r}")
    destination = Path(root) / scenario_id
    destination.mkdir(parents=True, exist_ok=False)
    for relative_name, payload in fixture.files:
        relative_path = PurePosixPath(relative_name)
        if relative_path.is_absolute() or ".." in relative_path.parts:
            raise ValueError(f"unsafe M18-C fixture path {relative_name!r}")
        file_path = destination.joinpath(*relative_path.parts)
        file_path.parent.mkdir(parents=True, exist_ok=True)
        file_path.write_bytes(payload)
    return destination
