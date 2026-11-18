"""Read-only drift check: handwritten API evidence snapshots vs. their source fixtures.

PROVISIONAL host-only check. The snapshots under `tests/developer-platform/api` are
handwritten report-v1 files, not scanner output, so this compares one handwritten
artifact with the fixture it names; it does not prove the snapshot equals what a real
scanner would emit. Snapshots are loaded with the existing `api/common` reader (no
second parser), and the fixture list/paths come from `report.py` (no second list).
Nothing in the repository is modified; drift cases use temporary copies only.
"""

from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

import report
import validate

sys.path.insert(0, str(report.TOOLKIT / "api"))
try:
    from common.pt01_report import ReportError, adapt_report_v1, load_report_document
finally:
    sys.path.remove(str(report.TOOLKIT / "api"))

SOURCE_PREFIX = "Source: "


def case_paths(platform):
    basename = report.API_CASES[platform]
    return (report.EVALUATION / platform / (basename + ".c"),
            report.API_EVIDENCE / platform / (basename + ".report-v1.json"))


def drift_problems(root, platform):
    """Return (problems, checked_api_names) for one platform below `root`.

    A problem is a short human-readable string naming file, line and the mismatch.
    """
    source, evidence = case_paths(platform)
    problems = []
    try:
        source_text = validate.read_manifest(root, source).decode("utf-8")
        data = validate.read_manifest(root, evidence)
        findings = adapt_report_v1(data, allowed_paths=[source.name])
        document = load_report_document(data)
    except (validate.InputError, UnicodeError, ReportError) as error:
        return ["{}: snapshot or source unreadable ({})".format(evidence, error)], 0
    lines = source_text.splitlines()

    named = [text.split(SOURCE_PREFIX, 1)[1].strip() for text in document.get("limitations", [])
             if isinstance(text, str) and SOURCE_PREFIX in text]
    if source.as_posix() not in named:
        problems.append("{}: limitations do not name source {}".format(evidence, source))

    for finding in findings:
        if finding.line > len(lines):
            problems.append("{}:{}: line is past the end of the source ({} lines)".format(
                source, finding.line, len(lines)))
        elif finding.evidence.strip() not in lines[finding.line - 1]:
            actual = [number for number, text in enumerate(lines, 1)
                      if finding.evidence.strip() in text]
            problems.append("{}:{}: snapshot evidence {!r} is not on that line (found on {})".format(
                source, finding.line, finding.evidence.strip(),
                actual or "no line"))

    # API names as the existing diagnosis reports them must appear on the cited line.
    with patch.object(report, "REPO", Path(root)):
        result = report.diagnose(platform, platform)
    if result["status"] != "passed":
        problems.append("{}: diagnosis did not pass ({})".format(evidence, result["reason"]))
        return problems, 0
    if result["truncated"]:
        problems.append("{}: diagnosis rows are truncated; API names were not all checked".format(evidence))
    checked = 0
    for row in result["rows"]:
        number = row["line"]
        text = lines[number - 1] if 1 <= number <= len(lines) else ""
        for api in row["apis"]:
            checked += 1
            if api.lower() not in text.lower():
                problems.append("{}:{}: API name {!r} is not on that line".format(source, number, api))
    return problems, checked


class ApiSnapshotDriftTests(unittest.TestCase):
    def test_fixed_fixtures_use_these_api_cases(self):
        # The fixture list is report.py's; this guards against the test silently
        # checking nothing if that table changes shape.
        self.assertTrue(report.API_CASES)
        self.assertTrue(set(report.API_CASES) <= set(report.FIXTURES))

    def test_snapshots_match_source_fixtures(self):
        for platform in report.API_CASES:
            with self.subTest(platform=platform):
                problems, checked = drift_problems(report.REPO, platform)
                self.assertEqual(problems, [])
                self.assertGreater(checked, 0, "no API name was checked")

    def make_copy(self, platform):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        for relative in case_paths(platform):
            (root / relative).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(report.REPO / relative, root / relative)
        return root

    def rewrite(self, root, relative, old, new):
        path = root / relative
        text = path.read_text(encoding="utf-8")
        self.assertIn(old, text)
        path.write_text(text.replace(old, new, 1), encoding="utf-8")

    def test_unmodified_temporary_copy_has_no_drift(self):
        for platform in report.API_CASES:
            with self.subTest(platform=platform):
                self.assertEqual(drift_problems(self.make_copy(platform), platform)[0], [])

    def test_changed_snapshot_line_number_is_detected(self):
        for platform in report.API_CASES:
            with self.subTest(platform=platform):
                root = self.make_copy(platform)
                _, evidence = case_paths(platform)
                self.rewrite(root, evidence, '"line": 2,', '"line": 3,')
                problems, _ = drift_problems(root, platform)
                self.assertTrue(problems)
                self.assertTrue(any(":3:" in problem and "not on that line" in problem
                                    for problem in problems), problems)

    def test_snapshot_line_past_end_of_source_is_detected(self):
        root = self.make_copy("windows")
        _, evidence = case_paths("windows")
        self.rewrite(root, evidence, '"line": 8,', '"line": 80,')
        problems, _ = drift_problems(root, "windows")
        self.assertTrue(any("past the end" in problem for problem in problems), problems)

    def test_changed_snapshot_api_name_is_detected(self):
        root = self.make_copy("windows")
        _, evidence = case_paths("windows")
        self.rewrite(root, evidence, "ReadFile(file", "WriteFile(file")
        problems, _ = drift_problems(root, "windows")
        self.assertTrue(any("not on that line" in problem for problem in problems), problems)

    def test_source_change_that_shifts_lines_is_detected(self):
        for platform in report.API_CASES:
            with self.subTest(platform=platform):
                root = self.make_copy(platform)
                source, _ = case_paths(platform)
                path = root / source
                path.write_text("/* inserted */\n" + path.read_text(encoding="utf-8"),
                                encoding="utf-8")
                problems, _ = drift_problems(root, platform)
                self.assertTrue(problems)

    def test_snapshot_that_names_another_source_is_detected(self):
        root = self.make_copy("linux")
        _, evidence = case_paths("linux")
        self.rewrite(root, evidence, "evaluation/linux/watch_changes.c", "evaluation/linux/other.c")
        problems, _ = drift_problems(root, "linux")
        self.assertTrue(any("do not name source" in problem for problem in problems), problems)

    def test_repository_files_are_not_modified_by_the_check(self):
        before = {}
        for platform in report.API_CASES:
            for relative in case_paths(platform):
                before[relative] = (report.REPO / relative).read_bytes()
        for platform in report.API_CASES:
            drift_problems(report.REPO, platform)
        for relative, data in before.items():
            self.assertEqual((report.REPO / relative).read_bytes(), data)


if __name__ == "__main__":
    unittest.main()
