#!/usr/bin/env python3
"""Regression tests for the persistence runner's direct-boot kernel preflight."""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


LANE = Path(__file__).resolve().parent
ROOT = LANE.parents[2]
ENTRY = LANE / "run_persistence_acceptance.py"
sys.path.insert(0, str(LANE))
import run_persistence_acceptance as acceptance


class PersistenceKernelPreflightTests(unittest.TestCase):
    def test_accepts_linux_x86_bzimage_header(self) -> None:
        image = bytearray(0x238)
        image[0x1FE:0x200] = b"\x55\xAA"
        image[0x202:0x206] = b"HdrS"
        image[0x206:0x208] = (0x020C).to_bytes(2, "little")
        image[0x236:0x238] = (0x0001).to_bytes(2, "little")

        with tempfile.TemporaryDirectory(prefix="notes-kernel-header-") as temporary:
            kernel = Path(temporary) / "kernel.bzImage"
            kernel.write_bytes(image)
            header = acceptance.validate_x86_64_kernel(kernel)

        self.assertEqual(header["boot_protocol_version"], 0x020C)
        self.assertTrue(header["xloadflags"] & 0x0001)

    def _successful_phase_serial(self, phase: str) -> str:
        phase_name = phase.upper()
        return "\n".join(
            (
                acceptance.SERVICE_HANDSHAKE,
                "nagi-init: state=ready",
                f"NAGI_NOTES_DESKTOP_INPUT_READY phase={phase} route=normal-desktop-handle-event",
                f'NAGI_NOTES_DESKTOP_{phase_name}_PASS id={acceptance.NOTES_ID} revision=2 '
                f'{acceptance.NOTES_BODY} normal_input=PASS fbdev=PASS nagi_vfs=PASS',
                f"NAGI_NOTES_DESKTOP_CHILD phase={phase} exit=Some(0) signal=None",
            )
        )

    def test_accepts_only_ordered_service_input_phase_and_child_markers(self) -> None:
        serial = self._successful_phase_serial("save")
        self.assertTrue(acceptance.phase_completion_ready(serial, "save", keys_sent=True))

    def test_rejects_service_ready_before_handshake(self) -> None:
        valid = self._successful_phase_serial("save").splitlines()
        reordered = "\n".join((valid[1], valid[0], *valid[2:]))
        with self.assertRaisesRegex(
            acceptance.AcceptanceError, "handshake did not precede service READY"
        ):
            acceptance.phase_completion_ready(reordered, "save", keys_sent=True)

    def test_rejects_input_ready_before_service_ready(self) -> None:
        valid = self._successful_phase_serial("save").splitlines()
        reordered = "\n".join((valid[2], valid[0], valid[1], valid[3], valid[4]))
        with self.assertRaisesRegex(acceptance.AcceptanceError, "before service READY"):
            acceptance.phase_completion_ready(reordered, "save", keys_sent=True)

    def test_rejects_phase_result_before_ready_or_input(self) -> None:
        valid = self._successful_phase_serial("save").splitlines()
        reordered = "\n".join((valid[0], valid[3], valid[4], valid[1], valid[2]))
        with self.assertRaisesRegex(acceptance.AcceptanceError, "preceded its input-ready marker"):
            acceptance.phase_completion_ready(reordered, "save", keys_sent=True)

    def test_rejects_phase_result_before_successful_key_delivery(self) -> None:
        with self.assertRaisesRegex(acceptance.AcceptanceError, "preceded successful QMP key delivery"):
            acceptance.phase_completion_ready(
                self._successful_phase_serial("save"), "save", keys_sent=False
            )

    def test_rejects_child_completion_before_phase_result(self) -> None:
        valid = self._successful_phase_serial("save").splitlines()
        reordered = "\n".join((valid[0], valid[1], valid[2], valid[4], valid[3]))
        with self.assertRaisesRegex(acceptance.AcceptanceError, "did not follow the phase result"):
            acceptance.phase_completion_ready(reordered, "save", keys_sent=True)

    def test_rejects_nonzero_notes_child(self) -> None:
        serial = self._successful_phase_serial("save").replace(
            "exit=Some(0) signal=None", "exit=Some(37) signal=None"
        )
        with self.assertRaisesRegex(acceptance.AcceptanceError, "did not report a successful exit"):
            acceptance.phase_completion_ready(serial, "save", keys_sent=True)

    def test_rejects_markers_from_the_other_phase(self) -> None:
        with self.assertRaisesRegex(acceptance.AcceptanceError, "other Notes phase"):
            acceptance.phase_completion_ready(
                self._successful_phase_serial("read"), "save", keys_sent=True
            )

    def test_requires_runner_and_qemu_success_after_guest_result(self) -> None:
        acceptance.require_successful_supervisor(
            "save",
            0,
            {"qemu_exit": 0, "timed_out": False, "cancelled": False},
        )
        with self.assertRaisesRegex(acceptance.AcceptanceError, "did not exit cleanly"):
            acceptance.require_successful_supervisor(
                "save",
                124,
                {"qemu_exit": -15, "timed_out": True, "cancelled": False},
            )
        with self.assertRaisesRegex(acceptance.AcceptanceError, "did not exit cleanly"):
            acceptance.require_successful_supervisor(
                "save",
                143,
                {"qemu_exit": -15, "timed_out": False, "cancelled": True},
            )
        with self.assertRaisesRegex(acceptance.AcceptanceError, "timed out or cancelled"):
            acceptance.require_successful_supervisor(
                "save",
                0,
                {"qemu_exit": 0, "timed_out": True, "cancelled": False},
            )

    def test_init_elf_in_kernel_slot_fails_before_output_or_qemu(self) -> None:
        with tempfile.TemporaryDirectory(prefix="notes-kernel-slot-") as temporary:
            root = Path(temporary)
            elf = root / "nagi-init.elf"
            elf.write_bytes(b"\x7fELF" + bytes(0x400))
            notes = root / "notes.elf"
            notes.write_bytes(b"notes sentinel\n")
            package = root / "app.xapp"
            package.write_bytes(b"package sentinel\n")
            service = root / "service.elf"
            service.write_bytes(b"service sentinel\n")
            client = root / "client.elf"
            client.write_bytes(b"client sentinel\n")
            qemu = root / "qemu-stub"
            qemu.write_text(
                "#!/bin/sh\nprintf 'invoked\\n' > \"$PERSISTENCE_QEMU_MARKER\"\nexit 97\n",
                encoding="utf-8",
            )
            qemu.chmod(0o700)
            qemu_marker = root / "qemu-invoked"
            output = root / "output"
            environment = os.environ.copy()
            environment["PERSISTENCE_QEMU_MARKER"] = str(qemu_marker)

            result = subprocess.run(
                [
                    sys.executable,
                    str(ENTRY),
                    "--init",
                    str(elf),
                    "--notes-elf",
                    str(notes),
                    "--package",
                    str(package),
                    "--service",
                    str(service),
                    "--client",
                    str(client),
                    "--kernel",
                    str(elf),
                    "--qemu",
                    str(qemu),
                    "--output-dir",
                    str(output),
                ],
                capture_output=True,
                text=True,
                env=environment,
                timeout=10,
                check=False,
            )

            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn("x86_64 kernel is missing the Linux boot flag 0xAA55", result.stderr)
            self.assertFalse(output.exists())
            self.assertFalse(qemu_marker.exists())


if __name__ == "__main__":
    unittest.main()
