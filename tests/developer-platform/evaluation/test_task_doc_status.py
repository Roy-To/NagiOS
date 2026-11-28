"""The NDP-09 task document must carry an honest, evidence-backed status line.

Checks (host only, static text): the `状態:` line is PARTIAL (not NOT STARTED, not
DONE), the evidence section links only files that exist inside the repository (after
resolving `..` and symlinks), and it says - in one bullet, not scattered words - that
Stage 3 / Nagi target acceptance is not done and that the judgement is PROVISIONAL.
The same checks run against a temporary copy that is edited to confirm they can fail.
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
    "tests/developer-platform/evaluation/test_task_doc_status.py",
)
# `状態: <担当> / <状態値>（補足）`: the value is the text between the first `/` and
# the opening parenthesis of the note.
STATUS_VALUE = re.compile(r"^状態:[^/\n]*/\s*([^（(\n]*?)\s*[（(]")


def doc_text(path=TASK_DOC):
    return Path(path).read_text(encoding="utf-8")


def status_line(text):
    lines = [line for line in text.splitlines() if line.startswith("状態:")]
    return lines[0] if len(lines) == 1 else ""


def status_value(line):
    """The status value itself, without the owner part or the parenthesised note."""
    match = STATUS_VALUE.match(line)
    return match.group(1) if match else None


def evidence_section(text):
    start = text.find(EVIDENCE_HEADING)
    if start < 0:
        return ""
    end = text.find("\n## ", start + len(EVIDENCE_HEADING))
    return text[start:] if end < 0 else text[start:end]


def is_inside(path, root):
    """True when the resolved `path` is `root` or below it (both resolved)."""
    try:
        Path(path).resolve().relative_to(Path(root).resolve())
    except ValueError:
        return False
    return True


def lines_with_all(text, phrases):
    return [line for line in text.splitlines() if all(p in line for p in phrases)]


def copy_inside(src_root, rel_or_path, dst_root, src_base=None):
    """Copy one file from `src_root` to the same relative place below `dst_root`.

    `rel_or_path` is resolved against `src_base` (default `src_root`). The resolved
    source must stay inside `src_root` and the destination inside `dst_root`; this is
    checked before any directory is created or any file is read.
    """
    src_root, dst_root = Path(src_root).resolve(), Path(dst_root).resolve()
    src = (Path(src_base or src_root) / rel_or_path).resolve()
    if not is_inside(src, src_root):
        raise ValueError("source outside repository: {}".format(rel_or_path))
    dst = (dst_root / src.relative_to(src_root)).resolve()
    if not is_inside(dst, dst_root):
        raise ValueError("destination outside copy root: {}".format(rel_or_path))
    if not dst.exists():
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(str(src), str(dst))
    return dst


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
        # The value before the note must be exactly PARTIAL; a note mentioning
        # PARTIAL must not keep e.g. `DONE扱い（旧状態はPARTIAL）` green.
        self.assertEqual(status_value(line), "PARTIAL", line)
        self.assertNotIn("NOT STARTED", self.text())

    def test_status_is_not_done(self):
        line = status_line(self.text())
        value = status_value(line)
        self.assertIsNotNone(value, line)
        # No word boundary check: `DONE扱い` has none between `E` and `扱`.
        self.assertNotRegex(value, r"DONE|PASS")
        self.assertIsNone(re.search(r"\bDONE\b|\bPASS\b", line), line)

    def test_evidence_links_exist_and_cover_required_files(self):
        section = evidence_section(self.text())
        self.assertTrue(section, "missing {}".format(EVIDENCE_HEADING))
        # The repository root as seen from the checked document (a temp copy has its own).
        root = Path(self.doc_path).resolve().parents[4]
        resolved = set()
        for dest in link_destinations(section):
            path = (Path(self.doc_path).parent / dest).resolve()
            with self.subTest(link=dest):
                # Containment first: never accept an outside file as evidence.
                self.assertTrue(is_inside(path, root), "outside repository: " + dest)
                self.assertTrue(path.is_file(), dest)
            resolved.add(path)
        for required in REQUIRED_EVIDENCE:
            with self.subTest(required=required):
                self.assertIn((root / required).resolve(), resolved)

    def test_states_stage3_and_target_acceptance_not_done(self):
        section = evidence_section(self.text())
        for phrase in ("host限定", "PROVISIONAL", "承認済みではない"):
            with self.subTest(phrase=phrase):
                self.assertIn(phrase, section)
        self.assertIn("Nagi target受入ではない", section)
        # Each "not done" statement must be closed within one line (bullet), so that a
        # heading word elsewhere cannot keep the check green.
        for subject in (("Stage 3", "Nagi target", "受入"),
                        ("実`nagi-pkg`", "PT-01 scanner")):
            with self.subTest(subject=subject):
                self.assertTrue(lines_with_all(section, subject + ("未実施",)),
                                "no single line says {} is 未実施".format(subject))
                self.assertEqual(lines_with_all(section, subject[:1] + ("実施済み",)), [])

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
        tmp = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, str(tmp))
        # The copied repository sits one level down, so a file next to it is "outside".
        self.tmp = tmp / "repo"
        self.outside = tmp / "outside.md"
        self.outside.write_text("not evidence\n", encoding="utf-8")
        self.copy = self.tmp / TASK_DOC.relative_to(ROOT)
        self.copy.parent.mkdir(parents=True)
        # Same depth below the copy root, so relative links resolve to real copies.
        for rel in REQUIRED_EVIDENCE + ("docs/development/developer-platform/handover.md",
                                        "docs/development/developer-platform/board.md",
                                        "tests/developer-platform/evaluation/README.md"):
            copy_inside(ROOT, rel, self.tmp)
        for dest in link_destinations(evidence_section(doc_text())):
            copy_inside(ROOT, dest, self.tmp, src_base=TASK_DOC.parent)

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

    def test_status_done_with_partial_note_fails(self):
        text = doc_text().replace("/ PARTIAL（", "/ DONE扱い（旧状態はPARTIAL。", 1)
        self.assertNotEqual(text, doc_text())
        self.assertIn("PARTIAL", status_line(text))
        failed = self.failed(text)
        self.assertIn("test_status_is_not_done", failed)
        self.assertIn("test_status_is_partial_not_not_started", failed)

    def test_status_value_with_suffix_fails(self):
        text = doc_text().replace("/ PARTIAL（", "/ PARTIAL完了（", 1)
        self.assertNotEqual(text, doc_text())
        self.assertEqual(self.failed(text), ["test_status_is_partial_not_not_started"])

    def test_renamed_self_test_reference_fails(self):
        old = "evaluation/test_task_doc_status.py)"
        self.assertEqual(doc_text().count(old), 1)
        text = doc_text().replace(old, "evaluation/test_task_doc_status_missing.py)", 1)
        self.assertEqual(self.failed(text),
                         ["test_evidence_links_exist_and_cover_required_files"])

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

    def test_only_stage3_bullet_marked_done_fails(self):
        # The heading "未実施（…）:" stays; only the Stage 3 bullet claims completion.
        old = "- Stage 3（Nagi target）の受入は未実施。"
        self.assertEqual(doc_text().count(old), 1)
        text = doc_text().replace(old, "- Stage 3（Nagi target）の受入は実施済み。", 1)
        self.assertIn("未実施（", evidence_section(text))
        self.assertEqual(self.failed(text),
                         ["test_states_stage3_and_target_acceptance_not_done"])

    def test_only_pkg_scanner_bullet_marked_done_fails(self):
        old = "PT-01 scanner接続は未実施。"
        self.assertEqual(doc_text().count(old), 1)
        text = doc_text().replace(old, "PT-01 scanner接続は実施済み。", 1)
        self.assertEqual(self.failed(text),
                         ["test_states_stage3_and_target_acceptance_not_done"])

    def link_extra(self, dest):
        anchor = "- Stage契約（NDP-09s2）: "
        self.assertEqual(doc_text().count(anchor), 1)
        return doc_text().replace(anchor, anchor + "[extra]({})と".format(dest), 1)

    def test_link_escaping_repository_fails(self):
        dest = "../../../../../outside.md"
        self.assertEqual((self.copy.parent / dest).resolve(), self.outside)
        self.assertTrue(self.outside.is_file())
        self.assertEqual(self.failed(self.link_extra(dest)),
                         ["test_evidence_links_exist_and_cover_required_files"])

    def test_symlink_to_outside_file_fails(self):
        link = self.tmp / "tools" / "nagi-developer-platform" / "validation" / "extra.md"
        try:
            link.symlink_to(self.outside)
        except (OSError, NotImplementedError) as error:
            self.skipTest("symlink unavailable: {}".format(error))
        dest = "../../../../tools/nagi-developer-platform/validation/extra.md"
        self.assertTrue((self.copy.parent / dest).is_file())
        self.assertEqual(self.failed(self.link_extra(dest)),
                         ["test_evidence_links_exist_and_cover_required_files"])

    def test_copy_refuses_source_outside_repository(self):
        dst = self.tmp.parent / "dst"
        with self.assertRaises(ValueError):
            copy_inside(self.tmp, "../outside.md", dst)
        self.assertFalse(dst.exists())

    def test_copy_refuses_symlinked_source_outside_repository(self):
        link = self.tmp / "escape.md"
        try:
            link.symlink_to(self.outside)
        except (OSError, NotImplementedError) as error:
            self.skipTest("symlink unavailable: {}".format(error))
        dst = self.tmp.parent / "dst"
        with self.assertRaises(ValueError):
            copy_inside(self.tmp, "escape.md", dst)
        self.assertFalse(dst.exists())


if __name__ == "__main__":
    unittest.main()
