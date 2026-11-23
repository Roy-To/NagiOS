#!/usr/bin/env python3
"""Synthetic serial-only checks for Task24 Identity/PID 1 READY evidence."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from io import StringIO
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parent))
import task24_ready_observer as observer


class Task24ReadyObserverTests(unittest.TestCase):
    def test_success_requires_one_identity_then_pid1_ready(self) -> None:
        result = observer.validate_success_serial(
            f"session attached\n{observer.IDENTITY_READY}\n{observer.PID1_READY}\n",
            "save",
        )
        self.assertEqual(result["identity_ready_count"], 1)
        self.assertEqual(result["pid1_ready_count"], 1)
        self.assertLess(result["identity_ready_line"], result["pid1_ready_line"])

    def test_duplicate_or_reordered_success_ready_is_rejected(self) -> None:
        duplicate = f"{observer.IDENTITY_READY}\n{observer.PID1_READY}\n{observer.PID1_READY}\n"
        with self.assertRaisesRegex(observer.ObservationError, "exactly one"):
            observer.validate_success_serial(duplicate, "read")
        reordered = f"{observer.PID1_READY}\n{observer.IDENTITY_READY}\n"
        with self.assertRaisesRegex(observer.ObservationError, "does not follow"):
            observer.validate_success_serial(reordered, "read")

    def test_rejection_requires_denial_and_zero_ready_without_echoing_line(self) -> None:
        serial = (
            f"{observer.LOGIN_WAIT}\n"
            f"{observer.AUTH_DENIED_PREFIX} (Rejected; submitted=do-not-print)\n"
        )
        result = observer.validate_rejection_serial(serial)
        self.assertEqual(result["account_denied_count"], 1)
        self.assertEqual(result["identity_ready_count"], 0)
        self.assertEqual(result["pid1_ready_count"], 0)
        self.assertNotIn("do-not-print", json.dumps(result))

    def test_rejection_with_ready_or_authenticated_marker_is_rejected(self) -> None:
        base = f"{observer.LOGIN_WAIT}\n{observer.AUTH_DENIED_PREFIX} (Rejected)\n"
        with self.assertRaisesRegex(observer.ObservationError, "zero Identity/PID 1 READY"):
            observer.validate_rejection_serial(base + f"{observer.IDENTITY_READY}\n")
        with self.assertRaisesRegex(observer.ObservationError, "authenticated marker"):
            observer.validate_rejection_serial(base + f"{observer.AUTHENTICATED_PREFIX}\n")

    def test_cli_reports_only_counts_and_line_numbers(self) -> None:
        with tempfile.TemporaryDirectory(prefix="task24-ready-observer-") as temporary:
            success = Path(temporary) / "save.serial"
            reject = Path(temporary) / "reject.serial"
            success.write_text(
                f"{observer.IDENTITY_READY}\n{observer.PID1_READY}\n", encoding="utf-8"
            )
            reject.write_text(
                f"{observer.LOGIN_WAIT}\n{observer.AUTH_DENIED_PREFIX} (Rejected; private-value)\n",
                encoding="utf-8",
            )
            output = StringIO()
            with redirect_stdout(output):
                status = observer.main(
                    ["--success", f"save={success}", "--rejection", reject.as_posix()]
                )
            self.assertEqual(status, 0)
            result = json.loads(output.getvalue())
            self.assertEqual(result["cases"][0]["pid1_ready_count"], 1)
            self.assertEqual(result["cases"][1]["pid1_ready_count"], 0)
            self.assertNotIn("private-value", output.getvalue())


if __name__ == "__main__":
    unittest.main()
