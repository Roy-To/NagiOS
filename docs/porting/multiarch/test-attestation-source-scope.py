"""Source-contract fixtures only; no receipt, build or guest acceptance is claimed."""
import hashlib
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
SPIKE = "experiments/nagi-aarch64-spike"
sys.path.insert(0, str(ROOT / SPIKE))
SPEC = importlib.util.spec_from_file_location("original_attestation_verifier", ROOT / SPIKE / "verify_attestation.py")
v = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(v)


class SourceScopeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="pr57-source-scope-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "-q")
        self.git("config", "user.name", "fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.write("docs/implementation_status.md", b"unchanged reference status\n")
        self.main = self.commit("main")
        self.write(SPIKE + "/attest.py", b"# generator fixture\n")
        self.write(SPIKE + "/run.py", b"# runner fixture\n")
        self.write("docs/implementation_status.md", b"new host note\nunchanged reference status\n")
        self.original = self.commit("source containing root status change")
        self.write("docs/implementation_status.md", b"unchanged reference status\n")
        self.source = self.commit("separate root status from source")

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root).decode().strip()

    def write(self, path, data):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)

    def commit(self, message):
        self.git("add", "-A")
        self.git("commit", "-qm", message)
        return self.git("rev-parse", "HEAD")

    def record(self, source):
        # Synthetic source metadata exercises check_source only. This is not a
        # receipt and contains no build/run records or evidence of execution.
        tree = self.git("rev-parse", source + "^{tree}")
        record = {"source": {"commit": source, "tree": tree, "main": self.main,
                  "tracked_clean": True, "source_clean": True,
                  "head_after_build": source, "tree_after_build": tree}}
        for key, name in [("generator", "attest.py"), ("runner", "run.py")]:
            path = SPIKE + "/" + name
            content = subprocess.check_output(["git", "show", source + ":" + path], cwd=self.root)
            record[key] = {"path": path, "git_blob": self.git("rev-parse", source + ":" + path),
                           "sha256": hashlib.sha256(content).hexdigest()}
        return record

    def check(self, source=None, record=None):
        source = source or self.source
        return v.check_source(self.root, record or self.record(source), self.main, source,
                              self.git("rev-parse", source + "^{tree}"))

    def test_original_source_with_root_note_still_rejects(self):
        with self.assertRaisesRegex(v.VerificationError, "source commit contains out-of-scope paths: docs/implementation_status.md"):
            self.check(self.original)

    def test_distinct_source_with_same_spike_subtree_meets_source_contract(self):
        self.assertNotEqual(self.source, self.original)
        self.assertNotEqual(self.git("rev-parse", self.source + "^{tree}"), self.git("rev-parse", self.original + "^{tree}"))
        self.assertEqual(self.git("rev-parse", self.source + ":" + SPIKE), self.git("rev-parse", self.original + ":" + SPIKE))
        self.assertEqual(self.check()[0], self.source)

    def test_same_subtree_does_not_allow_old_receipt_source_rebinding(self):
        with self.assertRaisesRegex(v.VerificationError, "source commit differs"):
            self.check(record=self.record(self.original))

    def test_new_commit_with_old_full_tree_still_rejects(self):
        record = self.record(self.source)
        record["source"]["tree"] = self.git("rev-parse", self.original + "^{tree}")
        with self.assertRaisesRegex(v.VerificationError, "tree does not match"):
            self.check(record=record)

    def test_root_note_cannot_be_attached_as_records_only_child(self):
        self.write("docs/implementation_status.md", b"later root note\n")
        self.commit("root note after source")
        with self.assertRaisesRegex(v.VerificationError, "record-only child changed source paths"):
            self.check()

    def test_owned_document_child_retains_source_binding(self):
        self.write("docs/porting/multiarch/source-note.md", b"source-bound records note\n")
        self.commit("owned document child")
        self.assertEqual(self.check()[0], self.source)

    def test_unrelated_kernel_change_cannot_enter_new_source(self):
        self.write("kernel/unrelated.rs", b"// out of scope\n")
        other = self.commit("unrelated source")
        with self.assertRaisesRegex(v.VerificationError, "out-of-scope paths: kernel/unrelated.rs"):
            self.check(other)

    def test_main_cannot_be_changed_in_record_to_hide_scope(self):
        record = self.record(self.original)
        record["source"]["main"] = self.original
        with self.assertRaisesRegex(v.VerificationError, "main differs from explicit"):
            self.check(self.original, record)


if __name__ == "__main__":
    unittest.main()
