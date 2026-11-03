import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from audit import AuditError, full_commit, inventory, write_or_check


def command(root, *args):
    return subprocess.check_output(["git", *args], cwd=root, text=True).strip()


class AuditTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="pr57-audit-", dir=os.environ.get("TMPDIR"))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        subprocess.run(["git", "config", "user.name", "fixture"], cwd=self.root, check=True)
        subprocess.run(["git", "config", "user.email", "fixture@example.invalid"], cwd=self.root, check=True)
        source = self.root / "kernel/src/lib.rs"
        source.parent.mkdir(parents=True)
        source.write_text("pub fn syscall_entry() {}\n")
        subprocess.run(["git", "add", "kernel/src/lib.rs"], cwd=self.root, check=True)
        subprocess.run(["git", "commit", "-qm", "baseline"], cwd=self.root, check=True)
        self.base = command(self.root, "rev-parse", "HEAD")

    def test_explicit_full_sha_inventory_needs_no_origin_remote(self):
        result = inventory(self.root, self.base)
        self.assertEqual(result["baseline_sha"], self.base)
        self.assertEqual(result["source_commit_sha"], self.base)
        self.assertEqual(result["source_tree_sha"], command(self.root, "rev-parse", "HEAD^{tree}"))
        self.assertEqual([item["path"] for item in result["files"]], ["kernel/src/lib.rs"])
        self.assertFalse((self.root / ".git/config").read_text().find("remote") >= 0)

    def test_missing_short_and_nonexistent_base_are_rejected(self):
        for value in (None, "", self.base[:12], "f" * 40):
            with self.subTest(value=value):
                with self.assertRaises(AuditError):
                    full_commit(self.root, value)

    def test_non_ancestor_base_is_rejected(self):
        subprocess.run(["git", "checkout", "--orphan", "unrelated"], cwd=self.root, check=True, stdout=subprocess.DEVNULL)
        (self.root / "other.txt").write_text("unrelated\n")
        subprocess.run(["git", "add", "other.txt"], cwd=self.root, check=True)
        subprocess.run(["git", "commit", "-qm", "unrelated"], cwd=self.root, check=True)
        with self.assertRaisesRegex(AuditError, "ancestor"):
            inventory(self.root, self.base)

    def test_tracked_dirty_source_is_rejected(self):
        (self.root / "kernel/src/lib.rs").write_text("changed\n")
        with self.assertRaisesRegex(AuditError, "dirty"):
            inventory(self.root, self.base)

    def test_guard_untracked_is_reported_separately(self):
        (self.root / ".entire").mkdir()
        (self.root / ".entire/config.json").write_text("{}\n")
        result = inventory(self.root, self.base)
        self.assertTrue(result["source_status"]["tracked_clean"])
        self.assertEqual(result["source_status"]["local_guard_untracked_roots"], [".entire"])
        self.assertEqual(result["source_status"]["local_guard_untracked_count"], 1)

    def test_other_untracked_source_is_rejected(self):
        (self.root / "unexpected.txt").write_text("untracked\n")
        with self.assertRaisesRegex(AuditError, "untracked"):
            inventory(self.root, self.base)

    def test_check_mode_compares_exact_generated_bytes(self):
        output = self.root / "inventory.json"
        write_or_check(output, "{\n}\n", check=False)
        write_or_check(output, "{\n}\n", check=True)
        with self.assertRaises(AuditError):
            write_or_check(output, "{}\n", check=True)


if __name__ == "__main__":
    unittest.main()
