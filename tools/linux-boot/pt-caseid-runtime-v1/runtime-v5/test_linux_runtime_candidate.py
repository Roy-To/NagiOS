#!/usr/bin/env python3
"""Host contract tests for the adopted Linux case-ID runtime candidate."""

from __future__ import annotations

import contextlib
import errno
import hashlib
import importlib.util
import io
import json
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


HERE = Path(__file__).resolve().parent
TASK_ROOT = HERE.parents[2]
CANDIDATE = HERE / "linux_runtime_candidate.py"
BUILDER_PATH = HERE.parent / "build_initramfs.py"
QEMU_CHILD_RUNNER = HERE.parents[1] / "run_qemu_child.py"
EXPECTED_CASE_IDS_BY_ENTRY = {
    "linux_model_clock_consumer": ("linux_model_clock_consumer",),
    "notes-host-preview": ("notes-host-preview.save", "notes-host-preview.read"),
    "in_process_echo": ("in_process_echo",),
    "sdk-development-smoke": ("sdk-development-smoke",),
    "installed_service_clock_smoke": ("installed_service_clock_smoke",),
    "notes-event-bridge-smoke": ("notes-event-bridge-smoke.save", "notes-event-bridge-smoke.read"),
    "desktop-host-preview-smoke": ("desktop-host-preview-smoke",),
    "desktop-evdev-stream-smoke": (
        "desktop-evdev-stream-smoke.create-empty",
        "desktop-evdev-stream-smoke.producer",
        "desktop-evdev-stream-smoke.stdin",
        "desktop-evdev-stream-smoke.read",
    ),
    "desktop-evdev-integrity-smoke": (
        "desktop-evdev-integrity-smoke.integrity-seed-v1",
        "desktop-evdev-integrity-smoke.integrity-save",
        "desktop-evdev-integrity-smoke.integrity-check",
    ),
}


def make_test_elf() -> bytes:
    """Return a minimal host-only ELF64 ET_EXEC accepted by the archive validator."""
    data = bytearray(120)
    data[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into(
        "<HHIQQQIHHHHHH", data, 16,
        2, 62, 1, 0x400000, 64, 0, 0, 64, 56, 1, 0, 0, 0,
    )
    struct.pack_into("<IIQQQQQQ", data, 64, 1, 5, 0, 0x400000, 0, len(data), len(data), 0x1000)
    return bytes(data)


def make_test_static_pie() -> bytes:
    """Return a minimal host-only ET_DYN fixture with PT_LOAD and DT_NULL."""
    data = bytearray(200)
    data[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into(
        "<HHIQQQIHHHHHH", data, 16,
        3, 62, 1, 0x80, 64, 0, 0, 64, 56, 2, 0, 0, 0,
    )
    struct.pack_into("<IIQQQQQQ", data, 64, 1, 5, 0, 0, 0, len(data), len(data), 0x1000)
    struct.pack_into("<IIQQQQQQ", data, 120, 2, 6, 184, 184, 0, 16, 16, 8)
    struct.pack_into("<qQ", data, 184, 0, 0)
    return bytes(data)


def load_subject():
    spec = importlib.util.spec_from_file_location("linux_runtime_candidate_test_subject", CANDIDATE)
    if spec is None or spec.loader is None:
        raise RuntimeError("candidate module could not be loaded")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def fake_qemu(path: Path, *, runner_status: int = 0, qemu_status: int = 0,
              delay: float = 0.0, marker: Path | None = None,
              entry_count: int = 9, duplicate_name: bool = False,
              substitute_last_name: bool = False, output_error: int = 0,
              crlf_serial: bool = False,
              failure_entries: dict[str, int] | None = None,
              pass_entries: set[str] | None = None,
              missing_cases_entry: str | None = None,
              duplicate_cases_entry: str | None = None,
              case_results: bool = True,
              omit_case_id: str | None = None,
              duplicate_case_id: str | None = None,
              unknown_case_id: str | None = None,
              wrong_case_outcome_id: str | None = None,
              malformed_case_marker: bool = False) -> None:
    lines = ["#!/usr/bin/env python3", "import pathlib, sys, time"]
    line_ending = "\r\n" if crlf_serial else "\n"
    if marker is not None:
        lines.append(f"pathlib.Path({str(marker)!r}).write_text('started')")
    if delay:
        lines.append(f"time.sleep({delay!r})")
    names = [
        "linux_model_clock_consumer", "notes-host-preview", "in_process_echo",
        "sdk-development-smoke", "installed_service_clock_smoke",
        "notes-event-bridge-smoke", "desktop-host-preview-smoke",
        "desktop-evdev-stream-smoke", "desktop-evdev-integrity-smoke",
    ]
    case_counts = {
        "linux_model_clock_consumer": 1,
        "notes-host-preview": 2,
        "in_process_echo": 1,
        "sdk-development-smoke": 1,
        "installed_service_clock_smoke": 1,
        "notes-event-bridge-smoke": 2,
        "desktop-host-preview-smoke": 1,
        "desktop-evdev-stream-smoke": 4,
        "desktop-evdev-integrity-smoke": 3,
    }
    if failure_entries is None:
        failure_entries = {"in_process_echo": 23} if runner_status == 1 else {}
    for index in range(entry_count):
        name_index = 0 if duplicate_name and index == 1 else index
        entry_name = names[name_index] if name_index < len(names) else f"entry{name_index}"
        if substitute_last_name and index == 8:
            entry_name = "unexpected-entry"
        values = [(0, 0)] * case_counts.get(entry_name, 1)
        if entry_name in failure_entries and values:
            values[0] = (failure_entries[entry_name], 0)
        if entry_name == missing_cases_entry and values:
            values.pop()
        if entry_name == duplicate_cases_entry and values:
            values.append(values[-1])
        cases = ",".join(f"{actual}/expected{expected}" for actual, expected in values)
        result = "FAIL" if entry_name in failure_entries else "PASS"
        if pass_entries and entry_name in pass_entries:
            result = "PASS"
        if case_results:
            ids = EXPECTED_CASE_IDS_BY_ENTRY.get(entry_name, ())
            for case_id, (actual, expected) in zip(ids, values):
                output_id = "unknown-case-id" if case_id == unknown_case_id else case_id
                case_outcome = "PASS" if actual == expected else "FAIL"
                if case_id == wrong_case_outcome_id:
                    case_outcome = "FAIL" if case_outcome == "PASS" else "PASS"
                case_line = (
                    f"CASE_RESULT id={output_id} exit={actual} expected={expected} "
                    f"outcome={case_outcome}"
                )
                if case_id != omit_case_id:
                    lines.append(f"print({case_line!r}, end={line_ending!r})")
                if case_id == duplicate_case_id:
                    lines.append(f"print({case_line!r}, end={line_ending!r})")
        lines.append(
            f"print('ENTRY_RESULT name={entry_name} cases={cases} outcome={result}', end={line_ending!r})"
        )
    if malformed_case_marker:
        lines.append(f"print('CASE_RESULT legacy-format-without-fixed-id', end={line_ending!r})")
    guest_exit = 0x100 if runner_status else 0
    lines.append(
        f"print('RUNNER_DONE entries=9 status={runner_status} output_error={output_error} scope=test', end={line_ending!r})"
    )
    lines.append(
        f"print('Kernel panic - not syncing: Attempted to kill init! exitcode=0x{guest_exit:08x}', end={line_ending!r})"
    )
    lines.append(f"sys.exit({qemu_status})")
    path.write_text("\n".join(lines) + "\n")
    path.chmod(0o755)


class CandidateContractTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix=".runner-candidate-test-", dir=TASK_ROOT)
        self.root = Path(self.temp.name)
        self.out = self.root / "out"
        self.kernel = self.root / "kernel.fixture"
        self.kernel.write_bytes(b"host-only fake kernel input")
        self.runner = make_test_elf()
        self.fail_child = make_test_elf()
        self.runner_elf = self.root / "runner.elf"
        self.fail_child_elf = self.root / "fail-child.elf"
        self.runner_elf.write_bytes(self.runner)
        self.fail_child_elf.write_bytes(self.fail_child)
        self.runner_elf.chmod(0o755)
        self.fail_child_elf.chmod(0o755)

        spec = importlib.util.spec_from_file_location("candidate_test_builder", BUILDER_PATH)
        builder = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(builder)
        self.builder = builder
        self.init_elf = self.root / "init.elf"
        self.init_elf.write_bytes(make_test_static_pie())
        self.init_elf.chmod(0o755)
        self.base_bundle = self.root / "base.fixture.cpio"
        self.base_bundle.write_bytes(builder.make_archive([
            ("bin", stat.S_IFDIR | 0o755, b"", 2, (0, 0)),
            ("bin/in_process_echo", stat.S_IFREG | 0o755, b"host-only original entry", 1, (0, 0)),
        ]))
        self.initrd = self.root / "runner.fixture.cpio"
        self.initrd.write_bytes(builder.make_archive([
            ("bin", stat.S_IFDIR | 0o755, b"", 2, (0, 0)),
            ("bin/entry-runtime-runner-io-fixed", stat.S_IFREG | 0o755,
             self.runner, 1, (0, 0)),
        ]))
        self.qemu = self.root / "fake-qemu"
        self.marker = self.root / "qemu-started"

    def tearDown(self):
        self.temp.cleanup()

    def common_run_args(self, *, expected: int = 0, log_name: str = "run.log") -> list[str]:
        return [
            sys.executable, str(CANDIDATE), "run",
            "--task-root", str(self.root),
            "--output-dir", str(self.out),
            "--qemu", str(self.qemu),
            "--qemu-child-runner", str(QEMU_CHILD_RUNNER),
            "--kernel", str(self.kernel),
            "--initrd", str(self.initrd),
            "--rdinit", "/bin/entry-runtime-runner-io-fixed",
            "--timeout-seconds", "5",
            "--expected-runner-status", str(expected),
            "--expected-entries", "9",
            "--log-name", log_name,
        ]

    def test_cli_rejects_missing_arguments_and_missing_inputs_before_write(self):
        missing_args = subprocess.run(
            [sys.executable, str(CANDIDATE), "run"],
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(missing_args.returncode, 2)

        fake_qemu(self.qemu, marker=self.marker)
        absent_out = self.root / "not-created"
        args = [
            sys.executable, str(CANDIDATE), "build-fixtures",
            "--task-root", str(self.root),
            "--output-dir", str(absent_out),
            "--bundle", str(self.root / "missing-bundle.cpio"),
            "--runner-elf", str(self.runner_elf),
            "--fail-child-elf", str(self.fail_child_elf),
        ]
        result = subprocess.run(args, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertFalse(absent_out.exists())
        self.assertFalse(self.marker.exists())

        missing_kernel_args = self.common_run_args(log_name="missing-kernel.log")
        missing_kernel_args[
            missing_kernel_args.index("--kernel") + 1
        ] = str(self.root / "missing-kernel")
        missing_kernel = subprocess.run(
            missing_kernel_args, capture_output=True, text=True, check=False,
        )
        self.assertEqual(missing_kernel.returncode, 2)
        self.assertFalse(self.out.exists())
        self.assertFalse(self.marker.exists())

    def test_rejects_output_outside_task_root_without_creating_it(self):
        outside = self.root.parent / f".outside-{self.root.name}"
        self.assertFalse(outside.exists())
        result = subprocess.run([
            sys.executable, str(CANDIDATE), "build-fixtures",
            "--task-root", str(self.root),
            "--output-dir", str(outside),
            "--bundle", str(self.base_bundle),
            "--runner-elf", str(self.runner_elf),
            "--fail-child-elf", str(self.fail_child_elf),
        ], capture_output=True, text=True, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(outside.exists())

    def test_success_and_expected_failure_contracts(self):
        fake_qemu(self.qemu)
        success = subprocess.run(self.common_run_args(), capture_output=True, text=True, check=False)
        self.assertEqual(success.returncode, 0, success.stderr)
        payload = json.loads(success.stdout.strip().splitlines()[-1])
        self.assertEqual(payload["entry_groups"], {"pass": 9, "fail": 0})
        self.assertEqual(payload["runner_status"], 0)
        self.assertEqual(payload["guest_pid1_status"], 0)

        fake_qemu(self.qemu, runner_status=1)
        expected_failure = subprocess.run(
            self.common_run_args(expected=1, log_name="expected-failure.log"),
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(expected_failure.returncode, 0, expected_failure.stderr)
        failure_payload = json.loads(expected_failure.stdout.strip().splitlines()[-1])
        self.assertEqual(failure_payload["entry_groups"], {"pass": 8, "fail": 1})
        self.assertEqual(failure_payload["runner_status"], 1)
        self.assertEqual(failure_payload["guest_pid1_status"], 1)
        result_lines = [
            line for line in Path(failure_payload["log"]).read_text().splitlines()
            if line.startswith("ENTRY_RESULT") and " outcome=FAIL" in line
        ]
        self.assertEqual(
            result_lines,
            ["ENTRY_RESULT name=in_process_echo cases=23/expected0 outcome=FAIL"],
        )

    def test_expected_negative_requires_only_in_process_echo_exit_23(self):
        scenarios = [
            ("other_group_failed", {"notes-host-preview": 23}, {}),
            ("target_wrong_exit", {"in_process_echo": 24}, {}),
            ("target_exit_23_without_fail_outcome", {"in_process_echo": 23}, {"pass_entries": {"in_process_echo"}}),
            ("multiple_groups_failed", {"in_process_echo": 23, "notes-host-preview": 23}, {}),
            ("target_entry_missing", {"in_process_echo": 23}, {"entry_count": 8}),
            ("target_entry_duplicated", {"in_process_echo": 23}, {"duplicate_name": True}),
            ("missing_subcase", {"in_process_echo": 23}, {"missing_cases_entry": "notes-host-preview"}),
            ("duplicated_subcase", {"in_process_echo": 23}, {"duplicate_cases_entry": "notes-host-preview"}),
            ("aggregate_runner_abnormal", {"in_process_echo": 23}, {"runner_status": 2}),
        ]
        for label, failures, extra in scenarios:
            with self.subTest(scenario=label):
                runner_status = extra.pop("runner_status", 1)
                fake_qemu(
                    self.qemu,
                    runner_status=runner_status,
                    failure_entries=failures,
                    **extra,
                )
                result = subprocess.run(
                    self.common_run_args(expected=1, log_name=f"{label}.log"),
                    capture_output=True, text=True, check=False,
                )
                self.assertNotEqual(result.returncode, 0, result.stdout)

    def test_case_ids_require_exact_unique_set_even_when_results_match(self):
        scenarios = [
            (
                "same_value_missing_plus_duplicate",
                {"omit_case_id": "linux_model_clock_consumer",
                 "duplicate_case_id": "notes-host-preview.save"},
            ),
            ("unknown_id", {"unknown_case_id": "linux_model_clock_consumer"}),
            ("old_format_without_case_ids", {"case_results": False}),
            ("malformed_marker_mixed_with_fixed_ids", {"malformed_case_marker": True}),
            ("wrong_outcome_for_exit", {"wrong_case_outcome_id": "in_process_echo"}),
            ("runner_io_error", {"output_error": 5}),
        ]
        for label, extra in scenarios:
            with self.subTest(scenario=label):
                fake_qemu(
                    self.qemu,
                    runner_status=1,
                    failure_entries={"in_process_echo": 23},
                    **extra,
                )
                result = subprocess.run(
                    self.common_run_args(expected=1, log_name=f"case-id-{label}.log"),
                    capture_output=True, text=True, check=False,
                )
                self.assertNotEqual(result.returncode, 0, result.stdout)

    def test_crlf_serial_logs_are_parsed(self):
        fake_qemu(self.qemu, crlf_serial=True)
        result = subprocess.run(
            self.common_run_args(log_name="crlf-serial.log"),
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(result.stdout.strip().splitlines()[-1])
        self.assertEqual(payload["entry_groups"], {"pass": 9, "fail": 0})

    def test_status_mismatch_qemu_exit_and_timeout_are_failures(self):
        fake_qemu(self.qemu, runner_status=1)
        mismatch = subprocess.run(self.common_run_args(expected=0, log_name="status-mismatch.log"),
                                  capture_output=True, text=True, check=False)
        self.assertEqual(mismatch.returncode, 1)

        fake_qemu(self.qemu, qemu_status=7)
        qemu_exit = subprocess.run(
            self.common_run_args(log_name="qemu-exit-mismatch.log"),
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(qemu_exit.returncode, 1)

        fake_qemu(self.qemu, delay=10)
        timeout_args = self.common_run_args(log_name="timeout.log")
        timeout_args[timeout_args.index("--timeout-seconds") + 1] = "1"
        timeout = subprocess.run(timeout_args, capture_output=True, text=True, check=False)
        self.assertEqual(timeout.returncode, 1)
        self.assertIn("timed_out", timeout.stderr)

    def test_missing_or_duplicate_entry_and_guest_io_error_cannot_pass(self):
        fake_qemu(self.qemu, entry_count=8)
        missing_entry = subprocess.run(
            self.common_run_args(log_name="missing-entry.log"),
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(missing_entry.returncode, 1)

        fake_qemu(self.qemu, duplicate_name=True)
        duplicate_entry = subprocess.run(
            self.common_run_args(log_name="duplicate-entry.log"),
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(duplicate_entry.returncode, 1)

        fake_qemu(self.qemu, output_error=5)
        guest_io_error = subprocess.run(
            self.common_run_args(log_name="guest-io-error.log"),
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(guest_io_error.returncode, 1)

        fake_qemu(self.qemu, substitute_last_name=True)
        substituted_entry_args = self.common_run_args(log_name="substituted-entry.log")
        substituted_entry = subprocess.run(
            substituted_entry_args, capture_output=True, text=True, check=False,
        )
        self.assertEqual(substituted_entry.returncode, 1)

    def test_log_io_error_returns_failure(self):
        fake_qemu(self.qemu)
        subject = load_subject()
        args = self.common_run_args(log_name="injected-log-io-error.log")[3:]
        output = io.StringIO()
        error_output = io.StringIO()
        with mock.patch.object(subject.os, "fsync", side_effect=OSError(errno.EIO, "injected fsync")):
            with contextlib.redirect_stdout(output), contextlib.redirect_stderr(error_output):
                result = subject.main(["run", *args])
        self.assertEqual(result, 1)
        self.assertIn("I/O error syncing QEMU log", error_output.getvalue())

    def test_fixture_builder_produces_both_readback_verified_archives(self):
        output = self.root / "fixtures"
        result = subprocess.run([
            sys.executable, str(CANDIDATE), "build-fixtures",
            "--task-root", str(self.root),
            "--output-dir", str(output),
            "--bundle", str(self.base_bundle),
            "--runner-elf", str(self.runner_elf),
            "--fail-child-elf", str(self.fail_child_elf),
        ], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(result.stdout.strip().splitlines()[-1])
        positive = Path(payload["positive_fixture"]).read_bytes()
        negative = Path(payload["negative_fixture"]).read_bytes()
        self.assertEqual(payload["positive_sha256"], hashlib.sha256(positive).hexdigest())
        self.assertEqual(payload["negative_sha256"], hashlib.sha256(negative).hexdigest())

    def test_public_initramfs_builder_uses_explicit_sources_and_hides_host_paths(self):
        output = self.root / "initramfs-output"
        command = [
            sys.executable, str(BUILDER_PATH),
            "--init", str(self.init_elf),
            "--workspace", str(self.root),
            "--output-dir", str(output),
        ]
        for guest_name in self.builder.REQUIRED_SMOKE_NAMES:
            command.extend(("--smoke", f"{guest_name}={self.runner_elf}"))
        result = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn(str(self.root), result.stdout)
        self.assertIn("initramfs=initramfs.cpio", result.stdout)
        self.assertIn("guest_path=init", result.stdout)

        archive = (output / "initramfs.cpio").read_bytes()
        parsed = self.builder.parse_newc(archive)
        self.assertEqual(
            [item["name"] for item in parsed[:-1]],
            [
                "bin", "dev", "dev/console", "dev/null", "dev/ttyS0", "proc", "sys",
                "run", "tmp", "init", *self.builder.REQUIRED_SMOKE_NAMES,
            ],
        )

        missing_output = self.root / "missing-required-output"
        missing = subprocess.run([
            sys.executable, str(BUILDER_PATH),
            "--init", str(self.init_elf),
            "--workspace", str(self.root),
            "--output-dir", str(missing_output),
        ], capture_output=True, text=True, check=False)
        self.assertEqual(missing.returncode, 1)
        self.assertFalse(missing_output.exists())
        self.assertIn("every required guest path", missing.stderr)

        duplicate_output = self.root / "duplicate-output"
        duplicate = [
            sys.executable, str(BUILDER_PATH),
            "--init", str(self.init_elf),
            "--workspace", str(self.root),
            "--output-dir", str(duplicate_output),
            "--smoke", f"{self.builder.REQUIRED_SMOKE_NAMES[0]}={self.runner_elf}",
            "--smoke", f"{self.builder.REQUIRED_SMOKE_NAMES[0]}={self.runner_elf}",
        ]
        rejected = subprocess.run(duplicate, capture_output=True, text=True, check=False)
        self.assertEqual(rejected.returncode, 1)
        self.assertFalse(duplicate_output.exists())
        self.assertIn("repeats guest path", rejected.stderr)

        non_normalized_output = self.root / ".." / f"unsafe-{self.root.name}"
        non_normalized = subprocess.run([
            sys.executable, str(BUILDER_PATH),
            "--init", str(self.init_elf),
            "--workspace", str(self.root),
            "--output-dir", str(non_normalized_output),
        ], capture_output=True, text=True, check=False)
        self.assertEqual(non_normalized.returncode, 1)
        self.assertFalse(non_normalized_output.exists())
        self.assertIn("absolute normalized path", non_normalized.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
