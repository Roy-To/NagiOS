"""Run the migration report on the PROVISIONAL `ui_invalid` fixture (NDP-A021).

The fixture is one self-made QML-like UI file that is not XML. The existing analyzer
answers `invalid_xml`; these tests pin how that shows up end to end: `ui_analysis` is
`failed` (not `unsupported`), `ui_mapping` is `not_run`, independent checks continue, the
CLI exits 1, and the Overview and Markdown problem lines describe the reason. Host report
only; not Nagi target acceptance.
"""

import contextlib
import io
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import unittest
from unittest.mock import patch

import report
import stage2
import validate


SCRIPT = Path(report.__file__).resolve()
# JSON of the eight existing fixtures produced by the pre-NDP-A021 report.py (see README.md),
# with `recorded_at` set to "T" and the validator and clang forced missing.
BASE_SNAPSHOT = SCRIPT.parent / "report_base_existing_fixtures.json"
EVALUATION_README = report.REPO / report.EVALUATION / "README.md"
FIXTURE_COUNT_MARKER = re.compile(r"Migration report fixture count: (\d+)")
FIXTURE = "ui_invalid"
UI_FILE = "tests/developer-platform/evaluation/ui_invalid/panel_layout.qml"
# The fixtures that existed before NDP-A021, in their `--fixture all` order.
EXISTING_FIXTURES = ("portable", "windows", "linux", "ui", "mixed", "app_windows",
                     "ui_unsupported", "app_linux")
INVALID_XML_TEXT = "The file is not valid XML."


def strip_time(value):
    return json.loads(re.sub(r'"recorded_at": "[^"]*"', '"recorded_at": "T"', json.dumps(value)))


class UiInvalidReportTests(unittest.TestCase):
    def run_cli(self, *arguments):
        return subprocess.run([sys.executable, "-B", str(SCRIPT), *arguments],
                              cwd=report.REPO, capture_output=True, timeout=60)

    def run_main(self, *arguments):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = report.main(list(arguments))
        return code, out.getvalue()

    def test_json_analysis_is_failed_and_mapping_is_not_run_with_exit_code_1(self):
        run = self.run_cli("--fixture", FIXTURE)
        self.assertEqual(run.stderr, b"")
        self.assertEqual(run.returncode, 1, run.stdout.decode())
        self.assertNotIn(os.fsencode(report.REPO), run.stdout)
        result = json.loads(run.stdout)
        checks = result["checks"]
        self.assertEqual(result["fixture"], FIXTURE)
        self.assertEqual(result["approval"], "PROVISIONAL")
        self.assertEqual(result["target_acceptance"], "not_run")

        analysis, mapping = checks["ui_analysis"], checks["ui_mapping"]
        self.assertEqual((analysis["status"], analysis["reason"]), ("failed", "file_failed"))
        self.assertEqual((mapping["status"], mapping["reason"]), ("not_run", "file_not_run"))
        for item in (analysis, mapping):
            self.assertEqual(item["input_paths"], [UI_FILE])
            self.assertEqual(item["counts"]["files"], 1)
        self.assertEqual(analysis["counts"]["files_failed"], 1)
        self.assertEqual(analysis["counts"]["files_unsupported"], 0)
        self.assertEqual(analysis["counts"]["files_passed"], 0)
        self.assertEqual(mapping["counts"]["files_not_run"], 1)
        file_analysis, = analysis["files"]
        self.assertEqual((file_analysis["status"], file_analysis["reason"]),
                         ("failed", "invalid_xml"))
        self.assertEqual(file_analysis["input_paths"], [UI_FILE])
        file_mapping, = mapping["files"]
        self.assertEqual((file_mapping["status"], file_mapping["reason"]),
                         ("not_run", "ui_analysis_did_not_pass"))
        # Nothing was extracted from the invalid file.
        self.assertNotIn("format", file_analysis)
        self.assertNotIn("gap_rows", file_mapping)
        self.assertNotIn("axis_gaps", file_mapping)

    def test_independent_checks_continue_and_overview_counts_match(self):
        run = self.run_cli("--fixture", FIXTURE)
        self.assertEqual(run.returncode, 1)
        result = json.loads(run.stdout)
        checks = result["checks"]
        self.assertEqual((checks["generation"]["status"], checks["generation"]["reason"]),
                         ("passed", "scaffold_created"))
        for name in ("windows_diagnosis", "linux_diagnosis"):
            self.assertEqual((checks[name]["status"], checks[name]["reason"]),
                             ("unsupported", "no_platform_evidence_for_fixture"))
        self.assertEqual((checks["stage_2"]["status"], checks["stage_2"]["reason"]),
                         ("unsupported", "no_approved_stage2_target_for_fixture"))
        self.assertEqual(checks["stage_3"]["status"], "not_run")
        overview = result["overview"]
        self.assertEqual(overview["total"], len(checks))
        self.assertEqual(sum(overview["counts"].values()), len(checks))
        # `ui_analysis` is the only failed check of this fixture.
        self.assertEqual(overview["counts"]["failed"], 1)
        self.assertIn({"name": "ui_analysis", "status": "failed",
                       "reason": "file_failed"}, overview["not_passed"])
        self.assertIn({"name": "ui_mapping", "status": "not_run",
                       "reason": "file_not_run"}, overview["not_passed"])

    def test_markdown_overview_and_problem_lines_describe_reasons(self):
        run = self.run_cli("--fixture", FIXTURE, "--format", "markdown")
        self.assertEqual(run.returncode, 1, run.stderr.decode())
        markdown = run.stdout.decode()
        self.assertIn("Fixture `ui_invalid`", markdown)
        head = markdown.split("## Checks")[0]
        descriptions = report.REASON_DESCRIPTIONS
        self.assertEqual(descriptions["invalid_xml"], INVALID_XML_TEXT)
        self.assertIn("- ui_analysis · `failed` · `file_failed` — "
                      + descriptions["file_failed"], head)
        self.assertIn("- ui_mapping · `not_run` · `file_not_run` — "
                      + descriptions["file_not_run"], head)
        lines = markdown.splitlines()
        self.assertIn("- ui_analysis `{}` · `failed` · `invalid_xml` — {}".format(
            UI_FILE, INVALID_XML_TEXT), lines)
        self.assertIn("- ui_mapping `{}` · `not_run` · `ui_analysis_did_not_pass` — {}".format(
            UI_FILE, descriptions["ui_analysis_did_not_pass"]), lines)
        # No mapping row or axis detail exists for the invalid file.
        self.assertNotIn(" axis gaps · ", markdown)
        self.assertFalse([line for line in lines
                          if line.startswith("- ui_mapping `{}` row ".format(UI_FILE))])

    def test_reasons_are_described(self):
        result = report.build_report(FIXTURE)
        self.assertEqual(report.undescribed_reasons([result]), [])
        found = report.collect_reasons(result)
        for reason in ("file_failed", "invalid_xml", "file_not_run", "ui_analysis_did_not_pass"):
            self.assertIn(reason, found)
            self.assertIn(reason, report.REASON_DESCRIPTIONS)

    def test_failed_is_not_unsupported_unlike_the_ui_unsupported_fixture(self):
        invalid = report.build_report(FIXTURE)["checks"]["ui_analysis"]
        unsupported = report.build_report("ui_unsupported")["checks"]["ui_analysis"]
        self.assertEqual((invalid["status"], unsupported["status"]), ("failed", "unsupported"))
        self.assertEqual(self.run_main("--fixture", "ui_unsupported")[0], 0)
        self.assertEqual(self.run_main("--fixture", FIXTURE)[0], 1)

    def test_invalid_xml_is_the_analyzer_answer_not_a_report_rule(self):
        # The routing is data only: the same file through the analyzer CLI gives the
        # stable `invalid_xml` code (a failure, not `unsupported_format`).
        with self.assertRaises(report.CheckError) as raised:
            report.run_json_script("ui/analysis/analyze.py", ("--root", str(report.REPO), UI_FILE))
        self.assertEqual((str(raised.exception), raised.exception.status),
                         ("invalid_xml", "failed"))
        self.assertEqual(report.UI_FIXTURE_PATHS[FIXTURE], (Path(UI_FILE),))
        self.assertIn(FIXTURE, report.FIXTURES)

    def test_all_appends_the_fixture_and_keeps_existing_reports_unchanged(self):
        code, output = self.run_main("--fixture", "all")
        # Any failed check in any fixture makes the exit code 1; ui_invalid is the one.
        self.assertEqual(code, 1)
        everything = json.loads(output)
        summary = everything["summary"]
        self.assertEqual(summary["fixtures"], [*EXISTING_FIXTURES, FIXTURE])
        self.assertEqual(summary["fixtures"], list(report.FIXTURES))
        self.assertEqual(list(everything["reports"]), summary["fixtures"])
        self.assertEqual(report.FIXTURES[:len(EXISTING_FIXTURES)], EXISTING_FIXTURES)
        self.assertEqual(summary["total"], 9 * len(summary["checks"]))
        self.assertEqual(summary["matrix"][FIXTURE]["ui_analysis"],
                         {"status": "failed", "reason": "file_failed"})
        self.assertEqual(summary["matrix"][FIXTURE]["ui_mapping"],
                         {"status": "not_run", "reason": "file_not_run"})
        self.assertEqual(summary["by_fixture"][FIXTURE],
                         everything["reports"][FIXTURE]["overview"]["counts"])
        self.assertEqual(summary["counts"]["failed"], 1)
        self.assertEqual([item for item in summary["not_passed"] if item["status"] == "failed"],
                         [{"fixture": FIXTURE, "name": "ui_analysis", "status": "failed",
                           "reason": "file_failed"}])
        self.assertIn({"reason": "file_failed",
                       "description": report.REASON_DESCRIPTIONS["file_failed"]},
                      summary["reasons"])
        self.assertEqual(report.undescribed_reasons(list(everything["reports"].values())), [])
        # The reports of the existing fixtures equal their single-fixture reports (time aside).
        for fixture in EXISTING_FIXTURES:
            with self.subTest(fixture=fixture):
                self.assertEqual(strip_time(everything["reports"][fixture]),
                                 strip_time(report.build_report(fixture)))

    def test_all_markdown_has_a_part_and_matrix_row_for_the_fixture(self):
        code, markdown = self.run_main("--fixture", "all", "--format", "markdown")
        self.assertEqual(code, 1)
        self.assertIn("\n| ui_invalid | ", markdown.split("# Migration evaluation\n")[0])
        part = markdown.split("\n---\n\n")[1:][report.FIXTURES.index(FIXTURE)]
        self.assertIn("Fixture `ui_invalid`", part)
        self.assertIn("`invalid_xml` — " + INVALID_XML_TEXT, part)
        self.assertLessEqual(len(markdown.encode()), report.MAX_REPORT_BYTES * len(report.FIXTURES))

    def test_all_output_limit_follows_the_fixture_count(self):
        self.assertEqual(len(report.FIXTURES), 9)
        code, output = self.run_main("--fixture", "all")
        self.assertEqual(code, 1)
        size = len(output.encode("utf-8"))
        self.assertLessEqual(size, report.MAX_REPORT_BYTES * len(report.FIXTURES))
        # A per-report cap just large enough for nine reports lets the output through; a cap
        # just below `size / 9` rejects it, so the limit is the cap times the fixture count.
        with patch.object(report, "MAX_REPORT_BYTES", math.ceil(size / 9) + 1024):
            code, again = self.run_main("--fixture", "all")
        self.assertEqual(code, 1)
        self.assertTrue(again.startswith("{"))
        err = io.StringIO()
        with patch.object(report, "MAX_REPORT_BYTES", size // 9 - 1024), \
                contextlib.redirect_stderr(err):
            code, refused = self.run_main("--fixture", "all")
        self.assertEqual((code, refused), (1, ""))
        self.assertEqual(err.getvalue().strip(), "report_output_limit")

    def test_all_output_limit_changes_with_the_fixture_count_for_the_same_output(self):
        # One fixed `all` report and one fixed per-report cap; only the length of FIXTURES
        # changes. A limit that ignored the fixture count (for example a hard-coded nine)
        # would not switch between accept and refuse here.
        fixed = report.build_all_report()
        size = len((json.dumps(fixed, indent=2) + "\n").encode("utf-8"))
        for count in (len(report.FIXTURES), len(report.FIXTURES) + 1):
            cap = math.ceil(size / count)
            self.assertGreater(size, cap * (count - 1))
            fewer = tuple("f{}".format(index) for index in range(count - 1))
            exact = tuple("f{}".format(index) for index in range(count))
            with self.subTest(count=count), \
                    patch.object(report, "build_all_report", return_value=fixed), \
                    patch.object(report, "MAX_REPORT_BYTES", cap):
                with patch.object(report, "FIXTURES", exact):
                    code, output = self.run_main("--fixture", "all")
                self.assertEqual(code, 1)
                self.assertEqual(len(output.encode("utf-8")), size)
                err = io.StringIO()
                with patch.object(report, "FIXTURES", fewer), contextlib.redirect_stderr(err):
                    code, refused = self.run_main("--fixture", "all")
                self.assertEqual((code, refused), (1, ""))
                self.assertEqual(err.getvalue().strip(), "report_output_limit")


@contextlib.contextmanager
def without_host_tools():
    with patch.object(validate, "VALIDATOR", Path("/nonexistent/nagi-pkg")), \
            patch.object(stage2.shutil, "which", return_value=None):
        yield


class ExistingFixturesMatchBaseTests(unittest.TestCase):
    """The eight existing reports must equal what the pre-NDP-A021 report.py produced."""

    @classmethod
    def setUpClass(cls):
        cls.snapshot = json.loads(BASE_SNAPSHOT.read_text(encoding="utf-8"))

    def test_snapshot_lists_exactly_the_existing_fixtures(self):
        self.assertEqual(self.snapshot["fixtures"], list(EXISTING_FIXTURES))
        self.assertEqual(list(self.snapshot["reports"]), sorted(EXISTING_FIXTURES))
        self.assertNotIn(FIXTURE, self.snapshot["reports"])

    def test_single_fixture_reports_equal_the_base_snapshot(self):
        with without_host_tools():
            for fixture in EXISTING_FIXTURES:
                with self.subTest(fixture=fixture):
                    self.assertEqual(strip_time(report.build_report(fixture)),
                                     self.snapshot["reports"][fixture])

    def test_all_reports_equal_the_base_snapshot_and_keep_their_order(self):
        out = io.StringIO()
        with without_host_tools(), contextlib.redirect_stdout(out):
            code = report.main(["--fixture", "all"])
        self.assertEqual(code, 1)
        reports = json.loads(out.getvalue())["reports"]
        self.assertEqual(list(reports)[:len(EXISTING_FIXTURES)], list(EXISTING_FIXTURES))
        for fixture in EXISTING_FIXTURES:
            with self.subTest(fixture=fixture):
                self.assertEqual(strip_time(reports[fixture]), self.snapshot["reports"][fixture])


class EvaluationReadmeFixtureCountTests(unittest.TestCase):
    def test_readme_states_the_report_fixture_count_once_and_it_matches(self):
        text = EVALUATION_README.read_text(encoding="utf-8")
        stated = FIXTURE_COUNT_MARKER.findall(text)
        self.assertEqual(len(stated), 1, "evaluation README must state the count exactly once")
        self.assertEqual(int(stated[0]), len(report.FIXTURES))

    def test_readme_lists_every_report_fixture_in_order(self):
        text = EVALUATION_README.read_text(encoding="utf-8")
        start = FIXTURE_COUNT_MARKER.search(text).end()
        listed = re.findall(r"`([a-z_]+)`", text[start:text.index(")", start)])
        self.assertEqual(listed, list(report.FIXTURES))


if __name__ == "__main__":
    unittest.main()
