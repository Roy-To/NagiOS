"""Stage 2 bounded/redacted diagnostics summary (PROVISIONAL shape, see README).

Real-clang tests use deliberately broken or noisy copies of the self-made fixture.
Fake-compiler tests only exercise bounds/orchestration. Host result only.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

import stage2
import test_stage2
from test_stage2 import REPO, RELATIVE, Copy, needs_clang

EMPTY = {"errors": 0, "warnings": 0, "items": [], "truncated": False}
SECRET_BODY = "ndp_secret_body_identifier"
SECRET_ENV = "ndp-secret-env-value-9f3"


def warning_target(**changes):
    """A target without -Werror so warnings do not fail the build."""
    entry = {"id": "portable-summary", "directory": RELATIVE, "sources": ["summary.c", "summary_test.c"],
             "headers": ["summary.h"], "flags": ["-std=c11", "-Wall", "-Wextra", "-O0",
                                                  "-fno-color-diagnostics"],
             "output": "summary_test", "expect_stdout_prefix": "PASS portable summary:"}
    return {"portable-summary": stage2.Target({**entry, **changes})}


class SummarizeUnitTests(unittest.TestCase):
    NAMES = ("summary.c", "summary.h")

    def summarize(self, text, **kwargs):
        return stage2.summarize_diagnostics(text.encode(), self.NAMES, ("/tmp/nagi-ndp-stage2-abc",), **kwargs)

    def test_counts_and_items_keep_only_file_line_severity_category(self):
        text = ("summary.c:3:5: error: use of undeclared identifier 'secret'\n"
                "    3 |     secret = 1;\n      |     ^\n"
                "summary.h:9:1: warning: unused thing [-Wunused-variable]\n"
                "summary.c:4:1: note: declared here\n"
                "2 diagnostics generated.\n")
        result = self.summarize(text)
        self.assertEqual((result["errors"], result["warnings"], result["truncated"]), (1, 1, False))
        self.assertEqual(result["items"], [
            {"file": "summary.c", "line": 3, "severity": "error", "category": None},
            {"file": "summary.h", "line": 9, "severity": "warning", "category": "-Wunused-variable"}])
        self.assertNotIn("secret", json.dumps(result))

    def test_scratch_prefix_is_stripped_and_foreign_paths_are_dropped(self):
        text = ("/tmp/nagi-ndp-stage2-abc/summary.c:7:2: error: x\n"
                "/etc/passwd:1:1: error: leak\n"
                "/Users/someone/summary.c:1:1: error: foreign\n"
                "../summary.c:1:1: error: traversal\n"
                "other.c:1:1: error: unknown file\n")
        result = self.summarize(text)
        self.assertEqual(result["items"], [{"file": "summary.c", "line": 7, "severity": "error",
                                            "category": None}])
        self.assertEqual(result["errors"], 1)
        self.assertNotIn("/tmp", json.dumps(result))

    def test_fatal_error_counts_as_error_and_bad_category_is_dropped(self):
        result = self.summarize("summary.c:1:10: fatal error: 'x.h' file not found\n"
                                "summary.c:2:1: warning: m [-WBad_Flag]\n"
                                "summary.c:3:1: warning: m [-Wvalid-flag] trailing\n")
        self.assertEqual((result["errors"], result["warnings"]), (1, 2))
        self.assertEqual([item["category"] for item in result["items"]], [None, None, None])

    def test_werror_bracket_and_dot_slash_prefix(self):
        result = self.summarize("./summary.h:4:2: error: unused [-Werror,-Wunused-variable]\n"
                                "summary.c:5:2: error: only [-Werror]\n")
        self.assertEqual(result["items"], [
            {"file": "summary.h", "line": 4, "severity": "error", "category": "-Wunused-variable"},
            {"file": "summary.c", "line": 5, "severity": "error", "category": None}])

    def test_item_count_and_character_limits(self):
        text = "".join(f"summary.c:{n}:1: warning: w [-Wunused-variable]\n" for n in range(1, 101))
        by_count = self.summarize(text, max_items=5)
        self.assertEqual((len(by_count["items"]), by_count["warnings"], by_count["truncated"]), (5, 100, True))
        by_chars = self.summarize(text, max_chars=200)
        self.assertTrue(by_chars["truncated"])
        self.assertLessEqual(sum(len(json.dumps(i, sort_keys=True)) for i in by_chars["items"]), 200)
        self.assertGreater(len(by_chars["items"]), 0)
        self.assertEqual(by_chars["warnings"], 100)

    def test_empty_and_garbage_input(self):
        self.assertEqual(self.summarize(""), EMPTY)
        self.assertEqual(self.summarize("\x00\xff garbage\r\n::::\n"), EMPTY)


@needs_clang
class RealClangDiagnosticsTests(Copy):
    def line_of(self, name, needle):
        for number, text in enumerate((self.fixture / name).read_text().splitlines(), 1):
            if needle in text:
                return number
        self.fail(needle)

    def test_passed_run_has_empty_diagnostics(self):
        result = self.run2()
        self.assertEqual((result["status"], result["reason"]), ("passed", "build_and_test_passed"))
        self.assertEqual(result["diagnostics"], EMPTY)

    def test_compile_error_reports_count_and_fixture_relative_location(self):
        self.edit("summary.c", "return 0;", "return 0 +;")
        line = self.line_of("summary.c", "return 0 +;")
        result = self.run2()
        self.assertEqual((result["status"], result["reason"]), ("failed", "compile_failed"))
        diagnostics = result["diagnostics"]
        self.assertGreaterEqual(diagnostics["errors"], 1)
        self.assertIn({"file": "summary.c", "line": line, "severity": "error", "category": None},
                      diagnostics["items"])
        self.assertEqual(result["steps"][1]["status"], "not_run")
        self.assertEqual(result["steps"][0]["exit_code"], 1)  # existing fields unchanged

    def test_werror_warning_is_reported_with_its_category(self):
        self.edit("summary.c", "struct task_summary result", "int unused; struct task_summary result")
        result = self.run2()
        self.assertEqual(result["reason"], "compile_failed")
        self.assertTrue(any(item["category"] == "-Wunused-variable" and item["file"] == "summary.c"
                            for item in result["diagnostics"]["items"]))

    def test_header_error_is_reported_against_the_header(self):
        self.edit("summary.h", "#endif", "#error ndp header failure\n#endif")
        line = self.line_of("summary.h", "#error")
        result = self.run2()
        self.assertIn({"file": "summary.h", "line": line, "severity": "error", "category": None},
                      result["diagnostics"]["items"])

    def test_warning_only_build_passes_and_lists_warnings(self):
        self.edit("summary.c", "struct task_summary result", "int unused_ndp; struct task_summary result")
        line = self.line_of("summary.c", "unused_ndp")
        result = self.run2(targets=warning_target())
        self.assertEqual((result["status"], result["reason"]), ("passed", "build_and_test_passed"))
        diagnostics = result["diagnostics"]
        self.assertEqual((diagnostics["errors"], diagnostics["warnings"]), (0, 1))
        self.assertEqual(diagnostics["items"], [{"file": "summary.c", "line": line, "severity": "warning",
                                                 "category": "-Wunused-variable"}])

    def test_no_paths_body_or_environment_values_leak(self):
        self.edit("summary.c", "return 0;", f"return {SECRET_BODY} +;")
        text_before = (self.fixture / "summary.c").read_text()
        with patch.dict(os.environ, {"NDP_SECRET": SECRET_ENV, "HOME": str(self.base / "home-marker")}):
            result = self.run2()
        self.assertEqual(result["reason"], "compile_failed")
        self.assertTrue(result["diagnostics"]["items"])
        text = json.dumps(result)
        for forbidden in (SECRET_BODY, SECRET_ENV, str(self.base), str(self.base.parent), "nagi-ndp-stage2-",
                          "home-marker", "/private/", "/var/", "/tmp", str(REPO), "return 0"):
            self.assertNotIn(forbidden, text)
        self.assertEqual((self.fixture / "summary.c").read_text(), text_before)
        for item in result["diagnostics"]["items"]:
            self.assertEqual(set(item), {"file", "line", "severity", "category"})
            self.assertIn(item["file"], ("summary.c", "summary.h", "summary_test.c"))
            self.assertIsInstance(item["line"], int)

    def test_huge_real_output_is_bounded_by_items_and_characters(self):
        functions = "".join(f"static void ndp_noise_{n}(void) {{ int ndp_unused_{n}; }}\n" for n in range(150))
        with (self.fixture / "summary.c").open("a") as handle:
            handle.write(functions)
        result = self.run2(targets=warning_target())
        self.assertEqual(result["status"], "passed")
        diagnostics = result["diagnostics"]
        self.assertGreaterEqual(diagnostics["warnings"], 150)
        self.assertTrue(diagnostics["truncated"])
        self.assertLessEqual(len(diagnostics["items"]), stage2.MAX_DIAGNOSTIC_ITEMS)
        self.assertLessEqual(len(json.dumps(diagnostics["items"], sort_keys=True)),
                             stage2.MAX_DIAGNOSTIC_CHARS + 2 * stage2.MAX_DIAGNOSTIC_ITEMS)
        self.assertLessEqual(sum(len(json.dumps(i, sort_keys=True)) for i in diagnostics["items"]),
                             stage2.MAX_DIAGNOSTIC_CHARS)

    def test_cli_includes_diagnostics_without_raw_output(self):
        done = subprocess.run([sys.executable, "-B", str(Path(stage2.__file__)), "--fixture-dir", RELATIVE],
                              cwd=REPO, capture_output=True, timeout=120)
        self.assertEqual((done.returncode, done.stderr), (0, b""))
        self.assertEqual(json.loads(done.stdout)["diagnostics"], EMPTY)


class BoundedFloodTests(Copy):
    """Fake compilers print diagnostic-shaped lines; only bounds are observed here."""

    def test_unbounded_diagnostic_flood_stays_within_limits(self):
        flood = self.script("while :; do echo 'summary.c:1:1: error: ndp-secret-flood [-Wflood]'; done")
        result = self.run2(clang=flood)
        self.assertEqual(result["status"], "output_limit")
        diagnostics = result["diagnostics"]
        self.assertTrue(diagnostics["truncated"])
        self.assertLessEqual(len(diagnostics["items"]), stage2.MAX_DIAGNOSTIC_ITEMS)
        self.assertLessEqual(sum(len(json.dumps(i, sort_keys=True)) for i in diagnostics["items"]),
                             stage2.MAX_DIAGNOSTIC_CHARS)
        self.assertNotIn("ndp-secret-flood", json.dumps(result))
        self.assertLessEqual(len(json.dumps(result)), 4096)

    def test_absolute_scratch_paths_from_the_compiler_are_normalised(self):
        script = self.script('echo "$PWD/summary.c:2:3: error: msg containing $PWD" >&2; exit 1')
        result = self.run2(clang=script)
        self.assertEqual(result["diagnostics"]["items"], [
            {"file": "summary.c", "line": 2, "severity": "error", "category": None}])
        self.assertNotIn("nagi-ndp-stage2-", json.dumps(result))

    def test_compiler_text_without_location_is_not_copied(self):
        result = self.run2(clang=self.script("echo diagnostic >&2; exit 5"))
        self.assertEqual((result["reason"], result["steps"][0]["exit_code"]), ("compile_failed", 5))
        self.assertEqual(result["diagnostics"], EMPTY)
        self.assertNotIn("diagnostic ", json.dumps(result).replace('"diagnostics"', ""))


class RejectionResultsUnchangedTests(Copy):
    def test_rejections_and_unavailable_carry_no_diagnostics(self):
        for kwargs in ({"prior": {"stage": 1, "status": "failed"}}, {"compile_timeout": 0},
                       {"fixture_dir": "elsewhere"}):
            with self.subTest(kwargs=kwargs):
                result = self.run2(**kwargs)
                self.assertEqual(result["status"], "not_run")
                self.assertNotIn("diagnostics", result)
                self.assertEqual(result["steps"], [])
        unknown = stage2.run_stage2("missing", cwd=self.base, base=self.base)
        self.assertEqual((unknown["status"], unknown["reason"]), ("not_run", "unknown_target"))
        self.assertNotIn("diagnostics", unknown)
        missing = self.run2(clang="/not/a/clang")
        self.assertEqual((missing["status"], missing["reason"]), ("unavailable", "clang_unavailable"))
        self.assertNotIn("diagnostics", missing)
        self.assertEqual(stage2.run_stage3()["steps"], [])
        self.assertNotIn("diagnostics", stage2.run_stage3())

    def test_exit_codes_are_unchanged(self):
        self.assertEqual(stage2.EXIT_CODES, {"passed": 0, "unavailable": 2, "not_run": 3, "timeout": 124,
                                             "cancelled": 130})


if __name__ == "__main__":
    unittest.main()
