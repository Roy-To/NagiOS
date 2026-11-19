"""Run the migration report on the PROVISIONAL mixed-platform fixture (NDP-A017).

The `mixed` fixture is one self-made C source with Windows API and Linux API lines and a
handwritten report-v1 snapshot per platform. These tests check the host report only; they
are not scanner output and not Nagi target acceptance.
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
SOURCE = "tests/developer-platform/evaluation/mixed/read_and_watch.c"
WINDOWS_SNAPSHOT = "tests/developer-platform/api/mixed/read_and_watch.windows.report-v1.json"
LINUX_SNAPSHOT = "tests/developer-platform/api/mixed/read_and_watch.linux.report-v1.json"
# (line, rule_id) in the handwritten snapshots; None is a line no bundled rule matches.
# The unresolved lines call real platform APIs that the bundled rules do not list:
# Win32 `CancelIoEx` (windows.h) and glibc `euidaccess` (unistd.h, _GNU_SOURCE).
WINDOWS_ROWS = [(3, "WIN-HEADER"), (5, "WIN-FILE-OPEN"), (8, "WIN-FILE-READ"),
                (9, None), (10, "WIN-HANDLE-CLOSE")]
LINUX_ROWS = [(15, "LNX-INOTIFY"), (16, "POSIX-HEADER-FD"), (18, "LNX-INOTIFY"),
              (20, None), (21, "LNX-INOTIFY"), (22, "POSIX-FD-CLOSE")]


def gap_section(markdown):
    match = re.search(r"\n## API gaps\n\n(.*?)(?:\n\n|\Z)", markdown, re.S)
    return match.group(1).splitlines() if match else []


class MixedReportTests(unittest.TestCase):
    def run_cli(self, *arguments):
        return subprocess.run([sys.executable, "-B", str(SCRIPT), *arguments],
                              cwd=report.REPO, capture_output=True, timeout=60)

    def run_main(self, *arguments):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = report.main(list(arguments))
        return code, out.getvalue()

    def assert_diagnosis(self, item, snapshot, expected, unsupported):
        self.assertEqual((item["status"], item["reason"]), ("passed", "evidence_diagnosed"))
        self.assertEqual(item["input_paths"], [SOURCE, snapshot])
        self.assertEqual(item["counts"], {
            "inputs": 2, "sources": 1, "evidence_reports": 1, "rows": len(expected),
            "matched_rules": sum(rule is not None for _, rule in expected),
            "unresolved": sum(rule is None for _, rule in expected),
            "unsupported": unsupported})
        self.assertEqual([(row["line"], row["rule_id"]) for row in item["rows"]], expected)
        self.assertFalse(item["truncated"])
        for row in item["rows"]:
            if row["rule_id"] is None:
                self.assertEqual((row["apis"], row["availability"]), ([], "unknown"))
            else:
                self.assertTrue(row["apis"])

    def test_json_has_windows_and_linux_diagnosis_and_unresolved_lines(self):
        run = self.run_cli("--fixture", "mixed")
        self.assertEqual(run.stderr, b"")
        self.assertEqual(run.returncode, 0, run.stdout.decode())
        self.assertLessEqual(len(run.stdout), report.MAX_REPORT_BYTES)
        self.assertNotIn(os.fsencode(report.REPO), run.stdout)
        result = json.loads(run.stdout)
        checks = result["checks"]
        self.assertEqual(result["fixture"], "mixed")
        self.assertEqual(result["target_acceptance"], "not_run")
        self.assertEqual(checks["stage_3"]["status"], "not_run")
        self.assert_diagnosis(checks["windows_diagnosis"], WINDOWS_SNAPSHOT, WINDOWS_ROWS, 4)
        self.assert_diagnosis(checks["linux_diagnosis"], LINUX_SNAPSHOT, LINUX_ROWS, 5)
        # Each platform diagnosis only reports rules of its own family.
        self.assertTrue(all(row["rule_id"] is None or row["rule_id"].startswith(
            ("WIN-", "DOTNET-", "CRT-")) for row in checks["windows_diagnosis"]["rows"]))
        self.assertTrue(all(row["rule_id"] is None or row["rule_id"].startswith(
            ("LNX-", "POSIX-")) for row in checks["linux_diagnosis"]["rows"]))
        self.assertEqual({name: item["status"] for name, item in checks.items()
                          if name in ("ui_analysis", "ui_mapping", "stage_2")},
                         {"ui_analysis": "unsupported", "ui_mapping": "unsupported",
                          "stage_2": "unsupported"})
        self.assertEqual(checks["stage_2"]["reason"], "no_approved_stage2_target_for_fixture")

    def test_markdown_lists_unsupported_and_unresolved_lines_with_line_numbers(self):
        run = self.run_cli("--fixture", "mixed", "--format", "markdown")
        self.assertEqual(run.returncode, 0, run.stderr.decode())
        markdown = run.stdout.decode()
        self.assertIn("Fixture `mixed`", markdown)
        self.assertIn("Nagi target acceptance `not_run`", markdown)
        lines = gap_section(markdown)
        self.assertEqual(len(lines), len(WINDOWS_ROWS) + len(LINUX_ROWS), lines)
        for line in lines:
            self.assertRegex(line, r"^- (windows|linux)_diagnosis line [1-9][0-9]* · ")
        self.assertEqual(
            [(name, int(number)) for name, number in (
                re.match(r"- (\w+)_diagnosis line (\d+) ", line).groups() for line in lines)],
            [("windows", number) for number, _ in WINDOWS_ROWS]
            + [("linux", number) for number, _ in LINUX_ROWS])
        for expected in (
                "- windows_diagnosis line 5 · `CreateFileW` · unsupported (rule `WIN-FILE-OPEN`)",
                "- windows_diagnosis line 9 · — · no rule matched (unresolved)",
                "- linux_diagnosis line 21 · `IN_MODIFY`, `inotify_add_watch` · "
                "unsupported (rule `LNX-INOTIFY`)",
                "- linux_diagnosis line 20 · — · no rule matched (unresolved)"):
            self.assertIn(expected, lines)

    def test_all_matrix_and_reasons_include_mixed(self):
        code, output = self.run_main("--fixture", "all")
        self.assertEqual(code, 0)
        result = json.loads(output)
        summary = result["summary"]
        self.assertEqual(result["target_acceptance"], "not_run")
        self.assertEqual(summary["fixtures"], list(report.FIXTURES))
        # `mixed` stays fifth; NDP-A018 appended `app_windows` after it.
        self.assertEqual(summary["fixtures"][4], "mixed")
        self.assertEqual(summary["matrix"]["mixed"]["windows_diagnosis"],
                         {"status": "passed", "reason": "evidence_diagnosed"})
        self.assertEqual(summary["matrix"]["mixed"]["linux_diagnosis"],
                         {"status": "passed", "reason": "evidence_diagnosed"})
        self.assertEqual(summary["by_fixture"]["mixed"],
                         result["reports"]["mixed"]["overview"]["counts"])
        self.assertTrue(any(item["fixture"] == "mixed" for item in summary["not_passed"]))
        self.assertEqual(report.undescribed_reasons(list(result["reports"].values())), [])
        # `reasons` aggregates every report's not_passed reason, mixed included.
        mixed_reasons = {item["reason"] for item in
                         result["reports"]["mixed"]["overview"]["not_passed"]}
        expected = {item["reason"] for report_item in result["reports"].values()
                    for item in report_item["overview"]["not_passed"]}
        self.assertTrue(mixed_reasons)
        self.assertEqual({item["reason"] for item in summary["reasons"]}, expected)
        self.assertEqual(len(summary["reasons"]), len(expected))
        self.assertLessEqual(mixed_reasons, {item["reason"] for item in summary["reasons"]})
        for item in summary["reasons"]:
            self.assertTrue(item["reason"] in report.REASON_DESCRIPTIONS
                            or item["reason"] in report.CODE_ONLY_REASONS, item["reason"])
        for item in result["reports"].values():
            self.assertEqual(item["target_acceptance"], "not_run")

    def test_all_markdown_has_a_mixed_part_with_both_platforms(self):
        code, markdown = self.run_main("--fixture", "all", "--format", "markdown")
        self.assertEqual(code, 0)
        part = markdown.split("\n---\n\n")[1:][report.FIXTURES.index("mixed")]
        self.assertIn("Fixture `mixed`", part)
        self.assertEqual(len(gap_section(part)), len(WINDOWS_ROWS) + len(LINUX_ROWS))
        self.assertIn("\n| mixed | ", markdown.split("# Migration evaluation\n")[0])

    def test_existing_fixtures_do_not_pick_up_the_mixed_evidence(self):
        for fixture in ("portable", "windows", "linux", "ui"):
            with self.subTest(fixture=fixture):
                checks = report.build_report(fixture)["checks"]
                for name in ("windows_diagnosis", "linux_diagnosis"):
                    paths = " ".join(checks[name]["input_paths"])
                    self.assertNotIn("mixed", paths)
                    if name.split("_")[0] != fixture:
                        self.assertEqual((checks[name]["status"], checks[name]["reason"]),
                                         ("unsupported", "no_platform_evidence_for_fixture"))
        self.assertEqual(report.diagnose("windows", "linux")["status"], "unsupported")
        self.assertEqual(report.diagnose("mixed", "ui")["status"], "unsupported")

    def test_mixed_is_a_selectable_fixture_and_unknown_names_are_rejected(self):
        self.assertIn("mixed", report.FIXTURES)
        with self.assertRaises(ValueError):
            report.build_report("mixed_platform")
        self.assertEqual(self.run_cli("--fixture", "mixed_platform").returncode, 2)


if __name__ == "__main__":
    unittest.main()
