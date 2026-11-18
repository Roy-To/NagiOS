"""Run the migration report on the checked-in evaluation fixtures."""

import contextlib
import hashlib
import io
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import generated_check
import report
import stage2
import validate


SCRIPT = Path(report.__file__).resolve()
HAS_VALIDATOR = (validate.VALIDATOR.is_file() and not validate.VALIDATOR.is_symlink()
                 and os.access(validate.VALIDATOR, os.X_OK))
HAS_CLANG = shutil.which("clang") is not None


def tree_hash():
    digest = hashlib.sha256()
    for relative in (report.EVALUATION, report.API_EVIDENCE):
        for path in sorted((report.REPO / relative).rglob("*")):
            name = path.relative_to(report.REPO).as_posix().encode()
            digest.update(len(name).to_bytes(4, "big") + name)
            if path.is_file():
                digest.update(b"f" + hashlib.sha256(path.read_bytes()).digest())
            elif path.is_dir():
                digest.update(b"d")
    return digest.hexdigest()


class ReportTests(unittest.TestCase):
    def run_cli(self, *arguments, root=report.REPO):
        script = root / SCRIPT.relative_to(report.REPO)
        return subprocess.run([sys.executable, "-B", str(script), *arguments],
                              cwd=root, capture_output=True, timeout=60)

    def assert_counts(self, item, **counts):
        self.assertEqual(item["counts"], {"inputs": len(item["input_paths"]), **counts})

    def assert_target_not_run(self, result):
        self.assertEqual(result["target_acceptance"], "not_run")
        self.assertEqual(result["checks"]["stage_3"],
                         {"status": "not_run", "reason": "stage_not_connected",
                          "counts": {"inputs": 0}, "input_paths": []})

    def assert_portable(self, checks):
        host = checks["stage_2"]
        self.assertEqual(host["input_paths"], [
            "tests/developer-platform/evaluation/portable/summary.c",
            "tests/developer-platform/evaluation/portable/summary_test.c",
            "tests/developer-platform/evaluation/portable/summary.h",
        ])
        if HAS_CLANG:
            self.assertEqual(host["status"], "passed")
            self.assertEqual(host["reason"], "build_and_test_passed")
            self.assert_counts(host, sources=2, headers=1, steps_run=2, steps_passed=2,
                               errors=0, warnings=0)
            self.assertEqual([(step["name"], step["status"], step["exit_code"])
                              for step in host["steps"]],
                             [("compile", "passed", 0), ("run_test", "passed", 0)])
        else:
            self.assertEqual(host["status"], "not_run")
            self.assertEqual(host["reason"], "clang_unavailable")
            self.assert_counts(host, sources=2, headers=1, steps_run=0, steps_passed=0)
            self.assertEqual(host["steps"], [])

    def assert_platform(self, checks, platform):
        item = checks[platform + "_diagnosis"]
        if platform == "windows":
            basename = "read_prefix"
            expected = [(2, "WIN-HEADER"), (4, "WIN-FILE-OPEN"),
                        (7, "WIN-FILE-READ"), (8, "WIN-HANDLE-CLOSE")]
        else:
            basename = "watch_changes"
            expected = [(2, "LNX-INOTIFY"), (3, "POSIX-HEADER-FD"),
                        (5, "LNX-INOTIFY"), (7, "LNX-INOTIFY"), (8, "POSIX-FD-CLOSE")]
        self.assertEqual(item["input_paths"], [
            "tests/developer-platform/evaluation/{}/{}.c".format(platform, basename),
            "tests/developer-platform/api/{}/{}.report-v1.json".format(platform, basename),
        ])
        self.assert_counts(item, sources=1, evidence_reports=1, rows=len(expected),
                           matched_rules=len(expected), unresolved=0, unsupported=len(expected))
        self.assertEqual([(row["line"], row["rule_id"]) for row in item["rows"]], expected)
        self.assertFalse(item["truncated"])

    def assert_ui(self, checks):
        analyzed, mapped = checks["ui_analysis"], checks["ui_mapping"]
        files = {"files": 7, "files_passed": 7, "files_failed": 0, "files_not_run": 0,
                 "files_unsupported": 0}
        self.assert_counts(analyzed, widgets=30, layouts=6, connections=6, tab_order=4,
                           bindings=7, resources=6, unknown=4, custom_widgets=2,
                           unresolved_references=1, **files)
        self.assert_counts(mapped, rows=63, supported=0, partial=33, unsupported=20,
                           needs_decision=10, **files)
        self.assertEqual(
            [(Path(entry["input_paths"][0]).name, entry["format"], entry["status"],
              entry["counts"]["widgets"], entry["counts"]["layouts"])
             for entry in analyzed["files"]],
            [("task_panel.ui", "qt-designer-ui", "passed", 3, 1),
             ("gtk_minimal.ui", "gtk-builder-ui", "passed", 1, 0),
             ("gtk3_task_panel.ui", "gtk-builder-ui", "passed", 6, 1),
             ("gtk4_task_panel.ui", "gtk-builder-ui", "passed", 5, 1),
             ("xaml_minimal.xaml", "xaml-ui", "passed", 1, 0),
             ("wpf_task_panel.xaml", "xaml-ui", "passed", 7, 2),
             ("winui_task_panel.xaml", "xaml-ui", "passed", 7, 1)],
        )
        self.assertEqual(
            [(entry["input_paths"], entry["format"], entry["status"]) for entry in mapped["files"]],
            [(entry["input_paths"], entry["format"], "passed") for entry in analyzed["files"]],
        )
        self.assertEqual([entry["counts"]["rows"] for entry in mapped["files"]],
                         [6, 1, 11, 9, 1, 16, 19])

    def test_cli_reports_all_four_fixtures_twice_with_unchanged_inputs(self):
        before = tree_hash()
        try:
            for fixture in ("portable", "windows", "linux", "ui"):
                with self.subTest(fixture=fixture):
                    first, second = (self.run_cli("--fixture", fixture) for _ in range(2))
                    for execution in (first, second):
                        self.assertEqual(execution.returncode, 0,
                                         (execution.stdout.decode(), execution.stderr.decode()))
                        self.assertEqual(execution.stderr, b"")
                        self.assertLessEqual(len(execution.stdout), report.MAX_REPORT_BYTES)
                        self.assertNotIn(os.fsencode(report.REPO), execution.stdout)
                    actual, repeated = json.loads(first.stdout), json.loads(second.stdout)
                    self.assertTrue(actual.pop("recorded_at"))
                    self.assertTrue(repeated.pop("recorded_at"))
                    self.assertEqual(actual, repeated)
                    self.assertEqual(actual["fixture"], fixture)
                    self.assertEqual(actual["scope"], "host_fixture_evaluation")
                    self.assert_target_not_run(actual)
                    checks = actual["checks"]
                    self.assertEqual({name: item["status"] for name, item in checks.items()}, {
                        "windows_diagnosis": "passed" if fixture == "windows" else "unsupported",
                        "linux_diagnosis": "passed" if fixture == "linux" else "unsupported",
                        "ui_analysis": "passed" if fixture == "ui" else "unsupported",
                        "ui_mapping": "passed" if fixture == "ui" else "unsupported",
                        "generation": "passed",
                        "stage_1": "passed" if HAS_VALIDATOR else "not_run",
                        "stage_2": ("passed" if HAS_CLANG else "not_run")
                        if fixture == "portable" else "unsupported",
                        "stage_2_package": "not_run",
                        "stage_3": "not_run",
                    })
                    self.assert_counts(checks["generation"], files=5)
                    self.assert_counts(checks["stage_1"], manifests=int(HAS_VALIDATOR))
                    self.assertEqual(checks["stage_1"]["reason"],
                                     "metadata_valid" if HAS_VALIDATOR else "validator_unavailable")
                    self.assertEqual(checks["stage_1"]["input_paths"],
                                     ["samples/level-zero/manifest.json"])
                    self.assert_counts(checks["stage_2_package"])
                    for item in checks.values():
                        if item["status"] == "unsupported":
                            self.assertEqual(item["input_paths"], [])
                            self.assert_counts(item)
                        for relative in item["input_paths"]:
                            self.assertFalse(Path(relative).is_absolute())
                            self.assertNotIn("..", Path(relative).parts)
                    if fixture == "portable":
                        self.assert_portable(checks)
                    elif fixture == "ui":
                        self.assert_ui(checks)
                    else:
                        self.assert_platform(checks, fixture)
        finally:
            self.assertEqual(tree_hash(), before)

    def test_markdown_summarizes_host_evidence_and_unrun_target(self):
        result = self.run_cli("--fixture", "windows", "--format", "markdown")
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        markdown = result.stdout.decode()
        self.assertIn("**PROVISIONAL**", markdown)
        self.assertIn("| windows_diagnosis | `passed` | `evidence_diagnosed` |", markdown)
        self.assertIn("matched_rules=4", markdown)
        self.assertIn("tests/developer-platform/evaluation/windows/read_prefix.c", markdown)
        self.assertIn("| stage_3 | `not_run` | `stage_not_connected` |", markdown)
        self.assertIn("Nagi target acceptance `not_run`", markdown)
        self.assertNotRegex(markdown, r"(?i)(stage_3|target acceptance)[^\n]*(pass|acceptance achieved)")

    def test_overview_matches_checks_for_all_fixtures(self):
        for fixture in ("portable", "windows", "linux", "ui"):
            with self.subTest(fixture=fixture):
                result = report.build_report(fixture)
                checks, overview = result["checks"], result["overview"]
                self.assertEqual(overview["total"], len(checks))
                self.assertEqual(sum(overview["counts"].values()), len(checks))
                for status in ("passed", "failed", "not_run", "unsupported"):
                    self.assertEqual(overview["counts"][status],
                                     sum(item["status"] == status for item in checks.values()))
                self.assertEqual(
                    overview["not_passed"],
                    [{"name": name, "status": item["status"], "reason": item["reason"]}
                     for name, item in checks.items() if item["status"] != "passed"])
                self.assertEqual(result["version"], 1)

    def test_markdown_overview_describes_known_reasons_and_not_unknown_ones(self):
        result = report.build_report("windows")
        result["checks"]["stage_3"]["reason"] = "made_up_reason_code"
        result["overview"] = report.build_overview(result["checks"])
        markdown = report.markdown_report(result)
        head = markdown.split("## Checks")[0]
        self.assertIn("### Passed", head)
        self.assertIn("### Unsupported or not run (reason)", head)
        self.assertIn("- windows_diagnosis", head)
        self.assertIn("`package_and_signature_checks_not_connected` — "
                      + report.REASON_DESCRIPTIONS["package_and_signature_checks_not_connected"], head)
        self.assertIn("- stage_3 · `not_run` · `made_up_reason_code`\n", head)
        self.assertLess(head.index("## Overview"), markdown.index("| Check |"))

    def run_main(self, *arguments):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = report.main(list(arguments))
        return code, out.getvalue()

    def test_all_json_matrix_covers_four_fixtures_and_matches_overviews(self):
        run = self.run_cli("--fixture", "all")
        self.assertEqual(run.stderr, b"")
        self.assertEqual(run.returncode, 0, run.stdout.decode())
        self.assertNotIn(os.fsencode(report.REPO), run.stdout)
        result = json.loads(run.stdout)
        summary = result["summary"]
        self.assertEqual(result["fixture"], "all")
        self.assertEqual(result["target_acceptance"], "not_run")
        self.assertEqual(summary["fixtures"], ["portable", "windows", "linux", "ui"])
        self.assertEqual(list(result["reports"]), summary["fixtures"])
        self.assertEqual(summary["total"], len(summary["fixtures"]) * len(summary["checks"]))
        for fixture, individual in result["reports"].items():
            self.assertEqual(individual["fixture"], fixture)
            self.assertEqual(summary["by_fixture"][fixture], individual["overview"]["counts"])
            self.assertEqual(summary["matrix"][fixture], {
                name: {"status": item["status"], "reason": item["reason"]}
                for name, item in individual["checks"].items()})
        for status in report.CHECK_STATUSES:
            self.assertEqual(summary["counts"][status], sum(
                item["overview"]["counts"][status] for item in result["reports"].values()))
            self.assertEqual(summary["counts"][status], sum(
                cell["status"] == status for row in summary["matrix"].values()
                for cell in row.values()))
        self.assertEqual(sum(summary["counts"].values()), summary["total"])
        self.assertEqual(summary["matrix"]["windows"]["windows_diagnosis"]["status"], "passed")
        self.assertEqual(summary["matrix"]["ui"]["stage_3"],
                         {"status": "not_run", "reason": "stage_not_connected"})
        self.assertEqual(summary["matrix"]["portable"]["windows_diagnosis"],
                         {"status": "unsupported", "reason": "no_platform_evidence_for_fixture"})

    def test_all_markdown_starts_with_matrix_and_keeps_individual_output(self):
        code, markdown = self.run_main("--fixture", "all", "--format", "markdown")
        self.assertEqual(code, 0)
        head = markdown.split("# Migration evaluation\n")[0]
        self.assertTrue(markdown.startswith("# Migration evaluation: all fixtures"))
        self.assertIn("## Fixture x check matrix", head)
        for fixture in report.FIXTURES:
            self.assertIn("\n| {} | ".format(fixture), head)
        self.assertIn("| Fixture | windows_diagnosis | linux_diagnosis |", head)
        self.assertIn("`unsupported`<br>`no_platform_evidence_for_fixture`", head)
        self.assertIn("`no_ui_inputs_for_fixture` — "
                      + report.REASON_DESCRIPTIONS["no_ui_inputs_for_fixture"], head)
        self.assertEqual(markdown.count("# Migration evaluation\n"), len(report.FIXTURES))
        for fixture in report.FIXTURES:
            self.assertIn("**PROVISIONAL** · Fixture `{}` · Toolkit (non-AI)".format(fixture), markdown)
        # Each individual section equals the single-fixture output (time aside).
        sections = markdown.split("\n---\n\n")[1:]
        self.assertEqual(len(sections), len(report.FIXTURES))
        strip = lambda text: re.sub(r"Recorded at [^ ]+\.", "Recorded at T.", text)
        for fixture, section in zip(report.FIXTURES, sections):
            self.assertEqual(strip(section), strip(report.markdown_report(report.build_report(fixture))))

    def test_all_counts_and_exit_code_follow_failed_checks(self):
        real = report.build_report

        def failing(fixture):
            result = real(fixture)
            if fixture == "linux":
                result["checks"]["stage_3"] = {**result["checks"]["stage_3"],
                                               "status": "failed", "reason": "tool_exit_nonzero"}
                result["overview"] = report.build_overview(result["checks"])
            return result

        with patch.object(report, "build_report", failing):
            code, output = self.run_main("--fixture", "all")
        self.assertEqual(code, 1)
        summary = json.loads(output)["summary"]
        self.assertEqual(summary["counts"]["failed"], 1)
        self.assertEqual(summary["matrix"]["linux"]["stage_3"]["status"], "failed")
        self.assertIn({"fixture": "linux", "name": "stage_3", "status": "failed",
                       "reason": "tool_exit_nonzero"}, summary["not_passed"])
        with patch.object(report, "build_report", real):
            code, _ = self.run_main("--fixture", "all")
        self.assertEqual(code, 0)

    def test_all_not_run_is_never_counted_as_passed(self):
        with patch.object(report, "host_build",
                          lambda fixture: report.unavailable("clang_unavailable")), \
                patch.object(generated_check, "generate_scaffold",
                             side_effect=lambda root: {"status": "unavailable", "reason": "x"}):
            code, output = self.run_main("--fixture", "all")
        summary = json.loads(output)["summary"]
        self.assertEqual(code, 0)
        self.assertEqual(summary["matrix"]["portable"]["stage_2"],
                         {"status": "not_run", "reason": "clang_unavailable"})
        self.assertEqual(summary["counts"]["passed"], sum(
            cell["status"] == "passed" for row in summary["matrix"].values() for cell in row.values()))

    def test_all_reasons_use_descriptions_and_unknown_codes_stay_code_only(self):
        reports = {fixture: report.build_report(fixture) for fixture in ("windows", "ui")}
        reports["windows"]["checks"]["stage_3"]["reason"] = "made_up_reason_code"
        reports["windows"]["overview"] = report.build_overview(reports["windows"]["checks"])
        summary = report.build_summary(reports)
        by_reason = {item["reason"]: item["description"] for item in summary["reasons"]}
        self.assertIsNone(by_reason["made_up_reason_code"])
        self.assertEqual(by_reason["no_ui_inputs_for_fixture"],
                         report.REASON_DESCRIPTIONS["no_ui_inputs_for_fixture"])
        self.assertEqual(by_reason["stage_not_connected"],
                         report.REASON_DESCRIPTIONS["stage_not_connected"])
        text = "\n".join(report.summary_lines(summary))
        self.assertIn("- windows / stage_3 · `not_run` · `made_up_reason_code`\n", text + "\n")
        self.assertIn("`stage_not_connected` — " + report.REASON_DESCRIPTIONS["stage_not_connected"], text)

    def test_every_reason_in_real_fixture_reports_is_described_or_allowlisted(self):
        # Real reports for all four fixtures, in the normal environment and with the
        # validator and clang forced missing, so not_run reasons are covered even on
        # a host that has both tools (and the normal run on a host that has neither).
        reports = []
        for fixture in report.FIXTURES:
            reports.append(report.build_report(fixture))
            with patch.object(validate, "VALIDATOR", Path("/nonexistent/nagi-pkg")):
                reports.append(report.build_report(fixture))
            with patch.object(stage2.shutil, "which", return_value=None):
                reports.append(report.build_report(fixture))
        missing = report.undescribed_reasons(reports)
        self.assertEqual(missing, [], "add a REASON_DESCRIPTIONS entry (or an intentional "
                         "CODE_ONLY_REASONS entry) for: {}".format(", ".join(missing)))
        found = set().union(*(report.collect_reasons(item) for item in reports))
        for reason in ("clang_unavailable", "validator_unavailable",
                       "package_and_signature_checks_not_connected"):
            self.assertIn(reason, found)

    def test_unlisted_reason_is_detected_in_checks_files_and_steps(self):
        base = report.build_report("windows")
        self.assertEqual(report.undescribed_reasons([base]), [])
        planted = report.build_report("windows")
        planted["checks"]["stage_3"]["reason"] = "new_unlisted_check_reason"
        planted["checks"]["ui_analysis"]["files"] = [{"status": "failed", "reason": "new_unlisted_file_reason"}]
        planted["checks"]["stage_2"]["steps"] = [{"status": "failed", "reason": "new_unlisted_step_reason"}]
        self.assertEqual(report.undescribed_reasons([base, planted]),
                         ["new_unlisted_check_reason", "new_unlisted_file_reason",
                          "new_unlisted_step_reason"])
        with patch.object(report, "CODE_ONLY_REASONS", frozenset({"new_unlisted_file_reason"})):
            self.assertEqual(report.undescribed_reasons([planted]),
                             ["new_unlisted_check_reason", "new_unlisted_step_reason"])

    def test_code_only_allowlist_does_not_overlap_descriptions(self):
        self.assertEqual(set(report.CODE_ONLY_REASONS) & set(report.REASON_DESCRIPTIONS), set())
        for reason, text in report.REASON_DESCRIPTIONS.items():
            self.assertTrue(text and text.endswith("."), reason)

    def test_markdown_problem_lines_describe_known_reasons_and_not_unknown_ones(self):
        result = report.build_report("ui")
        analysis = result["checks"]["ui_analysis"]
        analysis["files"][0] = {**analysis["files"][0], "status": "failed", "reason": "invalid_xml"}
        analysis["files"][1] = {**analysis["files"][1], "status": "not_run", "reason": "made_up_file_reason"}
        result["checks"]["stage_2"]["steps"] = [
            {"name": "compile", "status": "failed", "reason": "timeout"},
            {"name": "run_test", "status": "not_run", "reason": "made_up_step_reason"},
            {"name": "extra", "status": "passed", "reason": "build_and_test_passed"}]
        markdown = report.markdown_report(result)
        first = analysis["files"][0]["input_paths"][0]
        second = analysis["files"][1]["input_paths"][0]
        self.assertIn("- ui_analysis `{}` · `failed` · `invalid_xml` — {}\n".format(
            first, report.REASON_DESCRIPTIONS["invalid_xml"]), markdown)
        self.assertIn("- ui_analysis `{}` · `not_run` · `made_up_file_reason`\n".format(second), markdown)
        self.assertIn("- stage_2 `compile` · `failed` · `timeout` — {}\n".format(
            report.REASON_DESCRIPTIONS["timeout"]), markdown)
        self.assertIn("- stage_2 `run_test` · `not_run` · `made_up_step_reason`\n", markdown)
        self.assertNotIn("`extra`", markdown)

    def test_real_invalid_xml_skips_only_matching_mapping(self):
        with tempfile.TemporaryDirectory(prefix="ndp-report-input-") as directory:
            root = Path(directory)
            shutil.copytree(report.REPO / report.EVALUATION / "ui", root / report.EVALUATION / "ui")
            (root / report.UI_PATHS[0]).write_bytes(b"<ui>")
            with patch.object(report, "REPO", root):
                result = report.build_report("ui")
        checks = result["checks"]
        analyzed, mapped = checks["ui_analysis"], checks["ui_mapping"]
        self.assertEqual((analyzed["status"], analyzed["reason"]), ("failed", "file_failed"))
        self.assertEqual(analyzed["counts"]["files_failed"], 1)
        self.assertEqual(analyzed["counts"]["files_passed"], 6)
        self.assertEqual(analyzed["files"][0]["reason"], "invalid_xml")
        self.assertEqual((mapped["status"], mapped["reason"]), ("not_run", "file_not_run"))
        self.assertEqual(mapped["counts"]["files_passed"], 6)
        self.assertEqual(mapped["files"][0]["reason"], "ui_analysis_did_not_pass")
        self.assertEqual(mapped["files"][-1]["counts"]["rows"], 19)
        self.assertEqual(checks["generation"]["status"], "passed")
        self.assertIn("invalid_xml", report.markdown_report(result))
        self.assert_target_not_run(result)

    def test_generation_failure_skips_manifest_and_continues_host_build(self):
        inputs = dict(generated_check.DEFAULT_INPUTS, app_id="invalid app ID")
        with patch.object(generated_check, "DEFAULT_INPUTS", inputs), \
                patch.object(validate, "validate", side_effect=AssertionError("Stage 1 must not run")):
            result = report.build_report("portable")
        checks = result["checks"]
        self.assertEqual(checks["generation"]["status"], "failed")
        self.assertEqual(checks["generation"]["reason"], "invalid_template_input:app_id")
        self.assert_counts(checks["generation"], files=0)
        self.assertEqual(checks["stage_1"]["status"], "not_run")
        self.assertEqual(checks["stage_1"]["reason"], "generation_did_not_pass")
        self.assert_counts(checks["stage_1"], manifests=0)
        self.assert_portable(checks)
        self.assert_target_not_run(result)

    @staticmethod
    def raw_mapping_rows(path):
        """Rows of map_ui.py for one fixture file, straight from the tools."""
        _, encoded = report.run_json_script("ui/analysis/analyze.py",
                                            ("--root", str(report.REPO), path))
        mapping, _ = report.run_json_script("ui/mapping/map_ui.py", data=encoded)
        return mapping["rows"]

    def test_ui_gap_rows_match_status_counts_with_only_allowed_fields(self):
        mapped = report.build_report("ui")["checks"]["ui_mapping"]
        excluded = 0
        for item in mapped["files"]:
            counts, gaps = item["counts"], item["gap_rows"]
            raw = self.raw_mapping_rows(item["input_paths"][0])
            expected = [row["id"] for row in raw if row["mapping"]["status"] != "supported"]
            excluded += len(raw) - len(expected)
            self.assertEqual(item["gap_rows_total"], len(expected))
            self.assertFalse(item["gap_rows_truncated"])
            self.assertEqual(sorted(row["id"] for row in gaps), sorted(expected))
            # Row status counts bound the listed rows: unsupported and
            # needs_decision are listed in full, partial only when not a
            # supported vocabulary match.
            for status in ("unsupported", "needs_decision"):
                self.assertEqual(sum(row["status"] == status for row in gaps), counts[status])
            self.assertLessEqual(sum(row["status"] == "partial" for row in gaps), counts["partial"])
            self.assertLessEqual(len(gaps), counts["rows"])
            for row in gaps:
                self.assertEqual(set(row), {"id", "source_kind", "status", "mapping_status", "reason"})
                self.assertNotEqual(row["mapping_status"], "supported")
            ranks = [report.GAP_ORDER[row["mapping_status"]] for row in gaps]
            self.assertEqual(ranks, sorted(ranks))
        # The fixture has vocabulary-supported rows and they are left out.
        self.assertGreater(excluded, 0)
        # Existing keys stay as before.
        self.assertEqual(set(mapped["files"][0]) - {"gap_rows", "gap_rows_total", "gap_rows_truncated"},
                         {"status", "reason", "input_paths", "counts", "format", "contract_evidence"})

    def test_ui_gap_rows_exclude_supported_and_keep_severe_rows_when_truncated(self):
        def row(index, mapping_status, row_status):
            return {"id": "widgets:{}".format(index), "source_kind": "widgets", "status": row_status,
                    "mapping": {"status": mapping_status, "reason": "reason_{}".format(index)}}
        rows = [row(0, "supported", "partial"), row(1, "partial", "partial"),
                row(2, "supported", "partial"), row(3, "partial", "partial"),
                row(4, "needs_decision", "needs_decision"), row(5, "unsupported", "unsupported")]
        gaps = report.gap_rows(rows)
        self.assertEqual([gap["id"] for gap in gaps],
                         ["widgets:5", "widgets:4", "widgets:1", "widgets:3"])
        self.assertEqual([gap["mapping_status"] for gap in gaps],
                         ["unsupported", "needs_decision", "partial", "partial"])
        bad = rows + [row(6, "supported", "/abs/path")]
        with self.assertRaises(report.CheckError):
            report.gap_rows(bad)

    def test_ui_gap_rows_are_truncated_and_markdown_shows_omission(self):
        with patch.object(report, "MAX_UI_DETAILS", 3):
            result = report.build_report("ui")
        mapped = result["checks"]["ui_mapping"]
        largest = max(mapped["files"], key=lambda item: item["gap_rows_total"])
        total = largest["gap_rows_total"]
        self.assertGreater(total, 3)
        self.assertEqual(len(largest["gap_rows"]), 3)
        self.assertTrue(largest["gap_rows_truncated"])
        for item in mapped["files"]:
            self.assertEqual(item["gap_rows_truncated"], item["gap_rows_total"] > 3)
        # Truncation keeps the most severe rows.
        everything = report.gap_rows(self.raw_mapping_rows(largest["input_paths"][0]))
        self.assertEqual(largest["gap_rows"], everything[:3])
        with patch.object(report, "MAX_UI_MARKDOWN_ROWS", 2):
            markdown = report.markdown_report(result)
        path = largest["input_paths"][0]
        self.assertIn("- ui_mapping `{}` … {} more non-supported rows omitted (see JSON)".format(
            path, total - 2), markdown)
        self.assertEqual(markdown.count("- ui_mapping `{}` row ".format(path)), 2)

    def test_ui_gap_rows_markdown_lists_reasons_and_leaks_no_absolute_path(self):
        result = report.build_report("ui")
        markdown = report.markdown_report(result)
        first = result["checks"]["ui_mapping"]["files"][0]
        row = first["gap_rows"][0]
        self.assertIn("- ui_mapping `{}` row `{}` ({}) · `{}` · {}\n".format(
            first["input_paths"][0], row["id"], row["source_kind"], row["mapping_status"],
            report.describe_reason(row["reason"])), markdown)
        self.assertNotIn(str(report.REPO), markdown)
        self.assertNotIn(str(report.REPO), json.dumps(result))
        # A code without a description is shown as the code alone.
        row["reason"] = "made_up_row_reason"
        self.assertIn("· `made_up_row_reason`\n", report.markdown_report(result))

    def test_ui_gap_rows_absent_when_tool_unavailable_or_fixture_not_ui(self):
        error = report.CheckError("tool_unavailable", "not_run")
        with patch.object(report, "run_json_script", side_effect=error):
            result = report.build_report("ui")
        mapped = result["checks"]["ui_mapping"]
        self.assertEqual(mapped["status"], "not_run")
        for item in mapped["files"]:
            self.assertNotIn("gap_rows", item)
        self.assertEqual(report.gap_row_lines(mapped), [])
        self.assertNotIn("row `", report.markdown_report(result))
        other = report.build_report("windows")["checks"]["ui_mapping"]
        self.assertEqual(report.gap_row_lines(other), [])

    def test_ui_gap_row_with_unsafe_values_is_rejected(self):
        row = {"id": "widgets:0", "source_kind": "widgets", "status": "partial",
               "mapping": {"status": "partial", "reason": "/abs/path"}}
        with self.assertRaises(report.CheckError):
            report.gap_row(row)
        row["mapping"] = {"status": "made_up_status", "reason": "valid_code"}
        with self.assertRaises(report.CheckError):
            report.gap_row(row)

    def api_gap_section(self, markdown):
        self.assertEqual(markdown.count("\n## API gaps\n"), 1)
        return markdown.split("\n## API gaps\n")[1].split("\n\n")[0].strip()

    def test_api_gaps_markdown_lists_real_unsupported_rows_with_line_and_api(self):
        expected = {
            "windows": [(2, "windows.h", "WIN-HEADER"), (4, "CreateFileW", "WIN-FILE-OPEN"),
                        (7, "ReadFile", "WIN-FILE-READ"), (8, "CloseHandle", "WIN-HANDLE-CLOSE")],
            "linux": [(2, "sys/inotify.h", "LNX-INOTIFY"), (3, "unistd.h", "POSIX-HEADER-FD"),
                      (5, "`IN_CLOEXEC`, `IN_NONBLOCK`, `inotify_init1`", "LNX-INOTIFY"),
                      (7, "`IN_MODIFY`, `inotify_add_watch`", "LNX-INOTIFY"),
                      (8, "close", "POSIX-FD-CLOSE")],
        }
        for platform, rows in expected.items():
            result = report.build_report(platform)
            markdown = report.markdown_report(result)
            section = self.api_gap_section(markdown)
            self.assertEqual(len(section.splitlines()), len(rows))
            for line, apis, rule in rows:
                apis = apis if apis.startswith("`") else "`{}`".format(apis)
                self.assertIn("- {}_diagnosis line {} · {} · unsupported (rule `{}`)".format(
                    platform, line, apis, rule), section)
            self.assertNotIn("omitted", section)
            self.assertLess(markdown.index("## API gaps"), markdown.index("Stage 1 checks metadata"))
            self.assertTrue(all(len(item) <= 200 for item in section.splitlines()))

    def test_api_gaps_markdown_leaves_json_unchanged_and_is_absent_for_portable_and_ui(self):
        for fixture in ("portable", "ui"):
            self.assertNotIn("API gaps", report.markdown_report(report.build_report(fixture)))
        result = report.build_report("windows")
        before = json.dumps(result["checks"]["windows_diagnosis"], sort_keys=True)
        report.markdown_report(result)
        self.assertEqual(before, json.dumps(result["checks"]["windows_diagnosis"], sort_keys=True))
        self.assertNotIn("API gaps", before)

    def test_api_gaps_markdown_shows_unresolved_rows_and_caps_api_names(self):
        result = report.build_report("windows")
        item = result["checks"]["windows_diagnosis"]
        names = ["api{}".format(index) for index in range(9)]
        item["rows"] = [
            {"line": 3, "rule_id": None, "apis": ["MysteryCall"], "availability": "available"},
            {"line": 5, "rule_id": "WIN-X", "apis": names + ["x" * 200], "availability": "unsupported"},
            {"line": 6, "rule_id": "WIN-OK", "apis": ["Fine"], "availability": "available"},
        ]
        section = self.api_gap_section(report.markdown_report(result))
        self.assertIn("- windows_diagnosis line 3 · `MysteryCall` · no rule matched (unresolved)", section)
        self.assertNotIn("Fine", section)
        shown = ", ".join("`{}`".format(name) for name in names[:report.MAX_API_MARKDOWN_NAMES])
        self.assertIn("line 5 · {}, +4 more · unsupported (rule `WIN-X`)".format(shown), section)
        self.assertEqual(len(section.splitlines()), 2)

    def test_api_gaps_markdown_shows_omission_when_truncated(self):
        with patch.object(report, "MAX_API_DETAILS", 2):
            result = report.build_report("linux")
        item = result["checks"]["linux_diagnosis"]
        self.assertTrue(item["truncated"])
        self.assertEqual(len(item["rows"]), 2)
        section = self.api_gap_section(report.markdown_report(result))
        self.assertEqual(section.count("unsupported (rule"), 2)
        self.assertIn("- linux_diagnosis … rows beyond the first 2 are omitted (see JSON counts)",
                      section)

    def test_api_gaps_appear_in_each_fixture_part_of_all_markdown(self):
        code, markdown = self.run_main("--fixture", "all", "--format", "markdown")
        self.assertEqual(code, 0)
        parts = {fixture: part for fixture, part in zip(
            report.FIXTURES, markdown.split("\n---\n\n")[1:])}
        self.assertIn("- windows_diagnosis line 4 · `CreateFileW` · unsupported", parts["windows"])
        self.assertIn("- linux_diagnosis line 5 · `IN_CLOEXEC`", parts["linux"])
        self.assertNotIn("API gaps", parts["portable"])
        self.assertNotIn("API gaps", parts["ui"])
        self.assertEqual(markdown.count("## API gaps"), 2)
        self.assertLessEqual(len(markdown.encode()), report.MAX_REPORT_BYTES * len(report.FIXTURES))

    def stage2_with_diagnostics(self, diagnostics, status="failed", reason="compile_failed"):
        """Orchestration-only stand-in for `stage2.run_stage2` (compile step failed)."""
        steps = [{"name": "compile", "status": "failed", "reason": "compile_failed",
                  "exit_code": 1, "captured_output_bytes": 10},
                 {"name": "run_test", "status": "not_run", "reason": "compile_did_not_succeed",
                  "exit_code": None, "captured_output_bytes": 0}]
        return lambda *args, **kwargs: {"stage": 2, "status": status, "reason": reason,
                                        "steps": steps, "diagnostics": diagnostics}

    def stage2_diagnostics_section(self, markdown):
        self.assertEqual(markdown.count("\n## Stage 2 diagnostics\n"), 1)
        return markdown.split("\n## Stage 2 diagnostics\n")[1].split("\n\n")[0].strip()

    @unittest.skipUnless(HAS_CLANG, "clang is required for the real host build")
    def test_stage2_diagnostics_real_build_has_no_items_and_no_markdown_section(self):
        result = report.build_report("portable")
        host = result["checks"]["stage_2"]
        self.assertEqual(host["status"], "passed")
        self.assertEqual(host["diagnostics"], {"items": [], "truncated": False})
        self.assert_counts(host, sources=2, headers=1, steps_run=2, steps_passed=2,
                           errors=0, warnings=0)
        self.assertNotIn("Stage 2 diagnostics", report.markdown_report(result))

    def test_stage2_diagnostics_absent_without_clang_or_for_other_fixtures(self):
        with patch.object(stage2.shutil, "which", return_value=None):
            result = report.build_report("portable")
        host = result["checks"]["stage_2"]
        self.assertEqual(host["reason"], "clang_unavailable")
        self.assertNotIn("diagnostics", host)
        self.assertNotIn("Stage 2 diagnostics", report.markdown_report(result))
        for fixture in ("windows", "ui"):
            other = report.build_report(fixture)
            self.assertNotIn("diagnostics", other["checks"]["stage_2"])
            self.assertNotIn("Stage 2 diagnostics", report.markdown_report(other))

    def test_stage2_diagnostics_items_are_listed_with_json_and_markdown(self):
        items = [{"file": "summary.c", "line": 12, "severity": "warning", "category": "-Wunused-variable"},
                 {"file": "summary.h", "line": 3, "severity": "error", "category": None}]
        diagnostics = {"errors": 1, "warnings": 1, "items": items, "truncated": False}
        with patch.object(stage2, "run_stage2", self.stage2_with_diagnostics(diagnostics)):
            result = report.build_report("portable")
        host = result["checks"]["stage_2"]
        self.assertEqual(host["status"], "failed")
        self.assertEqual(host["diagnostics"], {"items": items, "truncated": False})
        self.assert_counts(host, sources=2, headers=1, steps_run=1, steps_passed=0,
                           errors=1, warnings=1)
        self.assertEqual(result["version"], 1)
        section = self.stage2_diagnostics_section(report.markdown_report(result))
        self.assertEqual(section.splitlines(), [
            "- host_build summary.c:12 · warning · -Wunused-variable",
            "- host_build summary.h:3 · error · none"])
        self.assertNotIn(str(report.REPO), section)

    def test_stage2_diagnostics_markdown_is_capped_and_notes_omission(self):
        items = [{"file": "summary.c", "line": number, "severity": "warning", "category": "-Wextra"}
                 for number in range(1, report.MAX_STAGE2_MARKDOWN_ITEMS + 4)]
        diagnostics = {"errors": 0, "warnings": len(items), "items": items, "truncated": False}
        with patch.object(stage2, "run_stage2", self.stage2_with_diagnostics(diagnostics)):
            result = report.build_report("portable")
        self.assertEqual(len(result["checks"]["stage_2"]["diagnostics"]["items"]), len(items))
        lines = self.stage2_diagnostics_section(report.markdown_report(result)).splitlines()
        self.assertEqual(len(lines), report.MAX_STAGE2_MARKDOWN_ITEMS + 1)
        self.assertEqual(lines[-1], "- host_build … more diagnostics omitted (see JSON counts)")

    def test_stage2_diagnostics_json_cap_marks_truncated(self):
        items = [{"file": "summary.c", "line": number, "severity": "warning", "category": None}
                 for number in range(1, 6)]
        diagnostics = {"errors": 0, "warnings": 5, "items": items, "truncated": False}
        with patch.object(stage2, "run_stage2", self.stage2_with_diagnostics(diagnostics)), \
                patch.object(report, "MAX_STAGE2_DIAGNOSTIC_ITEMS", 2):
            host = report.build_report("portable")["checks"]["stage_2"]
        self.assertEqual(host["diagnostics"], {"items": items[:2], "truncated": True})

    def test_stage2_diagnostics_with_invalid_item_fail_as_invalid_tool_output(self):
        good = {"file": "summary.c", "line": 4, "severity": "warning", "category": "-Wextra"}
        for label, change in (
                ("absolute path", {"file": "/Users/someone/summary.c"}),
                ("unknown file", {"file": "other.c"}),
                ("non-integer line", {"line": "4"}),
                ("bool line", {"line": True}),
                ("zero line", {"line": 0}),
                ("unknown severity", {"severity": "note"}),
                ("message as category", {"category": "unused variable 'x'"}),
                ("non-string category", {"category": 7}),
                ("extra key", {"message": "text"})):
            with self.subTest(label):
                diagnostics = {"errors": 0, "warnings": 1, "items": [{**good, **change}],
                               "truncated": False}
                with patch.object(stage2, "run_stage2", self.stage2_with_diagnostics(diagnostics)):
                    result = report.build_report("portable")
                host = result["checks"]["stage_2"]
                self.assertEqual((host["status"], host["reason"]), ("failed", "invalid_tool_output"))
                self.assertNotIn("diagnostics", host)
                self.assertNotIn("Stage 2 diagnostics", report.markdown_report(result))
        with patch.object(stage2, "run_stage2", self.stage2_with_diagnostics(
                {"errors": 0, "warnings": 0, "items": "x", "truncated": False})):
            host = report.build_report("portable")["checks"]["stage_2"]
        self.assertEqual(host["reason"], "invalid_tool_output")

    def test_missing_validator_does_not_stop_portable_build(self):
        with patch.object(validate, "VALIDATOR", Path("/nonexistent/nagi-pkg")):
            result = report.build_report("portable")
        checks = result["checks"]
        self.assertEqual(checks["generation"]["status"], "passed")
        self.assertEqual(checks["stage_1"]["status"], "not_run")
        self.assertEqual(checks["stage_1"]["reason"], "validator_unavailable")
        self.assert_counts(checks["stage_1"], manifests=0)
        self.assert_portable(checks)
        self.assert_target_not_run(result)

    def test_cli_fails_when_stage2_rejects_portable_input_before_compile(self):
        with tempfile.TemporaryDirectory(prefix="ndp-report-portable-") as directory:
            root = Path(directory) / "checkout"
            validation_dir = root / SCRIPT.parent.relative_to(report.REPO)
            validation_dir.mkdir(parents=True)
            for name in ("report.py", "generated_check.py", "validate.py", "stage2.py", "stages.json"):
                shutil.copyfile(SCRIPT.with_name(name), validation_dir / name)
            for relative in (Path("tools/nagi-developer-platform/generator"),
                             report.EVALUATION / "portable"):
                shutil.copytree(report.REPO / relative, root / relative)
            schema = Path("sdk/rust/schemas/app-manifest.schema.json")
            (root / schema.parent).mkdir(parents=True)
            shutil.copyfile(report.REPO / schema, root / schema)
            source = root / report.EVALUATION / "portable/summary.c"
            outside = root.parent / source.name
            source.rename(outside)
            source.symlink_to(outside)

            with self.subTest(reason="input_unavailable"):
                self.assert_stage2_input_failure(root, "input_unavailable")

            source.unlink()
            with source.open("wb") as oversized:
                oversized.truncate(validate.MAX_INPUT + 1)
            with self.subTest(reason="input_too_large"):
                self.assert_stage2_input_failure(root, "input_too_large")

            source.write_bytes(outside.read_bytes() + b"\n/* altered fixture */\n")
            with self.subTest(reason="fixture_digest_mismatch"):
                self.assert_stage2_input_failure(root, "fixture_digest_mismatch")

    def assert_stage2_input_failure(self, root, reason):
        run = self.run_cli("--fixture", "portable", root=root)
        self.assertEqual(run.stderr, b"")
        result = json.loads(run.stdout)
        checks = result["checks"]
        self.assertEqual(checks["generation"]["status"], "passed", checks["generation"])
        host = checks["stage_2"]
        self.assertEqual((run.returncode, host["status"], host["reason"]), (1, "failed", reason))
        self.assert_counts(host, sources=2, headers=1, steps_run=0, steps_passed=0)
        self.assertEqual(host["steps"], [])
        self.assert_target_not_run(result)

    def test_cli_rejects_path_as_fixture(self):
        for selection in ("../evaluation/ui", str(report.REPO / report.EVALUATION / "ui")):
            with self.subTest(selection=selection):
                run = self.run_cli("--fixture", selection)
                self.assertEqual(run.returncode, 2)
                self.assertEqual(run.stdout, b"")

    def test_diagnosis_refuses_linked_evidence_and_linked_parent(self):
        source = report.EVALUATION / "windows/read_prefix.c"
        evidence = report.API_EVIDENCE / "windows/read_prefix.report-v1.json"
        for ancestor in (False, True):
            with self.subTest(ancestor=ancestor), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / source.parent).mkdir(parents=True)
                shutil.copyfile(report.REPO / source, root / source)
                if ancestor:
                    (root / evidence.parent.parent).mkdir(parents=True)
                    (root / evidence.parent).symlink_to(report.REPO / evidence.parent)
                else:
                    (root / evidence.parent).mkdir(parents=True)
                    (root / evidence).symlink_to(report.REPO / evidence)
                with patch.object(report, "REPO", root):
                    result = report.diagnose("windows", "windows")
                self.assertEqual((result["status"], result["reason"]), ("failed", "input_unavailable"))
                self.assert_counts(result)

    def test_component_output_limit_is_reported_and_generation_still_runs(self):
        with patch.object(report, "MAX_TOOL_OUTPUT", 128):
            result = report.build_report("linux")
        checks = result["checks"]
        self.assertEqual(checks["linux_diagnosis"]["status"], "failed")
        self.assertEqual(checks["linux_diagnosis"]["reason"], "output_limit")
        self.assertEqual(checks["generation"]["status"], "passed")
        self.assert_target_not_run(result)


if __name__ == "__main__":
    unittest.main()
