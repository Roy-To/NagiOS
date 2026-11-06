"""Synthetic metadata fixtures only; no runtime execution or receipt is fabricated."""
import hashlib
import io
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("additional", Path(__file__).with_name("verify-additional-runtime.py"))
v = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(v)


class AdditionalTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="pr57-additional-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "-q")
        self.git("config", "user.name", "fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.write("docs/implementation_status.md", "fixed main status\n")
        self.write("experiments/nagi-aarch64-spike/run.py", "# baseline fixture\n")
        self.base = self.save("main")
        self.write(v.DOC + "state.json", "{\"historical\":true}\n")
        self.records = self.save("historical records")
        delta = self.rawgit("diff", "--binary", self.base, self.records)
        self.git("checkout", "-q", self.base)
        self.write("experiments/nagi-aarch64-spike/run.py", "# current source fixture\n")
        self.source = self.save("current source")
        self.rawgit("apply", "--index", "-", data=delta)
        self.record = {"schema": "nagi-multiarch-additional-runtime/1", "base": self.anchor(self.base),
            "current_source": self.anchor(self.source), "historical_records": self.anchor(self.records),
            "runtime_main": self.base, "spike_tree": self.git("rev-parse", self.source + ":experiments/nagi-aarch64-spike"),
            "formal_summary": {"path": None, "sha256": None}, "manual_summary": {"path": None, "sha256": None},
            "reported_observations": {"formal": "OWNER_REPORT_VERIFIER_EXIT_0_AND_33_ARTIFACTS",
                                      "manual": "OWNER_REPORT_MATCHING_ARCHIVE_ELF_EL1_EL2_NO_FORMAL_RECEIPT"},
            "acceptance": dict(v.ACCEPTANCE)}
        for path in v.EXTRAS:
            self.write(path, "fixture metadata\n")
        self.write_record()
        self.save("pending composition")

    def rawgit(self, *args, data=None):
        return subprocess.check_output(["git", *args], input=data, cwd=self.root)

    def git(self, *args):
        return self.rawgit(*args).decode().strip()

    def write(self, path, value):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(value)

    def save(self, message):
        self.git("add", "-A"); self.git("commit", "-qm", message)
        return self.git("rev-parse", "HEAD")

    def anchor(self, revision):
        return {"commit": revision, "tree": self.git("rev-parse", revision + "^{tree}")}

    def write_record(self):
        self.write(v.RECORD, json.dumps(self.record))

    def summary(self, label, mutate=lambda value: None):
        identity = {**self.record["current_source"], "main": self.base, "spike_tree": self.record["spike_tree"]}
        value = {"source": identity, "artifacts": {name: hashlib.sha256(name.encode()).hexdigest() for name in v.FIELDS}}
        if label == "formal":
            value.update(schema="nagi-receipt-verification-summary/1", kind="REPORTED_FORMAL_RECEIPT_VERIFICATION",
                         receipt_sha256="a" * 64, verifier_stdout_sha256="b" * 64,
                         verifier_exit_code=0, manifested_files=33)
        else:
            value.update(schema="nagi-manual-artifact-comparison/1", kind="MANUAL_OBSERVATION_ONLY", formal_receipt_generated=False)
        mutate(value)
        data = json.dumps(value)
        path = v.DIRECTORY + label + "-summary.json"
        self.write(path, data)
        self.record[label + "_summary"] = {"path": path, "sha256": hashlib.sha256(data.encode()).hexdigest()}
        self.write_record(); self.save("summary fixture")

    def validate(self):
        return v.validate(self.root, self.base, self.source, self.records, self.base)

    def test_pending_input_never_becomes_complete(self):
        result = self.validate()
        self.assertEqual(result["record_completeness"], "PENDING_INPUT")
        self.assertEqual(len(result["missing"]), 2)
        self.assertFalse(result["raw_receipt_rechecked_here"])

    def test_complete_fixture_is_metadata_only_and_manual_has_no_receipt(self):
        self.summary("formal"); self.summary("manual")
        result = self.validate()
        self.assertEqual(result["record_completeness"], "COMPLETE_METADATA_ONLY")
        self.assertEqual(result["manual_observation"], "DIGESTS_MATCH_NO_FORMAL_RECEIPT")
        self.assertEqual(result["phase_c"], "PENDING")
        self.assertFalse(result["raw_receipt_rechecked_here"])

    def cli(self):
        with patch.object(v, "ROOT", self.root), patch.object(v.sys, "argv", [
            "verify-additional-runtime.py", "--base", self.base, "--source", self.source,
            "--records", self.records, "--main", self.base,
        ]), patch.object(v.sys, "stdout", io.StringIO()) as output:
            code = v.main()
            return code, json.loads(output.getvalue())

    def test_cli_pending_returns_exit_2(self):
        code, result = self.cli()
        self.assertEqual(code, 2)
        self.assertEqual(result["record_completeness"], "PENDING_INPUT")

    def test_cli_complete_fixture_returns_metadata_only_exit_0(self):
        self.summary("formal"); self.summary("manual")
        code, result = self.cli()
        self.assertEqual(code, 0)
        self.assertEqual(result["record_completeness"], "COMPLETE_METADATA_ONLY")
        self.assertEqual(result["manual_formal_receipt"], "NOT_GENERATED")

    def test_formal_only_keeps_manual_input_pending(self):
        self.summary("formal")
        self.assertEqual(len(self.validate()["missing"]), 1)

    def test_summary_source_substitution_rejected(self):
        self.summary("formal", lambda x: x["source"].update(commit=self.base))
        with self.assertRaisesRegex(ValueError, "source/main/tree/subtree"): self.validate()

    def test_summary_digest_corruption_rejected(self):
        self.summary("formal")
        self.write(v.DIRECTORY + "formal-summary.json", "{}")
        self.save("tamper summary")
        with self.assertRaisesRegex(ValueError, "digest mismatch"): self.validate()

    def test_wrong_tree_anchor_rejected(self):
        self.record["current_source"]["tree"] = "0" * 40
        self.write_record(); self.save("bad tree")
        with self.assertRaisesRegex(ValueError, "tree mismatch"): self.validate()

    def test_wrong_explicit_main_rejected(self):
        with self.assertRaisesRegex(ValueError, "runtime main differs"):
            v.validate(self.root, self.base, self.source, self.records, self.source)

    def test_formal_nonzero_and_wrong_manifest_count_rejected(self):
        self.summary("formal", lambda x: x.update(verifier_exit_code=2))
        with self.assertRaisesRegex(ValueError, "exit 0 / 33"): self.validate()
        self.summary("formal", lambda x: x.update(manifested_files=32))
        with self.assertRaisesRegex(ValueError, "exit 0 / 33"): self.validate()

    def test_manual_pass_claim_or_receipt_fields_rejected(self):
        self.summary("manual", lambda x: x.update(outcome="PASS"))
        with self.assertRaisesRegex(ValueError, "receipt/PASS claims"): self.validate()
        self.summary("manual", lambda x: x.update(formal_receipt_generated=True))
        with self.assertRaisesRegex(ValueError, "without formal receipt"): self.validate()

    def test_mismatching_manual_artifacts_rejected(self):
        self.summary("formal")
        self.summary("manual", lambda x: x["artifacts"].update(elf_sha256="c" * 64))
        with self.assertRaisesRegex(ValueError, "comparison mismatch"): self.validate()

    def test_invalid_hash_rejected_even_with_valid_summary_digest(self):
        self.summary("formal", lambda x: x["artifacts"].update(elf_sha256="unknown"))
        with self.assertRaisesRegex(ValueError, "invalid SHA-256"): self.validate()

    def test_phase_promotion_and_raw_recheck_claim_rejected(self):
        self.record["acceptance"]["phase_c"] = "PASS"
        self.write_record(); self.save("phase promotion")
        with self.assertRaisesRegex(ValueError, "promote acceptance"): self.validate()
        self.record["acceptance"] = {**v.ACCEPTANCE, "raw_receipt_rechecked_here": True}
        self.write_record(); self.save("false raw recheck")
        with self.assertRaisesRegex(ValueError, "promote acceptance"): self.validate()

    def test_historical_evidence_mutation_rejected(self):
        self.write(v.DOC + "state.json", "{}")
        self.save("mutate old artifact")
        with self.assertRaisesRegex(ValueError, "exact input overlay"): self.validate()

    def test_invented_serial_file_rejected(self):
        self.write(v.DIRECTORY + "el1-serial.log", "synthetic expected output")
        self.save("invented serial")
        with self.assertRaisesRegex(ValueError, "invented serial"): self.validate()

    def test_root_status_change_rejected(self):
        self.write("docs/implementation_status.md", "new claim")
        self.save("unrelated root status")
        with self.assertRaisesRegex(ValueError, "exact input overlay"): self.validate()

    def test_summary_path_escape_rejected(self):
        self.record["formal_summary"] = {"path": "../private.json", "sha256": "a" * 64}
        self.write_record(); self.save("unsafe path")
        with self.assertRaisesRegex(ValueError, "fixed supplemental path"): self.validate()


if __name__ == "__main__":
    unittest.main()
