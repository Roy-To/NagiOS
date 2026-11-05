"""Execute the workflow's real Bash steps against isolated Git repositories."""

from __future__ import annotations

import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
CHANGE_SCOPE = WORKFLOW.split("  change-scope:\n", 1)[1].split("\n  ubuntu-host:", 1)[0]
REQUIRED = (
    "Cargo.toml",
    "Cargo.lock",
    "AGENTS.md",
    "docs/Nagi_OS_0.1_Codex_Implementation_Spec.md",
    "docs/implementation_status.md",
)


def step_script(name: str) -> str:
    step = CHANGE_SCOPE.split(f"name: {name}\n", 1)[1]
    block = step.split("        run: |\n", 1)[1]
    lines = []
    for line in block.splitlines():
        if line and not line.startswith("          "):
            break
        lines.append(line[10:])
    return "\n".join(lines) + "\n"


class ChangeScopeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.repo = self.directory / "repo"
        self.repo.mkdir()
        self.env = dict(os.environ)
        # Do not let the caller's Git configuration or repository redirect fixtures.
        for key in list(self.env):
            if key.startswith("GIT_"):
                del self.env[key]
        self.env.update(
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_AUTHOR_NAME="CI fixture",
            GIT_AUTHOR_EMAIL="ci@example.invalid",
            GIT_COMMITTER_NAME="CI fixture",
            GIT_COMMITTER_EMAIL="ci@example.invalid",
        )
        self.git("init", "-q")
        for path in REQUIRED + ("docs/guide.md", "source.rs"):
            self.write(path)
        self.base = self.commit()
        remote = self.directory / "origin.git"
        self.git("clone", "--bare", "--quiet", str(self.repo), str(remote))
        self.git("remote", "add", "origin", str(remote))

    def git(self, *args: str) -> str:
        return subprocess.check_output(
            ["git", *args], cwd=self.repo, env=self.env, text=True,
            stderr=subprocess.PIPE,
        ).strip()

    def write(self, path: str, text: str = "fixture\n") -> None:
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")

    def commit(self) -> str:
        self.git("add", "-A")
        self.git("commit", "-qm", "fixture")
        return self.git("rev-parse", "HEAD")

    def run_step(self, name: str, **env: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", step_script(name)],
            cwd=self.repo, env={**self.env, **env}, capture_output=True, text=True,
        )

    def guard(self) -> subprocess.CompletedProcess:
        return self.run_step("Verify required repository files")

    def scope(self, event: str = "pull_request", base: str | None = None,
              head: str | None = None) -> tuple[subprocess.CompletedProcess, str]:
        output = self.directory / "output"
        output.write_text("", encoding="utf-8")
        result = self.run_step(
            "Classify changed files", EVENT_NAME=event,
            BASE_SHA=self.base if base is None else base,
            BEFORE_SHA=self.base if base is None else base,
            HEAD_SHA=self.git("rev-parse", "HEAD") if head is None else head,
            GITHUB_OUTPUT=str(output),
        )
        return result, output.read_text(encoding="utf-8")

    def test_required_inventory_matches_repository_check(self) -> None:
        source = (ROOT / "tools/nagi-cli/src/commands.rs").read_text(encoding="utf-8")
        check = source.split("fn check_repository_layout(", 1)[1]
        inventory = check.split("let required = [", 1)[1].split("];", 1)[0]
        self.assertEqual(tuple(re.findall(r'"([^"]+)"', inventory)), REQUIRED)
        script = step_script("Verify required repository files")
        self.assertEqual(tuple(script.split("required=(", 1)[1].split(")", 1)[0].split()), REQUIRED)

    def test_guard_precedes_scope_and_regressions_run_in_lightweight_job(self) -> None:
        self.assertLess(CHANGE_SCOPE.index("Verify required repository files"), CHANGE_SCOPE.index("- id: scope"))
        self.assertIn("test_ci_change_scope.py", CHANGE_SCOPE)
        self.assertIn("shell: bash", CHANGE_SCOPE.split("name: Classify changed files", 1)[1])

    def test_required_documents_are_not_ignored_by_workflow_trigger(self) -> None:
        triggers = WORKFLOW.split("permissions:", 1)[0]
        for path in REQUIRED:
            self.assertNotIn(f"- '{path}'", triggers)

    def test_deleting_any_required_file_fails_guard(self) -> None:
        for path in REQUIRED:
            with self.subTest(path=path):
                target = self.repo / path
                original = target.read_bytes()
                target.unlink()
                result = self.guard()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(path, result.stdout)
                target.write_bytes(original)

    def test_required_document_deletion_is_rejected_before_docs_skip(self) -> None:
        (self.repo / REQUIRED[3]).unlink()
        self.commit()
        # The classifier alone still sees Markdown; the unconditional guard closes the gap.
        result, output = self.scope()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output, "docs_only=true\n")
        self.assertNotEqual(self.guard().returncode, 0)

    def test_required_document_rename_is_rejected_before_docs_skip(self) -> None:
        self.git("mv", REQUIRED[3], "docs/renamed.md")
        self.commit()
        result, output = self.scope()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output, "docs_only=true\n")
        self.assertNotEqual(self.guard().returncode, 0)

    def test_directory_cannot_replace_required_file(self) -> None:
        target = self.repo / REQUIRED[3]
        target.unlink()
        target.mkdir()
        self.assertNotEqual(self.guard().returncode, 0)

    def test_regular_docs_change_preserves_skip_on_push_and_pr(self) -> None:
        self.write("docs/guide.md", "updated\n")
        self.commit()
        self.assertEqual(self.guard().returncode, 0)
        for event in ("push", "pull_request"):
            with self.subTest(event=event):
                result, output = self.scope(event=event)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(output, "docs_only=true\n")

    def test_optional_docs_rename_and_delete_preserve_skip(self) -> None:
        self.git("mv", "docs/guide.md", "docs/new.md")
        self.commit()
        self.assertEqual(self.guard().returncode, 0)
        self.assertEqual(self.scope()[1], "docs_only=true\n")
        (self.repo / "docs/new.md").unlink()
        self.commit()
        self.assertEqual(self.guard().returncode, 0)
        self.assertEqual(self.scope()[1], "docs_only=true\n")

    def test_code_mixed_with_docs_runs_heavy_checks(self) -> None:
        self.write("docs/guide.md", "updated\n")
        self.write("source.rs", "updated\n")
        self.commit()
        result, output = self.scope()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output, "docs_only=false\n")

    def test_fetch_failure_fails_without_skip_output(self) -> None:
        self.git("remote", "set-url", "origin", str(self.directory / "missing.git"))
        result, output = self.scope()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(output, "")

    def test_diff_failure_fails_without_skip_output(self) -> None:
        result, output = self.scope(head="f" * 40)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(output, "")

    def test_dispatch_empty_diff_and_missing_base_do_not_skip(self) -> None:
        for event, base in (("workflow_dispatch", self.base), ("push", ""),
                            ("push", "0" * 40), ("pull_request", self.base)):
            with self.subTest(event=event, base=base):
                result, output = self.scope(event=event, base=base)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(output, "docs_only=false\n")


if __name__ == "__main__":
    unittest.main()
