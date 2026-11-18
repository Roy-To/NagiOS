"""Read-only drift check: handwritten API evidence snapshots vs. their source fixtures.

PROVISIONAL host-only check. The snapshots under `tests/developer-platform/api` are
handwritten report-v1 files, not scanner output, so this compares one handwritten
artifact with the fixture it names; it does not prove the snapshot equals what a real
scanner would emit. Snapshots are loaded with the existing `api/common` reader (no
second parser), and the fixture list/paths come from `report.py` (no second list).
Nothing in the repository is modified; drift cases use temporary copies only.
"""

from pathlib import Path
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


def case_paths(platform, fixture=None):
    """(source, snapshot) of a platform diagnosis; `fixture` defaults to the platform.

    The paths come from `report.api_case_paths`, the single table that `report.diagnose` uses.
    """
    return report.api_case_paths(fixture or platform, platform)


def drift_problems(root, platform, fixture=None):
    """Return (problems, checked_api_names) for one platform below `root`.

    `fixture` defaults to the platform's own fixture; `report.MIXED_FIXTURE` selects the
    mixed-platform source with that platform's snapshot. A problem is a short
    human-readable string naming file, line and the mismatch.
    """
    fixture = fixture or platform
    source, evidence = case_paths(platform, fixture)
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
        result = report.diagnose(fixture, platform)
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

    def make_copy(self, platform, fixture=None):
        # Repository reads go through the bounded, no-follow reader so a symlinked
        # fixture or parent directory cannot pull outside files into the copy.
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        for relative in case_paths(platform, fixture):
            data = validate.read_manifest(report.REPO, relative)
            (root / relative).parent.mkdir(parents=True, exist_ok=True)
            (root / relative).write_bytes(data)
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
        # Source and snapshot evidence stay identical, so the evidence check passes;
        # only the API name of one diagnosis row is replaced by a name absent from its
        # line. This isolates the API-name check from the evidence check.
        root = self.make_copy("windows")
        original = report.diagnose

        def renamed(fixture, platform):
            result = original(fixture, platform)
            for row in result["rows"]:
                row["apis"] = ["NotOnThisLine" if api == "ReadFile" else api
                               for api in row["apis"]]
            return result

        with patch.object(report, "diagnose", renamed):
            problems, _ = drift_problems(root, "windows")
        self.assertFalse(any("snapshot evidence" in problem for problem in problems), problems)
        self.assertIn("{}:7: API name 'NotOnThisLine' is not on that line".format(
            case_paths("windows")[0]), problems)

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

    # --- mixed-platform fixture (PROVISIONAL): one source, one snapshot per platform ---

    def test_mixed_fixture_uses_one_source_and_a_snapshot_per_platform(self):
        self.assertIn(report.MIXED_FIXTURE, report.FIXTURES)
        sources, snapshots = set(), set()
        for platform in report.API_CASES:
            source, evidence = case_paths(platform, report.MIXED_FIXTURE)
            sources.add(source)
            snapshots.add(evidence)
            self.assertEqual(evidence.parent, report.API_EVIDENCE / report.MIXED_FIXTURE)
        self.assertEqual(len(sources), 1)
        self.assertEqual(len(snapshots), len(report.API_CASES))
        # Not the handwritten snapshot that has no source (api/linux/mixed_platform).
        self.assertFalse(any("mixed_platform" in path.name for path in snapshots))

    def test_mixed_snapshots_match_the_mixed_source(self):
        for platform in report.API_CASES:
            with self.subTest(platform=platform):
                problems, checked = drift_problems(report.REPO, platform, report.MIXED_FIXTURE)
                self.assertEqual(problems, [])
                self.assertGreater(checked, 0, "no API name was checked")

    def test_mixed_source_holds_apis_of_both_platforms_on_the_cited_lines(self):
        source, _ = case_paths("windows", report.MIXED_FIXTURE)
        lines = validate.read_manifest(report.REPO, source).decode("utf-8").splitlines()
        expected = {"windows": {3: "windows.h", 5: "CreateFileW", 8: "ReadFile",
                                10: "CloseHandle"},
                    "linux": {14: "sys/inotify.h", 15: "unistd.h", 17: "inotify_init1",
                              20: "inotify_add_watch", 21: "close"}}
        for platform, names in expected.items():
            result = report.diagnose(report.MIXED_FIXTURE, platform)
            self.assertEqual(result["status"], "passed", platform)
            rows = {row["line"]: row for row in result["rows"]}
            for number, api in names.items():
                self.assertIn(api, lines[number - 1], (platform, number))
                self.assertIsNotNone(rows[number]["rule_id"], (platform, number))
                self.assertTrue(rows[number]["apis"], (platform, number))
        # The unresolved lines name an API that no bundled rule knows.
        self.assertIn("NtFancyFlush", lines[8])
        self.assertIn("fancy_notify_create", lines[18])

    def test_mixed_shifted_snapshot_line_is_detected(self):
        for platform, old, new in (("windows", '"line": 5,', '"line": 6,'),
                                   ("linux", '"line": 17,', '"line": 18,')):
            with self.subTest(platform=platform):
                root = self.make_copy(platform, report.MIXED_FIXTURE)
                self.rewrite(root, case_paths(platform, report.MIXED_FIXTURE)[1], old, new)
                problems, _ = drift_problems(root, platform, report.MIXED_FIXTURE)
                self.assertTrue(any(":{}:".format(new[len('"line": '):-1]) in problem
                                    and "not on that line" in problem for problem in problems),
                                problems)

    def test_mixed_source_change_that_shifts_lines_is_detected(self):
        for platform in report.API_CASES:
            with self.subTest(platform=platform):
                root = self.make_copy(platform, report.MIXED_FIXTURE)
                path = root / case_paths(platform, report.MIXED_FIXTURE)[0]
                path.write_text("/* inserted */\n" + path.read_text(encoding="utf-8"),
                                encoding="utf-8")
                problems, _ = drift_problems(root, platform, report.MIXED_FIXTURE)
                self.assertTrue(problems)

    def test_mixed_snapshot_line_past_end_of_source_is_detected(self):
        root = self.make_copy("linux", report.MIXED_FIXTURE)
        self.rewrite(root, case_paths("linux", report.MIXED_FIXTURE)[1], '"line": 21,', '"line": 210,')
        problems, _ = drift_problems(root, "linux", report.MIXED_FIXTURE)
        self.assertTrue(any("past the end" in problem for problem in problems), problems)

    def test_mixed_snapshot_that_names_another_source_is_detected(self):
        root = self.make_copy("windows", report.MIXED_FIXTURE)
        self.rewrite(root, case_paths("windows", report.MIXED_FIXTURE)[1],
                     "evaluation/mixed/read_and_watch.c", "evaluation/mixed/other.c")
        problems, _ = drift_problems(root, "windows", report.MIXED_FIXTURE)
        self.assertTrue(any("do not name source" in problem for problem in problems), problems)

    def test_repository_files_are_not_modified_by_the_check(self):
        before = {}
        for platform in report.API_CASES:
            for relative in case_paths(platform):
                before[relative] = validate.read_manifest(report.REPO, relative)
        for platform in report.API_CASES:
            drift_problems(report.REPO, platform)
        for relative, data in before.items():
            self.assertEqual(validate.read_manifest(report.REPO, relative), data)


if __name__ == "__main__":
    unittest.main()
