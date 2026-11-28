"""Run the migration report on the PROVISIONAL `ui_unsupported` fixture (NDP-A019).

The fixture is one self-made UI layout file whose XML root is none of Qt Designer, GTK
Builder or XAML. The existing analyzer answers `unsupported_format`; these tests pin how
that shows up end to end (check statuses, reasons, Markdown descriptions, exit code).
Host report only; not Nagi target acceptance.
"""

import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import report


SCRIPT = Path(report.__file__).resolve()
FIXTURE = "ui_unsupported"
UI_FILE = "tests/developer-platform/evaluation/ui_unsupported/panel_layout.xml"


class UiUnsupportedReportTests(unittest.TestCase):
    def run_cli(self, *arguments):
        return subprocess.run([sys.executable, "-B", str(SCRIPT), *arguments],
                              cwd=report.REPO, capture_output=True, timeout=60)

    def run_main(self, *arguments):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = report.main(list(arguments))
        return code, out.getvalue()

    def test_json_analysis_is_unsupported_and_mapping_is_not_run(self):
        run = self.run_cli("--fixture", FIXTURE)
        self.assertEqual(run.stderr, b"")
        self.assertEqual(run.returncode, 0, run.stdout.decode())
        self.assertNotIn(os.fsencode(report.REPO), run.stdout)
        result = json.loads(run.stdout)
        checks = result["checks"]
        self.assertEqual(result["fixture"], FIXTURE)
        self.assertEqual(result["approval"], "PROVISIONAL")
        self.assertEqual(result["target_acceptance"], "not_run")

        analysis, mapping = checks["ui_analysis"], checks["ui_mapping"]
        self.assertEqual((analysis["status"], analysis["reason"]),
                         ("unsupported", "file_unsupported"))
        self.assertEqual((mapping["status"], mapping["reason"]),
                         ("not_run", "file_not_run"))
        for item in (analysis, mapping):
            self.assertEqual(item["input_paths"], [UI_FILE])
            self.assertEqual(item["counts"]["files"], 1)
        self.assertEqual(analysis["counts"]["files_unsupported"], 1)
        self.assertEqual(analysis["counts"]["files_passed"], 0)
        self.assertEqual(mapping["counts"]["files_not_run"], 1)
        file_analysis, = analysis["files"]
        self.assertEqual((file_analysis["status"], file_analysis["reason"]),
                         ("unsupported", "unsupported_format"))
        self.assertEqual(file_analysis["input_paths"], [UI_FILE])
        file_mapping, = mapping["files"]
        self.assertEqual((file_mapping["status"], file_mapping["reason"]),
                         ("not_run", "ui_analysis_did_not_pass"))
        # Nothing was extracted from the unsupported file.
        self.assertNotIn("format", file_analysis)
        self.assertNotIn("gap_rows", file_mapping)
        self.assertNotIn("axis_gaps", file_mapping)

    def test_independent_checks_continue_and_overview_counts_match(self):
        result = json.loads(self.run_cli("--fixture", FIXTURE).stdout)
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
        self.assertIn({"name": "ui_analysis", "status": "unsupported",
                       "reason": "file_unsupported"}, overview["not_passed"])
        self.assertIn({"name": "ui_mapping", "status": "not_run",
                       "reason": "file_not_run"}, overview["not_passed"])

    def test_markdown_overview_and_problem_lines_describe_reasons(self):
        run = self.run_cli("--fixture", FIXTURE, "--format", "markdown")
        self.assertEqual(run.returncode, 0, run.stderr.decode())
        markdown = run.stdout.decode()
        self.assertIn("Fixture `ui_unsupported`", markdown)
        head = markdown.split("## Checks")[0]
        descriptions = report.REASON_DESCRIPTIONS
        self.assertIn("- ui_analysis · `unsupported` · `file_unsupported` — "
                      + descriptions["file_unsupported"], head)
        self.assertIn("- ui_mapping · `not_run` · `file_not_run` — "
                      + descriptions["file_not_run"], head)
        lines = markdown.splitlines()
        self.assertIn("- ui_analysis `{}` · `unsupported` · `unsupported_format` — {}".format(
            UI_FILE, descriptions["unsupported_format"]), lines)
        self.assertIn("- ui_mapping `{}` · `not_run` · `ui_analysis_did_not_pass` — {}".format(
            UI_FILE, descriptions["ui_analysis_did_not_pass"]), lines)
        # No mapping row or axis detail exists for the unsupported file.
        self.assertNotIn(" axis gaps · ", markdown)
        self.assertFalse([line for line in lines if line.startswith("- ui_mapping `{}` row ".format(UI_FILE))])

    def test_reasons_are_described_and_all_includes_the_fixture(self):
        result = report.build_report(FIXTURE)
        self.assertEqual(report.undescribed_reasons([result]), [])
        found = report.collect_reasons(result)
        for reason in ("file_unsupported", "unsupported_format", "file_not_run",
                       "ui_analysis_did_not_pass"):
            self.assertIn(reason, found)
            self.assertIn(reason, report.REASON_DESCRIPTIONS)

        code, output = self.run_main("--fixture", "all")
        self.assertEqual(code, 1)  # `ui_invalid` (NDP-A021) has a failed check
        everything = json.loads(output)
        summary = everything["summary"]
        # NDP-A019 appended `ui_unsupported` seventh; NDP-A020 appended `app_linux` after it.
        self.assertEqual(summary["fixtures"][6], FIXTURE)
        self.assertEqual(summary["matrix"][FIXTURE]["ui_analysis"],
                         {"status": "unsupported", "reason": "file_unsupported"})
        self.assertEqual(summary["matrix"][FIXTURE]["ui_mapping"],
                         {"status": "not_run", "reason": "file_not_run"})
        self.assertEqual(summary["by_fixture"][FIXTURE],
                         everything["reports"][FIXTURE]["overview"]["counts"])
        self.assertEqual(report.undescribed_reasons(list(everything["reports"].values())), [])

    def test_unsupported_format_is_the_analyzer_answer_not_a_report_rule(self):
        # The routing is data only: the same file through the analyzer CLI gives the
        # stable `unsupported_format` code, and the report keeps that code as is.
        with self.assertRaises(report.CheckError) as raised:
            report.run_json_script("ui/analysis/analyze.py", ("--root", str(report.REPO), UI_FILE))
        self.assertEqual((str(raised.exception), raised.exception.status),
                         ("unsupported_format", "unsupported"))
        self.assertEqual(report.UI_FIXTURE_PATHS[FIXTURE], (Path(UI_FILE),))
        self.assertIn(FIXTURE, report.FIXTURES)

    def test_non_xml_text_would_fail_not_unsupported(self):
        # Documents the limit recorded in the README: the analyzer decides by the XML
        # root, so a non-XML (QML-like) text is `invalid_xml` (failed), not unsupported.
        with tempfile.TemporaryDirectory(prefix="nagi-ndp-a019-") as directory:
            (Path(directory) / "panel.qml").write_bytes(b'Item {\n  Text { text: "Tasks" }\n}\n')
            with self.assertRaises(report.CheckError) as raised:
                report.run_json_script("ui/analysis/analyze.py",
                                       ("--root", directory, "panel.qml"))
        self.assertEqual((str(raised.exception), raised.exception.status),
                         ("invalid_xml", "failed"))


if __name__ == "__main__":
    unittest.main()
