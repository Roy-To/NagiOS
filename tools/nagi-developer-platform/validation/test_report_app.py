"""Run the migration report on the PROVISIONAL `app_windows` fixture (NDP-A018).

The fixture pairs the existing Windows C source (`windows/read_prefix.c`, handwritten
report-v1 snapshot) with one WPF XAML file, so API diagnosis and UI analysis/mapping appear
in one report. These tests check the host report only; not Nagi target acceptance.
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
SOURCE = "tests/developer-platform/evaluation/windows/read_prefix.c"
SNAPSHOT = "tests/developer-platform/api/windows/read_prefix.report-v1.json"
XAML = "tests/developer-platform/evaluation/ui/wpf_task_panel.xaml"
API_ROWS = [(2, "WIN-HEADER"), (4, "WIN-FILE-OPEN"), (7, "WIN-FILE-READ"),
            (8, "WIN-HANDLE-CLOSE")]
AXES = ("accessibility", "appearance_layout", "backend", "focus", "localization",
        "pointer_keyboard")


def section(markdown, title):
    match = re.search(r"\n## {}\n\n(.*?)(?:\n\n|\Z)".format(re.escape(title)), markdown, re.S)
    return match.group(1).splitlines() if match else []


class AppWindowsReportTests(unittest.TestCase):
    def run_cli(self, *arguments):
        return subprocess.run([sys.executable, "-B", str(SCRIPT), *arguments],
                              cwd=report.REPO, capture_output=True, timeout=60)

    def run_main(self, *arguments):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = report.main(list(arguments))
        return code, out.getvalue()

    def test_json_has_api_diagnosis_and_ui_analysis_and_mapping(self):
        run = self.run_cli("--fixture", "app_windows")
        self.assertEqual(run.stderr, b"")
        self.assertEqual(run.returncode, 0, run.stdout.decode())
        self.assertLessEqual(len(run.stdout), report.MAX_REPORT_BYTES)
        self.assertNotIn(os.fsencode(report.REPO), run.stdout)
        result = json.loads(run.stdout)
        checks = result["checks"]
        self.assertEqual(result["fixture"], "app_windows")
        self.assertEqual(result["approval"], "PROVISIONAL")
        self.assertEqual(result["target_acceptance"], "not_run")

        api = checks["windows_diagnosis"]
        self.assertEqual((api["status"], api["reason"]), ("passed", "evidence_diagnosed"))
        self.assertEqual(api["input_paths"], [SOURCE, SNAPSHOT])
        self.assertEqual([(row["line"], row["rule_id"]) for row in api["rows"]], API_ROWS)
        self.assertEqual(api["counts"]["unsupported"], 4)
        self.assertEqual(checks["linux_diagnosis"]["reason"], "no_platform_evidence_for_fixture")

        analysis, mapping = checks["ui_analysis"], checks["ui_mapping"]
        self.assertEqual((analysis["status"], analysis["reason"]), ("passed", "ui_analyzed"))
        self.assertEqual((mapping["status"], mapping["reason"]), ("passed", "ui_mapped"))
        for item in (analysis, mapping):
            self.assertEqual(item["input_paths"], [XAML])
            self.assertEqual([entry["input_paths"] for entry in item["files"]], [[XAML]])
            self.assertEqual(item["counts"]["files"], 1)
        self.assertEqual(analysis["counts"]["widgets"], 7)
        self.assertEqual(analysis["counts"]["custom_widgets"], 1)
        self.assertEqual(mapping["counts"]["rows"], 16)
        entry = mapping["files"][0]
        self.assertEqual(entry["format"], "xaml-ui")
        self.assertEqual(sorted(entry["axis_gaps"]), sorted(AXES))
        self.assertTrue(entry["gap_rows"])
        for counts in entry["axis_gaps"].values():
            self.assertEqual(sum(counts.values()), 16)

        # Stage 3 is never connected; there is no approved Stage 2 target here.
        self.assertEqual(checks["stage_3"]["status"], "not_run")
        self.assertEqual((checks["stage_2"]["status"], checks["stage_2"]["reason"]),
                         ("unsupported", "no_approved_stage2_target_for_fixture"))
        self.assertEqual(checks["generation"]["status"], "passed")
        self.assertEqual(result["overview"]["total"], len(checks))

    def test_markdown_lists_overview_api_gaps_ui_axis_rows_and_scaffold_files(self):
        run = self.run_cli("--fixture", "app_windows", "--format", "markdown")
        self.assertEqual(run.returncode, 0, run.stderr.decode())
        markdown = run.stdout.decode()
        self.assertIn("Fixture `app_windows`", markdown)
        self.assertIn("Nagi target acceptance `not_run`", markdown)
        self.assertIn("## Overview", markdown)
        head = markdown.split("## Checks")[0]
        for name in ("windows_diagnosis", "ui_analysis", "ui_mapping"):
            self.assertIn("- {}\n".format(name), head)
        self.assertIn("- stage_3 · `not_run` · `stage_not_connected`", head)

        gaps = section(markdown, "API gaps")
        self.assertEqual(gaps, [
            "- windows_diagnosis line 2 · `windows.h` · unsupported (rule `WIN-HEADER`)",
            "- windows_diagnosis line 4 · `CreateFileW` · unsupported (rule `WIN-FILE-OPEN`)",
            "- windows_diagnosis line 7 · `ReadFile` · unsupported (rule `WIN-FILE-READ`)",
            "- windows_diagnosis line 8 · `CloseHandle` · unsupported (rule `WIN-HANDLE-CLOSE`)"])

        axis_lines = [line for line in markdown.splitlines() if " axis gaps · " in line]
        self.assertEqual(len(axis_lines), 1)
        self.assertTrue(axis_lines[0].startswith("- ui_mapping `{}` axis gaps · ".format(XAML)))
        for axis in AXES:
            self.assertRegex(axis_lines[0], r"{} unsupported=\d+ needs_decision=\d+".format(axis))
        rows = [line for line in markdown.splitlines()
                if line.startswith("- ui_mapping `{}` row ".format(XAML))]
        self.assertTrue(rows)
        self.assertIn("row `connections:0` (connections) · `unsupported` · "
                      "`declared_event_requires_typed_app_adapter`", markdown)

        files = section(markdown, "Scaffold files")
        self.assertEqual(len(files), 5)
        self.assertIn("- `samples/level-zero/manifest.json`", files)
        # The sections appear in the report's fixed order.
        positions = [markdown.index(title) for title in (
            "## Overview", "## Checks", "## Scaffold files", "## API gaps")]
        self.assertEqual(positions, sorted(positions))
        self.assertLess(markdown.index("## API gaps"), markdown.index(" axis gaps · "))

    def test_all_includes_app_windows_and_counts_match_overviews(self):
        code, output = self.run_main("--fixture", "all")
        self.assertEqual(code, 1)  # `ui_invalid` has a failed check (NDP-A021)
        result = json.loads(output)
        summary = result["summary"]
        # NDP-A019 appended `ui_unsupported` after `app_windows`; NDP-A020 `app_linux`;
        # NDP-A021 `ui_invalid`.
        self.assertEqual(len(report.FIXTURES), 9)
        self.assertEqual(summary["fixtures"], list(report.FIXTURES))
        self.assertEqual(summary["fixtures"][5], "app_windows")
        self.assertEqual(summary["total"], 9 * len(summary["checks"]))
        for status in report.CHECK_STATUSES:
            self.assertEqual(summary["counts"][status], sum(
                item["overview"]["counts"][status] for item in result["reports"].values()))
        self.assertEqual(summary["by_fixture"]["app_windows"],
                         result["reports"]["app_windows"]["overview"]["counts"])
        matrix = summary["matrix"]["app_windows"]
        self.assertEqual(matrix["windows_diagnosis"]["status"], "passed")
        self.assertEqual(matrix["ui_analysis"]["status"], "passed")
        self.assertEqual(matrix["ui_mapping"]["status"], "passed")
        self.assertEqual(matrix["stage_3"], {"status": "not_run", "reason": "stage_not_connected"})
        self.assertEqual(report.undescribed_reasons(list(result["reports"].values())), [])

    def test_existing_fixtures_keep_their_inputs(self):
        # The UI routing is data per fixture; the `ui` pilot keeps all seven files and the
        # other fixtures (app_windows aside) still have no UI inputs.
        self.assertEqual(report.UI_FIXTURE_PATHS["ui"], report.UI_PATHS)
        self.assertEqual(len(report.UI_PATHS), 7)
        self.assertEqual(set(report.UI_FIXTURE_PATHS),
                         {"ui", "app_windows", "ui_unsupported", "app_linux", "ui_invalid"})
        for fixture in ("portable", "windows", "linux", "mixed"):
            with self.subTest(fixture=fixture):
                checks = report.build_report(fixture)["checks"]
                for name in ("ui_analysis", "ui_mapping"):
                    self.assertEqual((checks[name]["status"], checks[name]["reason"]),
                                     ("unsupported", "no_ui_inputs_for_fixture"))
        ui = report.build_report("ui")["checks"]
        self.assertEqual([entry["input_paths"][0] for entry in ui["ui_analysis"]["files"]],
                         [path.as_posix() for path in report.UI_PATHS])
        self.assertEqual(ui["windows_diagnosis"]["reason"], "no_platform_evidence_for_fixture")
        windows = report.build_report("windows")["checks"]["windows_diagnosis"]
        self.assertEqual(windows["input_paths"], [SOURCE, SNAPSHOT])
        # app_windows maps only the windows API case; no linux or mixed evidence.
        self.assertIsNone(report.api_case_paths("app_windows", "linux"))
        self.assertEqual([path.as_posix() for path in report.api_case_paths("app_windows", "windows")],
                         [SOURCE, SNAPSHOT])

    def test_selectable_and_unknown_fixture_is_rejected(self):
        self.assertIn("app_windows", report.FIXTURES)
        with self.assertRaises(ValueError):
            report.build_report("app_windows_extra")
        self.assertEqual(self.run_cli("--fixture", "app_windows_extra").returncode, 2)


if __name__ == "__main__":
    unittest.main()
