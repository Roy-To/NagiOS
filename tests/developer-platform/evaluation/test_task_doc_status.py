"""The NDP-09 task document must carry an honest, evidence-backed status line.

Checks (host only, static text): the `状態:` line is PARTIAL (not NOT STARTED, not
DONE), the evidence section links only files that exist, and it says that Stage 3 /
Nagi target acceptance is not done and that the judgement is PROVISIONAL. The same
checks run against a temporary copy that is edited to confirm they can fail.
"""
from pathlib import Path
import io
import re
import shutil
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
TASK_DOC = (ROOT / "docs" / "development" / "co-developer-starter-kit" / "tasks"
            / "NDP-09_VALIDATION_ORCHESTRATOR.md")
EVIDENCE_HEADING = "## 実績・根拠"
# Evidence the section must cite (relative to the repository root).
REQUIRED_EVIDENCE = (
    "tools/nagi-developer-platform/validation/README.md",
    "tools/nagi-developer-platform/validation/stages.json",
    "tools/nagi-developer-platform/validation/stage2.py",
    "tools/nagi-developer-platform/validation/generated_check.py",
    "tools/nagi-developer-platform/validation/report.py",
    "tools/nagi-developer-platform/validation/test_stage2.py",
    "tools/nagi-developer-platform/validation/test_generated_check.py",
    "tools/nagi-developer-platform/validation/test_report.py",
)


def doc_text(path=TASK_DOC):
    return Path(path).read_text(encoding="utf-8")


def status_line(text):
    lines = [line for line in text.splitlines() if line.startswith("状態:")]
    return lines[0] if len(lines) == 1 else ""


def evidence_section(text):
    start = text.find(EVIDENCE_HEADING)
    if start < 0:
        return ""
    end = text.find("\n## ", start + len(EVIDENCE_HEADING))
    return text[start:] if end < 0 else text[start:end]


def link_destinations(text):
    """Exact destinations of `[label](dest)` links, without any `#fragment`."""
    return re.findall(r"\[[^\]\n]+\]\(([^)\s#]+)(?:#[^)\s]*)?\)", text)


class TaskDocStatus(unittest.TestCase):
    """`doc_path` is swapped for the temporary-copy checks below."""

    doc_path = TASK_DOC

    def text(self):
        return doc_text(self.doc_path)

    def test_has_exactly_one_status_line(self):
        self.assertTrue(status_line(self.text()), "need exactly one `状態:` line")

    def test_status_is_partial_not_not_started(self):
        line = status_line(self.text())
        self.assertIn("PARTIAL", line)
        self.assertNotIn("NOT STARTED", self.text())

    def test_status_is_not_done(self):
        line = status_line(self.text())
        self.assertIsNone(re.search(r"\bDONE\b|\bPASS\b", line), line)

    def test_evidence_links_exist_and_cover_required_files(self):
        section = evidence_section(self.text())
        self.assertTrue(section, "missing {}".format(EVIDENCE_HEADING))
        resolved = set()
        for dest in link_destinations(section):
            path = (Path(self.doc_path).parent / dest).resolve()
            with self.subTest(link=dest):
                self.assertTrue(path.is_file(), dest)
            resolved.add(path)
        # The repository root as seen from the checked document (a temp copy has its own).
        root = Path(self.doc_path).resolve().parents[4]
        for required in REQUIRED_EVIDENCE:
            with self.subTest(required=required):
                self.assertIn((root / required).resolve(), resolved)

    def test_states_stage3_and_target_acceptance_not_done(self):
        section = evidence_section(self.text())
        for phrase in ("Stage 3", "Nagi target", "未実施", "host限定", "実`nagi-pkg`",
                       "PT-01 scanner", "PROVISIONAL", "承認済みではない"):
            with self.subTest(phrase=phrase):
                self.assertIn(phrase, section)
        self.assertIn("Nagi target受入ではない", section)

    def test_purpose_and_scope_sections_are_kept(self):
        text = self.text()
        for heading in ("## 目的", "## 編集範囲案", "## 実装"):
            self.assertIn(heading, text)


def run_checks_on(path):
    case = type("TaskDocCopy", (TaskDocStatus,), {"doc_path": Path(path)})
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(case)
    return unittest.TextTestRunner(stream=io.StringIO(), verbosity=0).run(suite)


class TaskDocMutationTests(unittest.TestCase):
    """Edit a temporary copy of the task doc and confirm the checks fail."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, str(self.tmp))
        self.copy = self.tmp / TASK_DOC.relative_to(ROOT)
        self.copy.parent.mkdir(parents=True)
        # Same depth below the copy root, so relative links resolve to real copies.
        for rel in REQUIRED_EVIDENCE + ("docs/development/developer-platform/handover.md",
                                        "docs/development/developer-platform/board.md",
                                        "tests/developer-platform/evaluation/README.md"):
            dest = self.tmp / rel
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(str(ROOT / rel), str(dest))
        for dest in link_destinations(evidence_section(doc_text())):
            target = (self.copy.parent / dest).resolve()
            if not target.exists():
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(str((TASK_DOC.parent / dest).resolve()), str(target))

    def failed(self, text):
        self.copy.write_text(text, encoding="utf-8")
        result = run_checks_on(self.copy)
        return sorted({getattr(t, "test_case", t)._testMethodName
                       for t, _ in result.failures + result.errors})

    def test_unmodified_copy_passes(self):
        self.assertEqual(self.failed(doc_text()), [])

    def test_status_reverted_to_not_started_fails(self):
        text = doc_text().replace("PARTIAL（host限定、PROVISIONAL。", "NOT STARTED（", 1)
        self.assertNotEqual(text, doc_text())
        self.assertIn("test_status_is_partial_not_not_started", self.failed(text))

    def test_status_marked_done_fails(self):
        text = doc_text().replace("/ PARTIAL（", "/ DONE（", 1)
        self.assertNotEqual(text, doc_text())
        failed = self.failed(text)
        self.assertIn("test_status_is_not_done", failed)
        self.assertIn("test_status_is_partial_not_not_started", failed)

    def test_missing_evidence_file_fails(self):
        (self.tmp / REQUIRED_EVIDENCE[0]).unlink()
        self.assertEqual(self.failed(doc_text()),
                         ["test_evidence_links_exist_and_cover_required_files"])

    def test_link_to_nonexistent_file_fails(self):
        text = doc_text().replace("validation/stages.json)", "validation/stages.missing.json)", 1)
        self.assertNotEqual(text, doc_text())
        self.assertEqual(self.failed(text),
                         ["test_evidence_links_exist_and_cover_required_files"])

    def test_dropping_stage3_unexecuted_note_fails(self):
        text = doc_text().replace("未実施", "実施済み")
        self.assertIn("test_states_stage3_and_target_acceptance_not_done", self.failed(text))


if __name__ == "__main__":
    unittest.main()
