"""Focused process and QMP supervision regressions for run_qemu_child.py."""

from __future__ import annotations

import importlib.util
import json
import signal
import socket
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


TOOLS = Path(__file__).parent
RUNNER = TOOLS / "run_qemu_child.py"
SPEC = importlib.util.spec_from_file_location("run_qemu_child", RUNNER)
assert SPEC is not None and SPEC.loader is not None
run_qemu_child = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(run_qemu_child)

SLEEPING_CHILD = "import time; time.sleep(30)"
EXITING_CHILD = "raise SystemExit(0)"


class FakeConnection:
    def __init__(self, incoming: list[bytes | BaseException]) -> None:
        self.incoming = list(incoming)
        self.outgoing: list[bytes] = []
        self.timeouts: list[float] = []
        self.closed = False

    def connect(self, _path: str) -> None:
        return None

    def settimeout(self, timeout: float) -> None:
        self.timeouts.append(timeout)

    def recv(self, _size: int) -> bytes:
        if not self.incoming:
            raise AssertionError("unexpected additional QMP read")
        value = self.incoming.pop(0)
        if isinstance(value, BaseException):
            raise value
        return value

    def sendall(self, payload: bytes) -> None:
        self.outgoing.append(payload)

    def close(self) -> None:
        self.closed = True


class RunQemuChildTests(unittest.TestCase):
    def make_command(
        self, directory: Path, child_code: str, timeout: int = 30
    ) -> tuple[list[str], Path, Path]:
        qmp_socket = directory / "qmp.sock"
        qemu_log = directory / "qemu.log"
        qmp_argument = f"unix:{qmp_socket},server=on,wait=off"
        command = [
            sys.executable,
            str(RUNNER),
            "--timeout-seconds",
            str(timeout),
            "--qmp-socket",
            str(qmp_socket),
            "--qemu-log",
            str(qemu_log),
            "--",
            sys.executable,
            "-c",
            child_code,
            "-qmp",
            qmp_argument,
        ]
        return command, qmp_socket, qemu_log

    def result_json(self, output: str) -> dict[str, object]:
        result_lines = [line for line in output.splitlines() if line.startswith("{")]
        self.assertTrue(result_lines, output)
        return json.loads(result_lines[-1])

    def test_normal_foreground_child_start_and_exit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            command, qmp_socket, qemu_log = self.make_command(directory, EXITING_CHILD)
            completed = subprocess.run(
                command,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=5,
                check=False,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr + completed.stdout)
            result = self.result_json(completed.stdout)
            self.assertEqual(result["qemu_exit"], 0)
            self.assertFalse(result["timed_out"])
            self.assertFalse(result["cancelled"])
            self.assertTrue(qemu_log.exists())
            self.assertFalse(qmp_socket.exists())

    def test_qmp_greeting_and_command_responses_complete_normally(self) -> None:
        connection = FakeConnection(
            [
                b'{"QMP":{}}\r\n',
                b'{"return":{}}\r\n',
                b'{"return":{}}\r\n',
            ]
        )
        with patch.object(run_qemu_child.socket, "socket", return_value=connection):
            run_qemu_child.qmp_command(Path("fake.sock"), "quit", timeout=1.0)
        self.assertEqual(
            connection.outgoing,
            [b'{"execute":"qmp_capabilities"}\r\n', b'{"execute":"quit"}\r\n'],
        )
        self.assertTrue(connection.closed)

    def test_qmp_command_forwards_structured_arguments(self) -> None:
        connection = FakeConnection(
            [
                b'{"QMP":{}}\r\n',
                b'{"return":{}}\r\n',
                b'{"return":""}\r\n',
            ]
        )
        with patch.object(run_qemu_child.socket, "socket", return_value=connection):
            run_qemu_child.qmp_command(
                Path("fake.sock"),
                "human-monitor-command",
                timeout=1.0,
                arguments={"command-line": "sendkey tab"},
            )
        self.assertEqual(
            json.loads(connection.outgoing[1]),
            {
                "execute": "human-monitor-command",
                "arguments": {"command-line": "sendkey tab"},
            },
        )
        self.assertTrue(connection.closed)

    def test_qmp_sendkey_rejects_unlisted_controls_before_connecting(self) -> None:
        with patch.object(run_qemu_child.socket, "socket") as make_socket:
            with self.assertRaises(ValueError):
                run_qemu_child.qmp_sendkey(Path("fake.sock"), "tab; quit")
        make_socket.assert_not_called()

    def test_silent_qmp_greeting_is_bounded(self) -> None:
        self.assert_silent_qmp_is_bounded([socket.timeout("no greeting")], writes=0)

    def test_silent_qmp_command_response_is_bounded(self) -> None:
        self.assert_silent_qmp_is_bounded(
            [b'{"QMP":{}}\r\n', socket.timeout("no command response")], writes=1
        )

    def test_qmp_events_do_not_restart_the_command_deadline(self) -> None:
        connection = FakeConnection(
            [
                b'{"QMP":{}}\r\n',
                b'{"return":{}}\r\n',
                b'{"event":"CLOCK_TICK"}\r\n',
                b'{"event":"CLOCK_TICK"}\r\n',
                b'{"return":{}}\r\n',
            ]
        )
        monotonic = iter([0.0, 0.2, 0.3, 0.4, 0.6, 0.75, 1.1])
        with patch.object(run_qemu_child.socket, "socket", return_value=connection):
            with patch.object(run_qemu_child.time, "monotonic", side_effect=monotonic):
                with self.assertRaises(TimeoutError):
                    run_qemu_child.qmp_command(Path("fake.sock"), "quit", timeout=1.0)
        self.assertEqual(
            connection.outgoing,
            [b'{"execute":"qmp_capabilities"}\r\n', b'{"execute":"quit"}\r\n'],
        )
        self.assertTrue(connection.closed)

    def assert_silent_qmp_is_bounded(
        self, incoming: list[bytes | BaseException], *, writes: int
    ) -> None:
        connection = FakeConnection(incoming)
        with patch.object(run_qemu_child.socket, "socket", return_value=connection):
            with self.assertRaises(TimeoutError):
                run_qemu_child.qmp_command(Path("fake.sock"), "quit", timeout=0.05)
        self.assertEqual(len(connection.outgoing), writes)
        self.assertTrue(connection.closed)

    def test_sigint_and_sigterm_cancellation_are_nonzero_and_recorded(self) -> None:
        for cancel_signal in (signal.SIGINT, signal.SIGTERM):
            with self.subTest(signal=cancel_signal.name):
                self.assert_cancel_is_nonzero_and_reaped(cancel_signal)

    def assert_cancel_is_nonzero_and_reaped(self, cancel_signal: int) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            command, qmp_socket, _ = self.make_command(directory, SLEEPING_CHILD)
            runner = subprocess.Popen(
                command,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            assert runner.stdout is not None
            started_line = runner.stdout.readline()
            self.assertIn("qemu_child_started pid=", started_line)
            runner.send_signal(cancel_signal)
            stdout, stderr = runner.communicate(timeout=6)
            self.assertEqual(runner.returncode, 128 + cancel_signal, stderr + stdout)
            result = self.result_json(stdout)
            self.assertTrue(result["cancelled"])
            self.assertEqual(result["cancel_signal"], cancel_signal)
            self.assertNotEqual(runner.returncode, 0)
            self.assertFalse(qmp_socket.exists())

    def test_timeout_reaps_foreground_child_when_qmp_is_unavailable(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            command, qmp_socket, _ = self.make_command(directory, SLEEPING_CHILD, timeout=1)
            completed = subprocess.run(
                command,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=5,
                check=False,
            )
            self.assertEqual(completed.returncode, 124, completed.stderr + completed.stdout)
            self.assertTrue(self.result_json(completed.stdout)["timed_out"])
            self.assertFalse(qmp_socket.exists())

    def test_daemonize_is_rejected_before_launch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            qmp_socket = directory / "qmp.sock"
            qemu_log = directory / "qemu.log"
            qmp_argument = f"unix:{qmp_socket},server=on,wait=off"
            command = [
                sys.executable,
                str(RUNNER),
                "--timeout-seconds",
                "1",
                "--qmp-socket",
                str(qmp_socket),
                "--qemu-log",
                str(qemu_log),
                "--",
                "qemu-system-x86_64",
                "-qmp",
                qmp_argument,
                "-daemonize",
            ]
            completed = subprocess.run(
                command,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=4,
                check=False,
            )
            self.assertEqual(completed.returncode, 2)
            self.assertIn("-daemonize is incompatible", completed.stderr)
            self.assertFalse(qemu_log.exists())
            self.assertFalse(qmp_socket.exists())


if __name__ == "__main__":
    unittest.main()
