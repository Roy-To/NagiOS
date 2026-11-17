"""Failure-marker regressions for the Linux development acceptance runner."""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch


LANE = Path(__file__).resolve().parent
sys.path.insert(0, str(LANE))
import run_persistence_acceptance as acceptance


class GuestFailureMarkerTests(unittest.TestCase):
    def test_pid1_lifecycle_failure_is_detected_as_a_complete_line(self) -> None:
        serial = (
            "nagi-init: development-only NIPC handshake and correlated file.search reply verified\n"
            "nagi-init: state=ready\n"
            "nagi-init: lifecycle failure (Unavailable); stopping services\r\n"
        )

        self.assertEqual(
            acceptance.guest_failure_marker(serial),
            "nagi-init: lifecycle failure (Unavailable); stopping services",
        )

    def test_embedded_or_extended_lifecycle_text_is_not_a_failure_marker(self) -> None:
        serial = (
            "diagnostic mentions nagi-init: lifecycle failure (Unavailable); stopping services\n"
            "nagi-init: lifecycle failure (Unavailable); stopping services extra\n"
        )

        self.assertIsNone(acceptance.guest_failure_marker(serial))

    def test_fail_closed_wait_is_detected_after_lifecycle_failure(self) -> None:
        marker = "nagi-init: PID 1 remains alive in fail-closed wait; service readiness is not asserted"

        self.assertEqual(acceptance.guest_failure_marker(marker + "\n"), marker)


class QemuQuitRaceTests(unittest.TestCase):
    def test_qmp_reply_race_accepts_supervisor_natural_exit(self) -> None:
        process = Mock()
        process.poll.return_value = None
        process.wait.return_value = 0

        with tempfile.TemporaryDirectory(prefix="notes-qmp-quit-race-") as temporary:
            controls = Path(temporary) / "controls.log"
            with patch.object(acceptance, "qmp_command", side_effect=RuntimeError("QMP closed")):
                stopped = acceptance.request_supervised_qemu_quit(
                    process, Path(temporary) / "qmp.sock", controls, race_grace_seconds=0.01
                )

            self.assertTrue(stopped)
            process.send_signal.assert_not_called()
            self.assertIn("qmp_reply=unavailable qmp_error=RuntimeError supervisor_exit=0", controls.read_text())

    def test_qmp_failure_keeps_running_supervisor_as_cancelled(self) -> None:
        process = Mock()
        process.poll.return_value = None
        process.wait.side_effect = subprocess.TimeoutExpired("supervisor", 0.01)

        with tempfile.TemporaryDirectory(prefix="notes-qmp-quit-running-") as temporary:
            controls = Path(temporary) / "controls.log"
            with patch.object(acceptance, "qmp_command", side_effect=RuntimeError("QMP unavailable")):
                stopped = acceptance.request_supervised_qemu_quit(
                    process, Path(temporary) / "qmp.sock", controls, race_grace_seconds=0.01
                )

            self.assertFalse(stopped)
            self.assertFalse(controls.exists())

    def test_successful_qmp_quit_is_logged(self) -> None:
        process = Mock()
        process.poll.return_value = None

        with tempfile.TemporaryDirectory(prefix="notes-qmp-quit-success-") as temporary:
            controls = Path(temporary) / "controls.log"
            with patch.object(acceptance, "qmp_command"):
                stopped = acceptance.request_supervised_qemu_quit(
                    process, Path(temporary) / "qmp.sock", controls
                )

            self.assertTrue(stopped)
            self.assertEqual(controls.read_text(), "command=quit qmp_reply=return\n")


if __name__ == "__main__":
    unittest.main()
