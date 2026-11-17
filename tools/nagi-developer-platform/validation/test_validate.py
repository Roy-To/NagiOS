import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import validate

FIXTURE = validate.REPO / "tests/developer-platform/validation/minimal-third-party.json"


class InputTests(unittest.TestCase):
    def test_paths_links_fifo_size_and_input_preservation(self):
        with tempfile.TemporaryDirectory(prefix="ndp-input-") as directory:
            root = Path(directory)
            data = FIXTURE.read_bytes()
            (root / "input.json").write_bytes(data)
            (root / "link.json").symlink_to(root / "input.json")
            (root / "linked").symlink_to(root, target_is_directory=True)
            os.mkfifo(root / "fifo")
            (root / "large.json").write_bytes(b"x" * (validate.MAX_INPUT + 1))
            self.assertEqual(validate.read_manifest(root, "input.json"), data)
            for path in ("../input.json", str(root / "input.json"), "link.json",
                         "linked/input.json", "fifo", "missing", "large.json", "bad\x00name"):
                with self.subTest(path=path), self.assertRaises(validate.InputError):
                    validate.read_manifest(root, path)
            self.assertEqual((root / "input.json").read_bytes(), data)

    def test_unconnected_stage_never_reads_or_executes(self):
        with patch.object(validate, "read_manifest", side_effect=AssertionError("must not read")), \
                patch.object(validate.subprocess, "Popen", side_effect=AssertionError("must not run")):
            for stage in (2, 3):
                result = validate.validate("/unavailable", "missing", stage=stage)
                self.assertEqual(result["status"], "not_run")

    def test_invalid_timeout_never_starts_process(self):
        with patch.object(validate.subprocess, "Popen") as process:
            for timeout in (0, -1, 61, float("nan"), float("inf")):
                self.assertEqual(validate.validate("/missing", "missing", timeout=timeout)["reason"],
                                 "invalid_timeout")
            process.assert_not_called()


class ProcessTests(unittest.TestCase):
    def run_child(self, source, **kwargs):
        return validate._run([sys.executable, "-B", "-c", source], tempfile.gettempdir(),
                             kwargs.pop("timeout", 2), **kwargs)

    def test_success_nonzero_and_unavailable(self):
        result = self.run_child('import sys; print("ok"); sys.stderr.write("diagnostic"); sys.exit(7)')
        self.assertEqual((result["status"], result["exit_code"], result["stdout"]),
                         ("exited", 7, b"ok\n"))
        result = validate._run(["/not/a/real/executable"], tempfile.gettempdir(), 1)
        self.assertEqual(result["status"], "unavailable")

    def test_timeout_kills_child(self):
        start = time.monotonic()
        result = self.run_child('import time; time.sleep(30)', timeout=0.15)
        self.assertEqual(result["status"], "timeout")
        self.assertIsNotNone(result["exit_code"])
        self.assertLess(time.monotonic() - start, 3)

    def test_running_and_prestart_cancellation(self):
        event = threading.Event()
        timer = threading.Timer(0.15, event.set)
        timer.start()
        try:
            result = self.run_child('import time; time.sleep(30)', cancel=event)
            self.assertEqual(result["status"], "cancelled")
        finally:
            timer.cancel()
        with patch.object(validate.subprocess, "Popen") as process:
            result = self.run_child('raise RuntimeError("never run")', cancel=event)
            self.assertEqual(result["status"], "cancelled")
            process.assert_not_called()

    def test_both_streams_are_bounded_and_exact_limit_succeeds(self):
        result = self.run_child('import os; os.write(1,b"x"*32); os.write(2,b"y"*32)', output_limit=64)
        self.assertEqual(result["status"], "exited")
        self.assertEqual(len(result["stdout"]) + len(result["stderr"]), 64)
        result = self.run_child('import os; os.write(1,b"x"*32); os.write(2,b"y"*33)', output_limit=64)
        self.assertEqual(result["status"], "output_limit")
        self.assertLessEqual(len(result["stdout"]) + len(result["stderr"]), 64)
        result = self.run_child('import os\nwhile True: os.write(2,b"secret"*8192)', output_limit=256)
        self.assertEqual(result["status"], "output_limit")
        self.assertLessEqual(len(result["stderr"]), 256)

    def test_descendant_keeping_pipe_open_obeys_deadline(self):
        result = self.run_child('import subprocess,sys; subprocess.Popen([sys.executable,"-c",'
                                '"import time; time.sleep(30)"])', timeout=0.2)
        self.assertEqual(result["status"], "timeout")


class AdapterTests(unittest.TestCase):
    def test_missing_tool_and_malformed_output_cannot_pass(self):
        with patch.object(validate, "VALIDATOR", Path("/not/a/validator")):
            self.assertEqual(validate.validate(FIXTURE.parent, FIXTURE.name)["status"], "unavailable")
        execution = {"status": "exited", "exit_code": 0, "stdout": b"wrong output", "stderr": b""}
        # Existing Python is only a placeholder for the executable availability check.
        with patch.object(validate, "VALIDATOR", Path(sys.executable).resolve()), \
                patch.object(validate, "_run", return_value=execution):
            result = validate.validate(FIXTURE.parent, FIXTURE.name)
        self.assertEqual(result["reason"], "invalid_tool_output")
        self.assertNotIn("wrong output", json.dumps(result))

    @unittest.skipUnless(validate.VALIDATOR.is_file(), "build the real nagi-pkg first")
    def test_real_validator_accepts_and_rejects_without_source_change(self):
        before = hashlib.sha256(FIXTURE.read_bytes()).digest()
        result = validate.validate(FIXTURE.parent, FIXTURE.name)
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["sdk_contract"], [1, 0])
        self.assertEqual(hashlib.sha256(FIXTURE.read_bytes()).digest(), before)
        with tempfile.TemporaryDirectory(prefix="ndp-actual-validator-") as directory:
            root = Path(directory)
            path = root / 'manifest;$(must-not-run).json'
            document = json.loads(FIXTURE.read_bytes())
            for mutation in ("version", "path", "secret"):
                invalid = dict(document)
                if mutation == "version":
                    invalid["sdkContractVersion"] = {"major": 99, "minor": 0}
                elif mutation == "path":
                    invalid["entrypoint"] = {"kind": "portable", "target": "../outside"}
                else:
                    invalid["unexpected-private-data"] = "secret-marker-should-not-leak"
                data = json.dumps(invalid).encode()
                path.write_bytes(data)
                result = validate.validate(root, path.name)
                self.assertEqual((result["status"], result["exit_code"]), ("failed", 2))
                self.assertNotIn("secret-marker", json.dumps(result))
                self.assertNotIn(directory, json.dumps(result))
                self.assertEqual(path.read_bytes(), data)

    @unittest.skipUnless(validate.VALIDATOR.is_file(), "build the real nagi-pkg first")
    def test_real_cli_success(self):
        result = subprocess.run([sys.executable, "-B", str(Path(validate.__file__)),
                                 "--root", str(FIXTURE.parent), FIXTURE.name],
                                capture_output=True, check=True, timeout=10)
        self.assertEqual(json.loads(result.stdout)["status"], "passed")
        self.assertEqual(result.stderr, b"")


if __name__ == "__main__":
    unittest.main()
