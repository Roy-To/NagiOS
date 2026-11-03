import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

VERIFY_PATH = Path(__file__).resolve().parents[2] / "docs/porting/multiarch/verify-records.py"
SPEC = importlib.util.spec_from_file_location("verify_records", VERIFY_PATH)
verify_records = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verify_records)

HISTORICAL_BASE = verify_records.HISTORICAL_BASE
RecordsError = verify_records.RecordsError
validate_current_main = verify_records.validate_current_main
validate_owned_scope = verify_records.validate_owned_scope
validate_record_commit = verify_records.validate_record_commit
validate_clean_record_worktree = verify_records.validate_clean_record_worktree


def run(root, *args, check=True):
    return subprocess.run(["git", *args], cwd=root, check=check, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)


class RecordBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="pr57-records-", dir=os.environ.get("TMPDIR"))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        run(self.root, "init", "-q")
        run(self.root, "config", "user.name", "fixture")
        run(self.root, "config", "user.email", "fixture@example.invalid")
        (self.root / "README.md").write_text("base\n")
        run(self.root, "add", "README.md")
        run(self.root, "commit", "-qm", "main base")
        self.main = run(self.root, "rev-parse", "HEAD").stdout.strip()
        (self.root / "experiments/nagi-multiarch-contracts").mkdir(parents=True)
        (self.root / "experiments/nagi-multiarch-contracts/audit.py").write_text("pass\n")
        run(self.root, "add", "experiments/nagi-multiarch-contracts/audit.py")
        run(self.root, "commit", "-qm", "source candidate")
        self.source = run(self.root, "rev-parse", "HEAD").stdout.strip()
        (self.root / "docs/porting/multiarch").mkdir(parents=True)
        (self.root / "docs/porting/multiarch/state.json").write_text("{}\n")
        run(self.root, "add", "docs/porting/multiarch/state.json")
        run(self.root, "commit", "-qm", "records only")
        self.records = run(self.root, "rev-parse", "HEAD").stdout.strip()

    def test_records_only_child_is_accepted(self):
        self.assertEqual(validate_record_commit(self.root, self.source, self.records),
                         ["docs/porting/multiarch/state.json"])
        self.assertTrue(validate_owned_scope(self.root, self.main, self.records))

    def test_source_change_in_records_child_is_rejected(self):
        (self.root / "experiments/nagi-multiarch-contracts/audit.py").write_text("changed\n")
        run(self.root, "add", "experiments/nagi-multiarch-contracts/audit.py")
        run(self.root, "commit", "-qm", "unexpected source change")
        head = run(self.root, "rev-parse", "HEAD").stdout.strip()
        with self.assertRaisesRegex(RecordsError, "records-only child"):
            validate_record_commit(self.root, self.source, head)

    def test_non_ancestor_source_is_rejected(self):
        run(self.root, "checkout", "--orphan", "unrelated")
        (self.root / "elsewhere.txt").write_text("unrelated\n")
        run(self.root, "add", "elsewhere.txt")
        run(self.root, "commit", "-qm", "unrelated")
        head = run(self.root, "rev-parse", "HEAD").stdout.strip()
        with self.assertRaisesRegex(RecordsError, "ancestor"):
            validate_record_commit(self.root, self.source, head)

    def test_owned_scope_rejects_nonowned_path(self):
        (self.root / "kernel/src/main.rs").parent.mkdir(parents=True)
        (self.root / "kernel/src/main.rs").write_text("kernel change\n")
        run(self.root, "add", "kernel/src/main.rs")
        run(self.root, "commit", "-qm", "outside scope")
        head = run(self.root, "rev-parse", "HEAD").stdout.strip()
        with self.assertRaisesRegex(RecordsError, "outside"):
            validate_owned_scope(self.root, self.main, head)

    def test_historical_base_is_never_current(self):
        with self.assertRaisesRegex(RecordsError, "historical"):
            validate_current_main(self.root, HISTORICAL_BASE, self.main)

    def test_inventory_base_must_equal_explicit_main(self):
        with self.assertRaisesRegex(RecordsError, "equal"):
            validate_current_main(self.root, self.source, self.main)

    def test_tracked_dirty_is_rejected(self):
        (self.root / "README.md").write_text("dirty\n")
        with self.assertRaisesRegex(RecordsError, "dirty"):
            validate_clean_record_worktree(self.root)

    def test_guard_untracked_and_other_untracked_are_separate(self):
        (self.root / ".codex").mkdir()
        (self.root / ".codex/config").write_text("local\n")
        result = validate_clean_record_worktree(self.root)
        self.assertTrue(result["tracked_clean"])
        self.assertEqual(result["local_guard_untracked_roots"], [".codex"])
        self.assertEqual(result["local_guard_untracked_count"], 1)
        (self.root / "outside.txt").write_text("not guard config\n")
        with self.assertRaisesRegex(RecordsError, "unexpected untracked"):
            validate_clean_record_worktree(self.root)


if __name__ == "__main__":
    unittest.main()
