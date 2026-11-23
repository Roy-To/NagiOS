"""Manifest-bound local RFB input for the Task24 development fixture."""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import task24_recovery_serials as recovery
from task24_rfb_input import RfbError, RfbKeyboard


FIXTURE_SCHEMA = "nagi-task24-rfb-auto-fixture/v1"
REGISTRATION_WAIT = "NAGI_LINUX_OWNER_REGISTRATION_WAITING_FOR_INPUT"
NOTES_INPUT_READY = recovery.INPUT
NOTES_SAVED_PREFIX = recovery.SAVED + " "
NOTES_REOPENED_PREFIX = recovery.REOPENED + " "
LOGIN_WAIT = recovery.LOGIN_WAIT
LOGIN_REJECTED = recovery.LOGIN_REJECTED


class AutomationError(RuntimeError):
    """The fixture manifest, loopback endpoint, or guest marker flow is invalid."""


@dataclass(frozen=True)
class DevelopmentFixture:
    path: Path
    sha256: str
    rfb_port: int
    owner_name: str = field(repr=False)
    owner_password: str = field(repr=False)
    wrong_password: str = field(repr=False)
    note_text: str = field(repr=False)

    @property
    def vnc_display(self) -> int:
        return self.rfb_port - 5900


def _identity(metadata: os.stat_result) -> tuple[int, ...]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_mode,
        metadata.st_nlink,
        metadata.st_uid,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def _read_private_manifest(path: Path, run_directory: Path) -> bytes:
    try:
        resolved = path.resolve(strict=True)
        before_path = path.lstat()
    except OSError as error:
        raise AutomationError(
            f"development fixture manifest is unavailable ({error.__class__.__name__})"
        ) from None
    if (
        path != resolved
        or path.parent != run_directory
        or path.name != "development-fixture.json"
        or stat.S_ISLNK(before_path.st_mode)
        or not stat.S_ISREG(before_path.st_mode)
        or stat.S_IMODE(before_path.st_mode) != 0o600
        or before_path.st_nlink != 1
        or before_path.st_size <= 0
        or before_path.st_size > 16 * 1024
        or (hasattr(os, "getuid") and before_path.st_uid != os.getuid())
    ):
        raise AutomationError(
            "development fixture manifest must be a private single-link file in the run directory"
        )

    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise AutomationError(
            f"development fixture manifest cannot be opened ({error.__class__.__name__})"
        ) from None
    try:
        before = os.fstat(descriptor)
        if _identity(before) != _identity(before_path):
            raise AutomationError("development fixture manifest changed before it was read")
        payload = bytearray()
        while len(payload) <= 16 * 1024:
            chunk = os.read(descriptor, min(4096, 16 * 1024 + 1 - len(payload)))
            if not chunk:
                break
            payload.extend(chunk)
        after = os.fstat(descriptor)
        try:
            after_path = path.lstat()
        except OSError as error:
            raise AutomationError(
                f"development fixture manifest path changed ({error.__class__.__name__})"
            ) from None
        if (
            len(payload) > 16 * 1024
            or _identity(before) != _identity(after)
            or _identity(after) != _identity(after_path)
            or stat.S_ISLNK(after_path.st_mode)
        ):
            raise AutomationError("development fixture manifest changed while it was read")
        return bytes(payload)
    finally:
        os.close(descriptor)


def load_development_fixture(
    path: Path,
    *,
    run_directory: Path,
    run_id: str,
    preflight_manifest_sha256: str,
) -> DevelopmentFixture:
    """Read a separate private sidecar; keep the official preflight schema fixed."""
    payload = _read_private_manifest(Path(path), run_directory)
    try:
        value: Any = json.loads(payload.decode("utf-8"))
    except (UnicodeError, json.JSONDecodeError):
        raise AutomationError("development fixture manifest is not valid UTF-8 JSON") from None
    required = {
        "schema",
        "run_id",
        "preflight_manifest_sha256",
        "rfb_port",
        "owner_name",
        "owner_password",
        "wrong_password",
        "note_text",
    }
    if not isinstance(value, dict) or set(value) != required:
        raise AutomationError("development fixture manifest has an unsupported shape")
    if value["schema"] != FIXTURE_SCHEMA or value["run_id"] != run_id:
        raise AutomationError("development fixture manifest is not bound to this run")
    if value["preflight_manifest_sha256"] != preflight_manifest_sha256:
        raise AutomationError("development fixture manifest is not bound to this preflight")

    port = value["rfb_port"]
    if isinstance(port, bool) or not isinstance(port, int) or not 5901 <= port <= 65535:
        raise AutomationError("development fixture RFB port must map to a valid QEMU VNC display")
    owner_name = value["owner_name"]
    owner_password = value["owner_password"]
    wrong_password = value["wrong_password"]
    note_text = value["note_text"]
    if (
        not isinstance(owner_name, str)
        or re.fullmatch(r"[a-z][a-z0-9]{0,31}", owner_name) is None
    ):
        raise AutomationError("development fixture owner name is invalid for guest registration")
    for password in (owner_password, wrong_password):
        if (
            not isinstance(password, str)
            or not 8 <= len(password) <= 64
            or re.fullmatch(r"[a-z0-9-]+", password) is None
        ):
            raise AutomationError("development fixture password is outside the supported ASCII set")
    if owner_password == wrong_password:
        raise AutomationError("development fixture wrong password must differ from its owner password")
    if (
        not isinstance(note_text, str)
        or not 1 <= len(note_text) <= 120
        or note_text.strip() != note_text
        or re.fullmatch(r"[a-z0-9 -]+", note_text) is None
    ):
        raise AutomationError("development fixture note must use lower-case ASCII text")
    return DevelopmentFixture(
        path=Path(path),
        sha256=hashlib.sha256(payload).hexdigest(),
        rfb_port=port,
        owner_name=owner_name,
        owner_password=owner_password,
        wrong_password=wrong_password,
        note_text=note_text,
    )


def _listener_names(stdout: str, expected_pid: int) -> list[str]:
    current_pid: int | None = None
    names: list[str] = []
    file_records = 0
    names_for_file = 0
    for line in stdout.splitlines():
        if line.startswith("p"):
            value = line[1:]
            if not value.isdecimal():
                raise AutomationError("lsof returned an invalid QEMU process record")
            if current_pid is not None:
                raise AutomationError("lsof returned more than one QEMU process record")
            current_pid = int(value)
            if current_pid != expected_pid:
                raise AutomationError("lsof returned a different QEMU child PID")
        elif line.startswith("f"):
            if current_pid != expected_pid or not line[1:]:
                raise AutomationError("lsof file record was not bound to the QEMU child")
            if file_records != names_for_file:
                raise AutomationError("lsof omitted a listener name for a file record")
            file_records += 1
        elif line.startswith("n"):
            if current_pid != expected_pid or file_records != names_for_file + 1:
                raise AutomationError("lsof listener record was not bound to the QEMU child")
            names.append(line[1:])
            names_for_file += 1
        elif line:
            raise AutomationError("lsof returned an unexpected listener record")
    if current_pid != expected_pid or file_records == 0 or file_records != names_for_file:
        raise AutomationError("lsof omitted a required QEMU listener file record")
    return names


def owned_loopback_listener(pid: int, port: int) -> bool:
    """Check the exact supervisor-owned QEMU PID's IPv4 loopback VNC listener."""
    if isinstance(pid, bool) or not isinstance(pid, int) or pid <= 0:
        raise AutomationError("QEMU child PID is invalid")
    lsof = shutil.which("lsof") or "/usr/sbin/lsof"
    command = [
        lsof,
        "-nP",
        "-a",
        "-p",
        str(pid),
        f"-iTCP:{port}",
        "-sTCP:LISTEN",
        "-Fpn",
    ]
    try:
        completed = subprocess.run(
            command,
            capture_output=True,
            text=True,
            timeout=2,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise AutomationError(f"QEMU listener ownership is unavailable ({error.__class__.__name__})") from None
    if completed.returncode == 1 and not completed.stdout.strip():
        return False
    if completed.returncode != 0:
        raise AutomationError("lsof could not verify the QEMU loopback listener")
    names = _listener_names(completed.stdout, pid)
    return names == [f"127.0.0.1:{port}"]


def _wait_for_owned_listener(pid: int, port: int, deadline: float) -> None:
    while time.monotonic() < deadline:
        if owned_loopback_listener(pid, port):
            return
        time.sleep(min(0.05, max(0.0, deadline - time.monotonic())))
    raise AutomationError("owned QEMU child did not expose its exact IPv4 loopback RFB listener")


def _require_owned_listener(pid: int, port: int, when: str) -> None:
    if not owned_loopback_listener(pid, port):
        raise AutomationError(f"QEMU loopback listener ownership changed {when}")


def _complete_lines(serial_text: str) -> list[str]:
    if not serial_text.endswith("\n"):
        serial_text = serial_text[: serial_text.rfind("\n") + 1]
    return [line.removesuffix("\r") for line in serial_text.splitlines()]


def _child_pid(supervisor_log: Path) -> int | None:
    try:
        lines = supervisor_log.read_text(encoding="utf-8", errors="replace").splitlines()
    except FileNotFoundError:
        return None
    except OSError as error:
        raise AutomationError(f"QEMU supervisor output is unavailable ({error.__class__.__name__})") from None
    entries = [line for line in lines if line.startswith("qemu_child_started pid=")]
    if not entries:
        return None
    if len(entries) != 1:
        raise AutomationError("QEMU supervisor did not identify exactly one child")
    match = re.fullmatch(r"qemu_child_started pid=([1-9][0-9]*)", entries[0])
    if match is None:
        raise AutomationError("QEMU supervisor child record is malformed")
    return int(match.group(1))


class AutomaticRfbInput:
    """Apply fixture keys only at the exact serial gates in the three-boot flow."""

    def __init__(self, fixture: DevelopmentFixture, phase: str) -> None:
        if phase not in ("save", "reject-close", "read"):
            raise AutomationError("unsupported automatic Task24 phase")
        self.fixture = fixture
        self.phase = phase
        self.keyboard: RfbKeyboard | None = None
        self.first_key_sent = False
        self.registration_sent = False
        self.save_login_sent = False
        self.note_input_sent = False
        self.note_closed = False
        self.wrong_password_sent = False
        self.rejection_closed = False
        self.owner_password_sent = False
        self.read_closed = False

    def _connect(self, pid: int, deadline: float) -> None:
        if self.keyboard is not None:
            return
        _wait_for_owned_listener(pid, self.fixture.rfb_port, deadline)
        try:
            self.keyboard = RfbKeyboard.connect_local(
                self.fixture.rfb_port,
                timeout=min(3.0, max(0.05, deadline - time.monotonic())),
            )
        except RfbError as error:
            raise AutomationError(f"local RFB handshake failed ({error.__class__.__name__})") from None
        _require_owned_listener(pid, self.fixture.rfb_port, "after RFB handshake")

    def _first_key(self, pid: int) -> RfbKeyboard:
        if self.keyboard is None:
            raise AutomationError("fixture input has no owned local RFB session")
        if not self.first_key_sent:
            _require_owned_listener(pid, self.fixture.rfb_port, "before the first fixture key")
            self.first_key_sent = True
        return self.keyboard

    def _tap(self, pid: int, keysym: int) -> None:
        try:
            self._first_key(pid).tap(keysym)
        except RfbError as error:
            raise AutomationError(f"local RFB key input failed ({error.__class__.__name__})") from None

    def _type(self, pid: int, value: str) -> None:
        try:
            self._first_key(pid).type_fixture_text(value)
        except RfbError as error:
            raise AutomationError(f"local RFB fixture text failed ({error.__class__.__name__})") from None

    def handle(self, serial_text: str, supervisor_log: Path, deadline: float) -> None:
        lines = _complete_lines(serial_text)
        line_set = set(lines)
        pid = _child_pid(supervisor_log)
        if pid is None:
            return

        if self.phase == "save":
            if REGISTRATION_WAIT in line_set and not self.registration_sent:
                self._connect(pid, deadline)
                self._tap(pid, 0xFF0D)  # Choose the manifest-pinned en-US language.
                self._type(pid, self.fixture.owner_name)
                self._tap(pid, 0xFF09)
                self._type(pid, self.fixture.owner_password)
                self._tap(pid, 0xFF09)
                self._type(pid, self.fixture.owner_password)
                self._tap(pid, 0xFF0D)
                self.registration_sent = True
            if LOGIN_WAIT in line_set and not self.save_login_sent:
                if not self.registration_sent:
                    raise AutomationError("first boot requested unlock before fixture owner registration")
                self._type(pid, self.fixture.owner_password)
                self._tap(pid, 0xFF0D)
                self.save_login_sent = True
            if NOTES_INPUT_READY in line_set and not self.note_input_sent:
                if not self.registration_sent or not self.save_login_sent:
                    raise AutomationError("Notes input appeared before fixture owner authentication")
                self._type(pid, self.fixture.note_text)
                self._tap(pid, 0xFF0D)
                self.note_input_sent = True
            if any(line.startswith(NOTES_SAVED_PREFIX) for line in lines) and not self.note_closed:
                if not self.note_input_sent:
                    raise AutomationError("Notes saved marker appeared before fixture note input")
                self._tap(pid, 0xFF1B)
                self.note_closed = True
        elif self.phase == "reject-close":
            if LOGIN_WAIT in line_set and not self.wrong_password_sent:
                self._connect(pid, deadline)
                self._type(pid, self.fixture.wrong_password)
                self._tap(pid, 0xFF0D)
                self.wrong_password_sent = True
            if (
                LOGIN_REJECTED in line_set
                and lines.count(LOGIN_WAIT) == 2
                and not self.rejection_closed
            ):
                self._tap(pid, 0xFF1B)
                self.rejection_closed = True
        else:
            if LOGIN_WAIT in line_set and not self.owner_password_sent:
                self._connect(pid, deadline)
                self._type(pid, self.fixture.owner_password)
                self._tap(pid, 0xFF0D)
                self.owner_password_sent = True
            if NOTES_INPUT_READY in line_set and not self.read_closed:
                if not any(line.startswith(NOTES_REOPENED_PREFIX) for line in lines):
                    raise AutomationError("read boot reached Notes input without reopening the saved note")
                self._tap(pid, 0xFF1B)
                self.read_closed = True

    def verify_complete(self) -> None:
        complete = {
            "save": (
                self.registration_sent
                and self.save_login_sent
                and self.note_input_sent
                and self.note_closed
            ),
            "reject-close": self.wrong_password_sent and self.rejection_closed,
            "read": self.owner_password_sent and self.read_closed,
        }[self.phase]
        if not complete:
            raise AutomationError(f"{self.phase} did not complete its gated fixture input sequence")

    def summary(self) -> dict[str, Any]:
        return {
            "mode": "MANIFEST_BACKED_RFB_FIXTURE",
            "fixture_manifest_sha256": self.fixture.sha256,
            "rfb_endpoint": f"127.0.0.1:{self.fixture.rfb_port}",
            "qemu_listener_owner_checked": self.first_key_sent,
            "fixture_secrets_recorded": False,
            "post_action_display_content": "NOT_VERIFIED",
        }

    def close(self) -> None:
        if self.keyboard is not None:
            self.keyboard.close()
            self.keyboard = None
