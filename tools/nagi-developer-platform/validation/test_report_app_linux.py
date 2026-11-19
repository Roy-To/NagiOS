"""Run the migration report on the PROVISIONAL `app_linux` fixture (NDP-A020).

The fixture pairs the existing Linux C source (`linux/watch_changes.c`, handwritten
report-v1 snapshot) with one GTK Builder `.ui` file, so API diagnosis and UI analysis/mapping
appear in one report. These tests check the host report only; not Nagi target acceptance.
"""

import contextlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import unittest

import report


SCRIPT = Path(report.__file__).resolve()
SOURCE = "tests/developer-platform/evaluation/linux/watch_changes.c"
SNAPSHOT = "tests/developer-platform/api/linux/watch_changes.report-v1.json"
GTK_UI = "tests/developer-platform/evaluation/ui/gtk3_task_panel.ui"
API_ROWS = [(2, "LNX-INOTIFY"), (3, "POSIX-HEADER-FD"), (5, "LNX-INOTIFY"),
            (7, "LNX-INOTIFY"), (8, "POSIX-FD-CLOSE")]
AXES = ("accessibility", "appearance_layout", "backend", "focus", "localization",
        "pointer_keyboard")


def section(markdown, title):
    match = re.search(r"\n## {}\n\n(.*?)(?:\n\n|\Z)".format(re.escape(title)), markdown, re.S)
    return match.group(1).splitlines() if match else []


class AppLinuxReportTests(unittest.TestCase):
    def run_cli(self, *arguments):
        return subprocess.run([sys.executable, "-B", str(SCRIPT), *arguments],
                              cwd=report.REPO, capture_output=True, timeout=60)

    def run_main(self, *arguments):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = report.main(list(arguments))
        return code, out.getvalue()

    def test_json_has_api_diagnosis_and_gtk_builder_analysis_and_mapping(self):
        run = self.run_cli("--fixture", "app_linux")
        self.assertEqual(run.stderr, b"")
        self.assertEqual(run.returncode, 0, run.stdout.decode())
        self.assertLessEqual(len(run.stdout), report.MAX_REPORT_BYTES)
        self.assertNotIn(os.fsencode(report.REPO), run.stdout)
        result = json.loads(run.stdout)
        checks = result["checks"]
        self.assertEqual(result["fixture"], "app_linux")
        self.assertEqual(result["approval"], "PROVISIONAL")
        self.assertEqual(result["target_acceptance"], "not_run")

        api = checks["linux_diagnosis"]
        self.assertEqual((api["status"], api["reason"]), ("passed", "evidence_diagnosed"))
        self.assertEqual(api["input_paths"], [SOURCE, SNAPSHOT])
        self.assertEqual([(row["line"], row["rule_id"]) for row in api["rows"]], API_ROWS)
        self.assertEqual(api["counts"]["unsupported"], 5)
        self.assertEqual(checks["windows_diagnosis"]["reason"], "no_platform_evidence_for_fixture")

        analysis, mapping = checks["ui_analysis"], checks["ui_mapping"]
        self.assertEqual((analysis["status"], analysis["reason"]), ("passed", "ui_analyzed"))
        self.assertEqual((mapping["status"], mapping["reason"]), ("passed", "ui_mapped"))
        for item in (analysis, mapping):
            self.assertEqual(item["input_paths"], [GTK_UI])
            self.assertEqual([entry["input_paths"] for entry in item["files"]], [[GTK_UI]])
            self.assertEqual(item["counts"]["files"], 1)
        self.assertEqual(analysis["counts"]["widgets"], 6)
        self.assertEqual(mapping["counts"]["rows"], 11)
        entry = mapping["files"][0]
        self.assertEqual(entry["format"], "gtk-builder-ui")
        self.assertEqual(sorted(entry["axis_gaps"]), sorted(AXES))
        self.assertTrue(entry["gap_rows"])
        for counts in entry["axis_gaps"].values():
            self.assertEqual(sum(counts.values()), 11)

        # Stage 3 is never connected; there is no approved Stage 2 target here.
        self.assertEqual(checks["stage_3"]["status"], "not_run")
        self.assertEqual((checks["stage_2"]["status"], checks["stage_2"]["reason"]),
                         ("unsupported", "no_approved_stage2_target_for_fixture"))
        self.assertEqual(checks["generation"]["status"], "passed")
        self.assertEqual(result["overview"]["total"], len(checks))

    def test_api_rows_match_the_reused_snapshot_and_linux_fixture(self):
        # The diagnosis is the same one the `linux` fixture produces from the same
        # snapshot (reused, not re-implemented), and every row is on a snapshot finding.
        app = report.build_report("app_linux")["checks"]["linux_diagnosis"]
        linux = report.build_report("linux")["checks"]["linux_diagnosis"]
        self.assertEqual(app, linux)
        snapshot = json.loads((report.REPO / SNAPSHOT).read_text(encoding="utf-8"))
        self.assertEqual([row["line"] for row in app["rows"]],
                         [finding["line"] for finding in snapshot["findings"]])
        self.assertEqual({finding["path"] for finding in snapshot["findings"]},
                         {Path(SOURCE).name})
        self.assertIn(SOURCE, snapshot["limitations"][0])
        lines = (report.REPO / SOURCE).read_text(encoding="utf-8").splitlines()
        for finding in snapshot["findings"]:
            self.assertEqual(lines[finding["line"] - 1], finding["evidence"])

    def test_markdown_lists_api_gaps_and_ui_mapping_row_details(self):
        run = self.run_cli("--fixture", "app_linux", "--format", "markdown")
        self.assertEqual(run.returncode, 0, run.stderr.decode())
        markdown = run.stdout.decode()
        self.assertIn("Fixture `app_linux`", markdown)
        self.assertIn("Nagi target acceptance `not_run`", markdown)
        head = markdown.split("## Checks")[0]
        for name in ("linux_diagnosis", "ui_analysis", "ui_mapping"):
            self.assertIn("- {}\n".format(name), head)
        self.assertIn("- stage_3 · `not_run` · `stage_not_connected`", head)

        gaps = section(markdown, "API gaps")
        self.assertEqual(gaps, [
            "- linux_diagnosis line 2 · `sys/inotify.h` · unsupported (rule `LNX-INOTIFY`)",
            "- linux_diagnosis line 3 · `unistd.h` · unsupported (rule `POSIX-HEADER-FD`)",
            "- linux_diagnosis line 5 · `IN_CLOEXEC`, `IN_NONBLOCK`, `inotify_init1` · "
            "unsupported (rule `LNX-INOTIFY`)",
            "- linux_diagnosis line 7 · `IN_MODIFY`, `inotify_add_watch` · "
            "unsupported (rule `LNX-INOTIFY`)",
            "- linux_diagnosis line 8 · `close` · unsupported (rule `POSIX-FD-CLOSE`)"])

        axis_lines = [line for line in markdown.splitlines() if " axis gaps · " in line]
        self.assertEqual(len(axis_lines), 1)
        self.assertTrue(axis_lines[0].startswith("- ui_mapping `{}` axis gaps · ".format(GTK_UI)))
        for axis in AXES:
            self.assertRegex(axis_lines[0], r"{} unsupported=\d+ needs_decision=\d+".format(axis))
        rows = [line for line in markdown.splitlines()
                if line.startswith("- ui_mapping `{}` row ".format(GTK_UI))]
        self.assertTrue(rows)
        self.assertIn("row `connections:0` (connections) · `unsupported` · "
                      "`declared_event_requires_typed_app_adapter`", markdown)

        self.assertEqual(len(section(markdown, "Scaffold files")), 5)
        positions = [markdown.index(title) for title in (
            "## Overview", "## Checks", "## Scaffold files", "## API gaps")]
        self.assertEqual(positions, sorted(positions))
        self.assertLess(markdown.index("## API gaps"), markdown.index(" axis gaps · "))

    def test_every_reason_is_described(self):
        result = report.build_report("app_linux")
        self.assertEqual(report.undescribed_reasons([result]), [])
        for reason in report.collect_reasons(result):
            self.assertTrue(reason in report.REASON_DESCRIPTIONS
                            or reason in report.CODE_ONLY_REASONS, reason)

    def test_all_includes_app_linux_last_and_counts_match_overviews(self):
        code, output = self.run_main("--fixture", "all")
        self.assertEqual(code, 0)
        result = json.loads(output)
        summary = result["summary"]
        self.assertEqual(summary["fixtures"], list(report.FIXTURES))
        # Existing fixtures keep their order; `app_linux` is appended last.
        self.assertEqual(summary["fixtures"][:7],
                         ["portable", "windows", "linux", "ui", "mixed", "app_windows",
                          "ui_unsupported"])
        self.assertEqual(summary["fixtures"][7], "app_linux")
        self.assertEqual(summary["total"], len(summary["fixtures"]) * len(summary["checks"]))
        for status in report.CHECK_STATUSES:
            self.assertEqual(summary["counts"][status], sum(
                item["overview"]["counts"][status] for item in result["reports"].values()))
        self.assertEqual(summary["by_fixture"]["app_linux"],
                         result["reports"]["app_linux"]["overview"]["counts"])
        matrix = summary["matrix"]["app_linux"]
        self.assertEqual(matrix["linux_diagnosis"]["status"], "passed")
        self.assertEqual(matrix["ui_analysis"]["status"], "passed")
        self.assertEqual(matrix["ui_mapping"]["status"], "passed")
        self.assertEqual(matrix["stage_3"], {"status": "not_run", "reason": "stage_not_connected"})
        self.assertEqual(report.undescribed_reasons(list(result["reports"].values())), [])
        for item in result["reports"].values():
            self.assertEqual(item["target_acceptance"], "not_run")

    def test_all_markdown_has_an_app_linux_part_and_matrix_row(self):
        code, markdown = self.run_main("--fixture", "all", "--format", "markdown")
        self.assertEqual(code, 0)
        self.assertIn("\n| app_linux | ", markdown.split("# Migration evaluation\n")[0])
        part = markdown.split("\n---\n\n")[1:][report.FIXTURES.index("app_linux")]
        self.assertIn("Fixture `app_linux`", part)
        self.assertEqual(len(section(part, "API gaps")), len(API_ROWS))
        self.assertLessEqual(len(markdown.encode()), report.MAX_REPORT_BYTES * len(report.FIXTURES))

    def test_existing_fixtures_do_not_pick_up_the_app_linux_inputs(self):
        self.assertEqual([path.as_posix() for path in report.UI_FIXTURE_PATHS["app_linux"]],
                         [GTK_UI])
        self.assertIsNone(report.api_case_paths("app_linux", "windows"))
        self.assertEqual([path.as_posix() for path in report.api_case_paths("app_linux", "linux")],
                         [SOURCE, SNAPSHOT])
        # app_windows still maps the windows case only; the linux fixture has no UI inputs.
        self.assertIsNone(report.api_case_paths("app_windows", "linux"))
        for fixture in ("portable", "windows", "linux", "mixed"):
            with self.subTest(fixture=fixture):
                checks = report.build_report(fixture)["checks"]
                for name in ("ui_analysis", "ui_mapping"):
                    self.assertEqual((checks[name]["status"], checks[name]["reason"]),
                                     ("unsupported", "no_ui_inputs_for_fixture"))
        self.assertEqual(report.diagnose("app_linux", "windows")["status"], "unsupported")

    def test_selectable_and_unknown_fixture_is_rejected(self):
        self.assertIn("app_linux", report.FIXTURES)
        with self.assertRaises(ValueError):
            report.build_report("app_linux_extra")
        self.assertEqual(self.run_cli("--fixture", "app_linux_extra").returncode, 2)


if __name__ == "__main__":
    unittest.main()
