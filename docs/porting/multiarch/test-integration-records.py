"""Composition integrity tests; the tiny historical verifier tests orchestration."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("integration", Path(__file__).with_name("verify-integration.py"))
v = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(v)


class CompositionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="pr57-composition-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "-q")
        self.git("config", "user.name", "fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.write(v.STATUS, "Original unrelated milestone text\n")
        self.write("experiments/nagi-aarch64-spike/run.py", "# original fixture\n")
        self.write(v.DOC + "verify-records.py", "# Historical verifier orchestration fixture, not real acceptance.\nraise SystemExit(0)\n")
        self.base = self.save("base")
        self.spike = self.git("rev-parse", self.base + ":experiments/nagi-aarch64-spike")
        self.summaries = [v.DOC + "runtime/first/summary.json", v.DOC + "runtime/second/summary.json"]
        for path in self.summaries:
            self.write(path, json.dumps({"source_commit": self.base, "source_tree": self.anchor(self.base)["tree"],
                       "main": self.base, "source_spike_subtree": self.spike, "acceptance": {"phase_c": "PENDING"}}))
        self.write(v.DOC + "state.json", json.dumps({"last_verified": {"commit_sha": self.base}}))
        self.records = self.save("historical records")
        patch = v.git(self.root, "diff", "--binary", self.base, self.records)
        self.git("checkout", "-q", self.base)
        self.write(v.STATUS, "## PR #57 independent ARM64 host boundary candidate\n\nHost proposal.\n\nOriginal unrelated milestone text\n")
        self.write("experiments/nagi-aarch64-spike/run.py", "# new fixture\n")
        self.source = self.save("new source")
        v.git(self.root, "apply", "--index", "-", data=patch)
        self.manifest = {"schema": "nagi-multiarch-source-record-composition/1",
            "base": self.anchor(self.base), "current_source": self.anchor(self.source),
            "historical_records": self.anchor(self.records),
            "historical_runtime": {"source": self.anchor(self.base), "main": self.base,
                                   "spike_tree": self.spike, "summaries": self.summaries},
            "root_status": {"path": v.STATUS, "base_sha256": self.digest(self.base, v.STATUS),
                            "source_sha256": self.digest(self.source, v.STATUS),
                            "change_kind": "prepend-only-exact-reviewed-source"},
            "acceptance": {"current_source_runtime": "NOT_RUN", "phase_c": "PENDING", "physical": "NOT_RUN",
                           "owner_scope_adoption": "PENDING", "scope_owner": "Integration Owner/os-core"}}
        for path in v.EXTRA:
            self.write(path, "# composition fixture\n")
        self.write_manifest()
        self.save("composition")

    def git(self, *args):
        return v.text(self.root, *args)

    def write(self, path, data):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(data)

    def save(self, message):
        self.git("add", "-A")
        self.git("commit", "-qm", message)
        return self.git("rev-parse", "HEAD")

    def anchor(self, revision):
        return {"commit": revision, "tree": self.git("rev-parse", revision + "^{tree}")}

    def digest(self, revision, path):
        return hashlib.sha256(v.blob(self.root, revision, path)).hexdigest()

    def write_manifest(self):
        self.write(v.MANIFEST, json.dumps(self.manifest))

    def validate(self):
        return v.validate(self.root, self.base, self.source, self.records)

    def test_exact_overlay_passes_without_promoting_acceptance(self):
        result = self.validate()
        self.assertEqual(result["integrity"], "PASS")
        self.assertEqual(result["runtime_source"], self.base)
        self.assertEqual(result["current_source_runtime"], "NOT_RUN")
        self.assertEqual(result["owner_scope_adoption"], "PENDING")

    def test_tree_and_commit_anchor_substitution_rejected(self):
        for field, value in [("tree", "0" * 40), ("commit", self.base)]:
            with self.subTest(field=field):
                saved = copy.deepcopy(self.manifest)
                self.manifest["current_source"][field] = value
                self.write_manifest(); self.save("bad anchor")
                with self.assertRaises(v.IntegrationError): self.validate()
                self.manifest = saved

    def test_short_cli_anchor_rejected(self):
        with self.assertRaisesRegex(v.IntegrationError, "explicit full"):
            v.validate(self.root, self.base[:8], self.source, self.records)

    def test_dirty_or_untracked_content_rejected(self):
        self.write("unexpected.txt", "not tracked")
        with self.assertRaisesRegex(v.IntegrationError, "clean"): self.validate()

    def test_historical_evidence_tamper_rejected(self):
        self.write(self.summaries[0], "{}")
        self.save("tamper historical evidence")
        with self.assertRaisesRegex(v.IntegrationError, "exact source/records overlay"): self.validate()

    def test_extra_unrelated_committed_file_rejected(self):
        self.write("kernel/unrelated.rs", "// unrelated")
        self.save("unrelated file")
        with self.assertRaisesRegex(v.IntegrationError, "exact source/records overlay"): self.validate()

    def test_missing_historical_file_rejected(self):
        (self.root / self.summaries[0]).unlink()
        self.save("drop historical file")
        with self.assertRaisesRegex(v.IntegrationError, "exact source/records overlay"): self.validate()

    def test_source_file_and_executable_mode_tamper_rejected(self):
        target = self.root / "experiments/nagi-aarch64-spike/run.py"
        target.chmod(0o755)
        self.save("mode change")
        with self.assertRaisesRegex(v.IntegrationError, "exact source/records overlay"): self.validate()
        target.chmod(0o644)
        target.write_text("# wrong source")
        self.save("source change")
        with self.assertRaisesRegex(v.IntegrationError, "exact source/records overlay"): self.validate()

    def test_root_status_change_and_digest_tamper_rejected(self):
        self.manifest["root_status"]["source_sha256"] = "0" * 64
        self.write_manifest(); self.save("wrong status hash")
        with self.assertRaisesRegex(v.IntegrationError, "digest mismatch"): self.validate()
        self.manifest["root_status"]["source_sha256"] = self.digest(self.source, v.STATUS)
        self.write_manifest()
        self.write(v.STATUS, "Claim a different milestone PASS\n")
        self.save("different status")
        with self.assertRaisesRegex(v.IntegrationError, "differs from reviewed source"): self.validate()

    def test_promoting_acceptance_or_ownership_rejected(self):
        for field in ["current_source_runtime", "phase_c", "physical", "owner_scope_adoption"]:
            with self.subTest(field=field):
                self.manifest["acceptance"][field] = "PASS"
                self.write_manifest(); self.save("promotion")
                with self.assertRaisesRegex(v.IntegrationError, "promote"): self.validate()
                self.manifest["acceptance"][field] = "PENDING" if field in ["phase_c", "owner_scope_adoption"] else "NOT_RUN"

    def test_rebinding_historical_runtime_to_current_source_rejected(self):
        self.manifest["historical_runtime"]["source"] = self.anchor(self.source)
        self.write_manifest(); self.save("relabel historical source")
        with self.assertRaisesRegex(v.IntegrationError, "relabeled"): self.validate()

    def test_historical_spike_subtree_substitution_rejected(self):
        self.manifest["historical_runtime"]["spike_tree"] = "0" * 40
        self.write_manifest(); self.save("wrong subtree")
        with self.assertRaisesRegex(v.IntegrationError, "subtree mismatch"): self.validate()

    def test_path_traversal_and_duplicate_summaries_rejected(self):
        for paths in [[self.summaries[0], self.summaries[0]], [v.DOC + "runtime/../summary.json", self.summaries[1]]]:
            self.manifest["historical_runtime"]["summaries"] = paths
            self.write_manifest(); self.save("wrong summary paths")
            with self.assertRaises(v.IntegrationError): self.validate()

    def recompose_source(self, path, contents):
        self.git("checkout", "-q", self.source)
        self.write(path, contents)
        self.source = self.save("alternative reviewed source")
        delta = v.git(self.root, "diff", "--binary", self.base, self.records)
        v.git(self.root, "apply", "--index", "-", data=delta)
        self.manifest["current_source"] = self.anchor(self.source)
        self.manifest["root_status"]["source_sha256"] = self.digest(self.source, v.STATUS)
        for extra in v.EXTRA: self.write(extra, "# fixture\n")
        self.write_manifest(); self.save("alternative composition")

    def test_root_status_rewrite_rejected_even_with_correct_anchor_and_hash(self):
        self.recompose_source(v.STATUS, "## PR #57 independent ARM64 host boundary candidate\nUnrelated old milestone changed\n")
        with self.assertRaisesRegex(v.IntegrationError, "rewrites existing content"): self.validate()

    def test_current_source_cannot_expand_into_kernel_scope(self):
        self.recompose_source("kernel/unrelated.rs", "// newly pinned but out of scope")
        with self.assertRaisesRegex(v.IntegrationError, "source changes outside"): self.validate()

    def test_pinned_source_cannot_replace_old_verifier(self):
        self.recompose_source(v.DOC + "verify-records.py", "# replacement\nraise SystemExit(0)\n")
        with self.assertRaisesRegex(v.IntegrationError, "schema/verifier changes"): self.validate()

    def test_historical_verifier_failure_is_propagated_and_worktree_removed(self):
        original = subprocess.run
        def fail_history(command, **kwargs):
            if command[0] == v.sys.executable:
                return subprocess.CompletedProcess(command, 2, "", "historical rejection fixture")
            return original(command, **kwargs)
        with patch.object(v.subprocess, "run", side_effect=fail_history):
            with self.assertRaisesRegex(v.IntegrationError, "historical records verifier rejected"): self.validate()
        self.assertEqual(self.git("worktree", "list", "--porcelain").count("worktree "), 1)


if __name__ == "__main__":
    unittest.main()
