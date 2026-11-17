"""Failure-path checks for the host evaluation orchestrator (subprocesses mocked)."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import run_host_checks as runner


class HostEvaluationTests(unittest.TestCase):
    def test_timeout_and_nonzero_cannot_pass(self):
        with patch.object(runner.subprocess, "run", side_effect=subprocess.TimeoutExpired("tool", 1)):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.run_command(["tool"], runner.ROOT, "portable_run")
            self.assertEqual((caught.exception.phase, caught.exception.code),
                             ("portable_run", "timeout"))
        result = subprocess.CompletedProcess(["tool"], 7, stdout=b"PASS", stderr=b"")
        with patch.object(runner.subprocess, "run", return_value=result):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.run_command(["tool"], runner.ROOT, "portable_build")
            self.assertEqual(caught.exception.code, "exit_7")

    def test_changed_stdout_cannot_pass(self):
        with patch.object(runner, "run_command", side_effect=[(b"one", 1), (b"two", 1)]):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.repeat_command(["tool"], runner.ROOT, "ui_analysis", 2)
            self.assertEqual(caught.exception.code, "nondeterministic_stdout")

    def test_invalid_repeat_count_never_starts_process(self):
        with patch.object(runner.subprocess, "run") as process:
            for count in (0, 1, 21):
                with self.subTest(count=count), self.assertRaises(runner.EvaluationError):
                    runner.evaluate(runner.ROOT, count)
            process.assert_not_called()

    def test_missing_compiler_never_starts_process(self):
        with patch.object(runner.shutil, "which", return_value=None), \
                patch.object(runner.subprocess, "run") as process:
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.evaluate(runner.ROOT, 2)
            self.assertEqual(caught.exception.code, "clang_unavailable")
            process.assert_not_called()

    def test_input_mutation_is_detected_even_after_command_failure(self):
        with tempfile.TemporaryDirectory(prefix="ndp-runner-integrity-") as directory:
            root = Path(directory)
            for path in runner.INPUTS:
                (root / path).parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(runner.ROOT / path, root / path)
            changed = root / runner.INPUTS[0]

            def fail_after_mutation(*args, **kwargs):
                changed.write_bytes(b"changed during failed command")
                raise runner.EvaluationError("portable_build", "exit_1")

            with patch.object(runner.shutil, "which", return_value="clang"), \
                    patch.object(runner, "run_command", side_effect=fail_after_mutation):
                with self.assertRaises(runner.EvaluationError) as caught:
                    runner.evaluate(root, 2)
            self.assertEqual((caught.exception.phase, caught.exception.code),
                             ("input_integrity", "source_changed"))
            self.assertEqual(changed.read_bytes(), b"changed during failed command")


if __name__ == "__main__":
    unittest.main()
