import hashlib
import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch
import subprocess

from run import (
    MARKERS,
    RunnerError,
    accepted,
    expected_el2_rejection,
    run_guest,
)


def fake_elf():
    data = bytearray(64)
    data[:6] = b"\x7fELF\x02\x01"
    data[18:20] = (183).to_bytes(2, "little")
    return bytes(data)


class AcceptanceTests(unittest.TestCase):
    def test_complete_ordered_guest_and_zero_exit(self):
        self.assertTrue(accepted(0, b"\n".join(MARKERS), False))

    def test_missing_duplicate_and_reordered_markers(self):
        self.assertFalse(accepted(0, b"\n".join(MARKERS[:-1]), False))
        self.assertFalse(accepted(0, b"\n".join(MARKERS + [MARKERS[-1]]), False))
        self.assertFalse(accepted(0, b"\n".join(reversed(MARKERS)), False))

    def test_crash_failure_and_timeout_even_with_markers(self):
        output = b"\n".join(MARKERS)
        self.assertFalse(accepted(1, output, False))
        self.assertFalse(accepted(-11, output, False))
        self.assertFalse(accepted(None, output, True))
        self.assertFalse(accepted(0, output + b"\nSPIKE FAIL", False))

    def test_el2_negative_requires_exact_failure_shape(self):
        self.assertTrue(expected_el2_rejection(1, b"SPIKE FAIL exception-level", False))
        self.assertFalse(expected_el2_rejection(1, b"SPIKE FAIL exception-level\nSPIKE COMPLETE PASS", False))
        self.assertFalse(expected_el2_rejection(0, b"SPIKE FAIL exception-level", False))
        self.assertFalse(expected_el2_rejection(1, b"SPIKE FAIL exception-level", True))

    def test_el2_negative_rejects_partial_success_and_unrelated_failures(self):
        failure = b"SPIKE FAIL exception-level"
        for serial in (
            failure + b" expected EL1 got EL0",
            b"prefix " + failure,
            failure + b"\n" + failure,
            failure + b"\nSPIKE FAIL panic",
            *(failure + b"\n" + marker for marker in MARKERS),
        ):
            with self.subTest(serial=serial):
                self.assertFalse(expected_el2_rejection(1, serial, False))
        self.assertTrue(expected_el2_rejection(1, failure + b"\n", False))

    def test_invalid_machine_properties_rejected_before_any_io(self):
        with patch("run.resolve_executable") as executable, \
             patch("run.subprocess.run") as process:
            for value in ("", "yes", "on,secure=on", None):
                with self.subTest(value=value), self.assertRaises(RunnerError):
                    run_guest("missing.elf", "missing-qemu", value, "unused")
            executable.assert_not_called()
            process.assert_not_called()

    def _fixture(self, root):
        binary = root / "external.elf"
        binary.write_bytes(fake_elf())
        qemu = root / "qemu"
        qemu.write_bytes(b"fake qemu executable\n")
        qemu.chmod(0o755)
        output = root / "run"
        return binary, qemu, output

    def test_runner_binds_binary_qemu_and_serial_digests_without_claiming_source(self):
        with tempfile.TemporaryDirectory() as temp:
            binary, qemu, output = self._fixture(Path(temp))
            serial = b"\n".join(MARKERS)
            process = subprocess.CompletedProcess([], 0, stdout=serial, stderr=b"")
            with patch("run.subprocess.check_output", return_value="QEMU fixture 1\n"), \
                 patch("run.subprocess.run", return_value=process):
                report = run_guest(binary, qemu, "off", output)
            self.assertEqual(report["outcome"], "PASS")
            self.assertEqual(report["binary_sha256"], hashlib.sha256(fake_elf()).hexdigest())
            self.assertEqual(report["binary_sha256_after"], report["binary_sha256"])
            self.assertEqual(report["qemu_sha256"], report["qemu_sha256_after"])
            self.assertEqual(report["virtualization"], "off")
            self.assertEqual(report["serial_sha256"], hashlib.sha256(serial).hexdigest())
            self.assertEqual(report["source_attestation"], "NOT_CLAIMED_BY_RUNNER")
            self.assertTrue((output / "result.json").is_file())

    def test_mutated_binary_during_run_is_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            binary, qemu, output = self._fixture(Path(temp))
            process = subprocess.CompletedProcess([], 0, stdout=b"\n".join(MARKERS), stderr=b"")

            def mutate(*args, **kwargs):
                binary.write_bytes(fake_elf() + b"changed")
                return process

            with patch("run.subprocess.check_output", return_value="QEMU fixture 1\n"), \
                 patch("run.subprocess.run", side_effect=mutate):
                report = run_guest(binary, qemu, "off", output)
            self.assertEqual(report["outcome"], "FAIL")
            self.assertNotEqual(report["binary_sha256"], report["binary_sha256_after"])

    def test_existing_output_is_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            binary, qemu, output = self._fixture(Path(temp))
            output.mkdir()
            with self.assertRaisesRegex(RunnerError, "already exists"):
                run_guest(binary, qemu, "off", output)

    def test_runner_cli_has_no_source_claim_flags(self):
        from run import build_parser
        with self.assertRaises(SystemExit):
            build_parser().parse_args([
                "--source-commit", "a" * 40, "--source-tree", "b" * 40,
                "--output", "unused",
            ])
        self.assertNotIn("--source-commit", build_parser().format_help())


if __name__ == "__main__":
    unittest.main()
