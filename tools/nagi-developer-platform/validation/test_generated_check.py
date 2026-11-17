"""Tests for the NDP-09c generator -> Stage 1 connection (host-only).

The real generator writes to temporary directories. Tests that need the real
`nagi-pkg` are skipped (never passed) when it is not prepared. Patching of
`validate.validate` is used only to observe orchestration, not as evidence of
the real validator.
"""
import json
from pathlib import Path
import unittest
from unittest.mock import patch

import generated_check
import validate

REAL_VALIDATOR = validate.VALIDATOR.is_file() and not validate.VALIDATOR.is_symlink()


class OrchestrationTests(unittest.TestCase):
    def test_validates_only_the_generated_manifest_with_stage_1(self):
        seen = {}

        def fake_validate(root, relative, stage=1, timeout=10, cancel=None):
            path = Path(root) / relative
            seen.update(relative=relative, stage=stage, timeout=timeout,
                        manifest=json.loads(path.read_text(encoding="utf-8")),
                        files=sorted(p.relative_to(root).as_posix()
                                     for p in Path(root).rglob("*") if p.is_file()))
            return {"stage": 1, "check": "manifest", "scope": "host_metadata_only",
                    "status": "passed", "reason": "metadata_valid", "exit_code": 0}

        with patch.object(validate, "validate", fake_validate), \
                patch.object(validate.subprocess, "Popen",
                             side_effect=AssertionError("no process may start")):
            result = generated_check.check_generated(timeout=5)
        self.assertEqual(seen["relative"], "samples/level-zero/manifest.json")
        self.assertEqual((seen["stage"], seen["timeout"]), (1, 5))
        self.assertEqual(seen["manifest"]["displayName"],
                         generated_check.DEFAULT_INPUTS["display_name"])
        # The generator's real output, exactly the contract's file set (no build products).
        self.assertIn("samples/level-zero/src/lib.rs", seen["files"])
        self.assertFalse([f for f in seen["files"] if "target" in f or f.endswith(".lock")])
        self.assertEqual((result["check"], result["generator"], result["built"]),
                         ("generated_manifest", "created", False))

    def test_generation_failure_skips_stage_1_and_is_not_success(self):
        bad = dict(generated_check.DEFAULT_INPUTS, app_id="not a valid id!")
        with patch.object(validate, "validate",
                          side_effect=AssertionError("Stage 1 must not run")):
            result = generated_check.check_generated(bad)
        self.assertEqual((result["status"], result["reason"], result["generator"]),
                         ("failed", "generation_failed", "failed"))
        self.assertTrue(result["generator_reason"])

    def test_protected_output_directory_is_a_generation_failure(self):
        inputs = dict(generated_check.DEFAULT_INPUTS, output_dir="sdk/level-zero")
        result = generated_check.check_generated(inputs)
        self.assertEqual((result["status"], result["generator_reason"]),
                         ("failed", "protected_output_directory"))

    def test_missing_validator_is_unavailable_not_passed(self):
        with patch.object(validate, "VALIDATOR", Path("/nonexistent/nagi-pkg")):
            result = generated_check.check_generated()
            self.assertEqual(generated_check.main([]), 2)
        self.assertEqual((result["status"], result["reason"], result["generator"]),
                         ("unavailable", "validator_unavailable", "created"))

    def test_results_contain_no_absolute_paths(self):
        with patch.object(validate, "VALIDATOR", Path("/nonexistent/nagi-pkg")):
            text = json.dumps(generated_check.check_generated())
        self.assertNotIn(str(validate.REPO), text)
        self.assertNotIn("/tmp", text)
        self.assertNotIn("/var/", text)

    def test_cancelled_before_run_is_cancelled(self):
        import threading
        cancel = threading.Event()
        cancel.set()
        result = generated_check.check_generated(cancel=cancel)
        self.assertEqual(result["status"], "cancelled")


@unittest.skipUnless(REAL_VALIDATOR,
                     "trusted nagi-pkg validator not prepared (see README); generated "
                     "Manifest was NOT validated by the real validator")
class RealValidatorTests(unittest.TestCase):
    def test_generated_scaffold_manifest_passes_real_stage_1(self):
        result = generated_check.check_generated()
        self.assertEqual(result["status"], "passed", result)
        self.assertEqual(result["reason"], "metadata_valid")
        self.assertEqual(result["generator"], "created")
        self.assertFalse(result["built"])


if __name__ == "__main__":
    unittest.main()
