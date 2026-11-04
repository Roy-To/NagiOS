"""Model-free tests for public-summary validation, not proof of guest execution."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("runtime_records", Path(__file__).with_name("verify-records.py"))
v = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(v)


class RuntimeSummaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR"))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        def git(*args):
            return subprocess.check_output(["git", *args], cwd=self.root, stderr=subprocess.DEVNULL, text=True).strip()
        git("init", "-q")
        git("config", "user.name", "fixture")
        git("config", "user.email", "fixture@example.invalid")
        spike = self.root / "experiments/nagi-aarch64-spike"
        spike.mkdir(parents=True)
        (spike / "fixture.txt").write_text("model-free fixture\n")
        git("add", ".")
        git("commit", "-qm", "fixture source")
        self.source = git("rev-parse", "HEAD")
        self.tree = git("rev-parse", "HEAD^{tree}")
        self.path = self.root / "docs/porting/multiarch/runtime/fixture/summary.json"
        self.path.parent.mkdir(parents=True)
        serials = {"el1": b"SPIKE ARM64 EL1 PL011 PASS\nSPIKE SVC VECTOR ERET PASS\nSPIKE GENERIC COUNTER PASS\nSPIKE COMPLETE PASS\n", "el2": b"SPIKE FAIL exception-level\n"}
        self.record = {"schema": "nagi-multiarch-runtime-followup/1", "source_commit": self.source,
            "source_tree": self.tree, "main": self.source, "source_tracked_clean": True,
            "source_spike_subtree": git("rev-parse", "HEAD:experiments/nagi-aarch64-spike"),
            "receipt_integrity": {"exit_code": 0, "raw_attestation_sha256": "a" * 64, "verifier_stdout_sha256": "b" * 64},
            "acceptance": {"el1": "PASS", "el2_negative": "MET", "source_bound_runs": "PASS",
                "two_build_comparison": "MATCHED", "phase_c": "PENDING", "independent_rebuild": "NOT_RUN"},
            "builds": [{"id": name, "exit_code": 0, "elf_machine": 183, "elf_bytes": 64, "elf_sha256": "c" * 64} for name in ("a", "b")],
            "qemu": {"version": "QEMU emulator version fixture", "sha256": "d" * 64}, "runs": []}
        for name, code, outcome, verdict in (("el1", 0, "PASS", "PASS"), ("el2", 1, "FAIL", "MET")):
            run = {"id": name, "runner_exit_code": code, "qemu_exit_code": code, "timed_out": False,
                "run_outcome": outcome, "acceptance_verdict": verdict}
            for kind, data in (("serial", serials[name]), ("stderr", b"")):
                filename = name + "-" + kind + ".log"
                (self.path.parent / filename).write_bytes(data)
                run[kind + "_path"] = filename
                run[kind + "_sha256"] = hashlib.sha256(data).hexdigest()
            self.record["runs"].append(run)
        self.status = {"runtime_record": {"path": self.path.relative_to(self.root).as_posix(), "sha256": "0" * 64,
            "raw_attestation_sha256": "a" * 64, "verifier_stdout_sha256": "b" * 64}}

    def verify(self):
        self.path.write_text(json.dumps(self.record))
        self.status["runtime_record"]["sha256"] = hashlib.sha256(self.path.read_bytes()).hexdigest()
        return v.validate_runtime_record(self.root, self.status, self.source, self.source, self.tree)

    def test_complete_summary_is_accepted_without_execution(self):
        self.assertEqual(self.verify()["acceptance"]["phase_c"], "PENDING")

    def test_source_substitution_is_rejected(self):
        self.record["source_commit"] = "0" * 40
        with self.assertRaisesRegex(v.RecordsError, "identity"):
            self.verify()

    def test_phase_c_or_independent_pass_is_rejected(self):
        for field in ("phase_c", "independent_rebuild"):
            original = copy.deepcopy(self.record)
            self.record["acceptance"][field] = "PASS"
            with self.assertRaisesRegex(v.RecordsError, "pending independent"):
                self.verify()
            self.record = original

    def test_different_builds_are_rejected(self):
        self.record["builds"][1]["elf_sha256"] = "e" * 64
        with self.assertRaisesRegex(v.RecordsError, "hashes differ"):
            self.verify()

    def test_log_tampering_is_rejected(self):
        (self.path.parent / "el1-serial.log").write_bytes(b"tampered")
        with self.assertRaisesRegex(v.RecordsError, "hash mismatch"):
            self.verify()

    def test_el2_complete_even_with_updated_hash_is_rejected(self):
        data = b"SPIKE FAIL exception-level\nSPIKE COMPLETE PASS\n"
        (self.path.parent / "el2-serial.log").write_bytes(data)
        self.record["runs"][1]["serial_sha256"] = hashlib.sha256(data).hexdigest()
        with self.assertRaisesRegex(v.RecordsError, "serial markers"):
            self.verify()

    def test_el2_timeout_is_rejected(self):
        self.record["runs"][1]["timed_out"] = True
        with self.assertRaisesRegex(v.RecordsError, "outcome"):
            self.verify()

    def test_boolean_exit_code_is_rejected(self):
        self.record["runs"][0]["runner_exit_code"] = False
        with self.assertRaisesRegex(v.RecordsError, "outcome"):
            self.verify()

    def test_receipt_verification_substitution_is_rejected(self):
        self.record["receipt_integrity"]["raw_attestation_sha256"] = "e" * 64
        with self.assertRaisesRegex(v.RecordsError, "binding"):
            self.verify()

    def test_escaping_path_is_rejected(self):
        self.status["runtime_record"]["path"] = "docs/porting/multiarch/runtime/../summary.json"
        with self.assertRaisesRegex(v.RecordsError, "runtime directory"):
            self.verify()

    def test_symlink_log_is_rejected(self):
        path = self.path.parent / "el1-serial.log"
        data = path.read_bytes()
        path.unlink()
        outside = self.root / "external.log"
        outside.write_bytes(data)
        path.symlink_to(outside)
        with self.assertRaisesRegex(v.RecordsError, "hash mismatch"):
            self.verify()


if __name__ == "__main__":
    unittest.main()
