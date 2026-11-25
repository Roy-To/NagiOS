"""Pin the `report_output_limit` branch of `report.main()` and the real size headroom.

`main()` renders the JSON/Markdown report, and when the UTF-8 size exceeds the limit
(`MAX_REPORT_BYTES` for one fixture, `MAX_REPORT_BYTES * len(FIXTURES)` for `--fixture all`)
it writes `report_output_limit` to stderr, writes nothing to stdout and returns 1.
Within the limit it writes the output and the exit code depends only on `failed` checks.

The branch tests reuse the real reports built once in `setUpClass`; `build_report` and
`build_all_report` are stubbed to return them, because these tests target the `main()`
orchestration and not the real subsystem. The headroom tests call the real, unstubbed
`report.main()` and measure the UTF-8 bytes of its captured stdout.
Host-only; not Nagi target acceptance. The headroom ratio is a PROVISIONAL assumption
(see README.md), not an approved contract.
"""

import contextlib
import copy
import io
import json
import unittest
from unittest import mock

import report


# PROVISIONAL assumption: real output stays below this fraction of its limit.
HEADROOM_RATIO = 0.5


def render(result, fmt, fixture):
    if fixture == "all":
        return (report.markdown_all_report(result) if fmt == "markdown"
                else json.dumps(result, indent=2) + "\n")
    return (report.markdown_report(result) if fmt == "markdown"
            else json.dumps(result, indent=2) + "\n")


def size(text):
    return len(text.encode("utf-8"))


class OutputLimitTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.all_report = report.build_all_report()
        cls.single = dict(cls.all_report["reports"])

    def run_main(self, fixture, fmt, all_report=None, single=None, max_bytes=None):
        all_report = self.all_report if all_report is None else all_report
        single = self.single[fixture] if (single is None and fixture != "all") else single
        out, err = io.StringIO(), io.StringIO()
        patches = [mock.patch.object(report, "build_all_report", return_value=all_report),
                   mock.patch.object(report, "build_report", return_value=single)]
        if max_bytes is not None:
            patches.append(mock.patch.object(report, "MAX_REPORT_BYTES", max_bytes))
        with contextlib.ExitStack() as stack:
            for patch in patches:
                stack.enter_context(patch)
            stack.enter_context(contextlib.redirect_stdout(out))
            stack.enter_context(contextlib.redirect_stderr(err))
            code = report.main(["--fixture", fixture, "--format", fmt])
        return code, out.getvalue(), err.getvalue()

    def test_over_limit_single_fixture_both_formats(self):
        for fixture in report.FIXTURES:
            for fmt in ("json", "markdown"):
                with self.subTest(fixture=fixture, format=fmt):
                    code, out, err = self.run_main(fixture, fmt, max_bytes=1)
                    self.assertEqual(code, 1)
                    self.assertEqual(out, "")
                    self.assertEqual(err.strip(), "report_output_limit")

    def test_over_limit_all_both_formats(self):
        for fmt in ("json", "markdown"):
            with self.subTest(format=fmt):
                code, out, err = self.run_main("all", fmt, max_bytes=1)
                self.assertEqual(code, 1)
                self.assertEqual(out, "")
                self.assertEqual(err.strip(), "report_output_limit")

    def test_limit_boundary_single_fixture(self):
        fixture = report.FIXTURES[0]
        for fmt in ("json", "markdown"):
            expected = render(self.single[fixture], fmt, fixture)
            with self.subTest(format=fmt, case="exact"):
                code, out, err = self.run_main(fixture, fmt, max_bytes=size(expected))
                self.assertEqual(out, expected)
                self.assertEqual(err, "")
                self.assertEqual(code, int(any(
                    item["status"] == "failed"
                    for item in self.single[fixture]["checks"].values())))
            with self.subTest(format=fmt, case="one_byte_over"):
                code, out, err = self.run_main(fixture, fmt, max_bytes=size(expected) - 1)
                self.assertEqual((code, out, err.strip()), (1, "", "report_output_limit"))

    def test_all_limit_scales_with_fixture_count(self):
        for fmt in ("json", "markdown"):
            expected = render(self.all_report, fmt, "all")
            per_fixture = -(-size(expected) // len(report.FIXTURES))  # ceil
            with self.subTest(format=fmt, case="at_limit"):
                code, out, err = self.run_main("all", fmt, max_bytes=per_fixture)
                self.assertEqual(out, expected)
                self.assertEqual(err, "")
                self.assertIn(code, (0, 1))
            with self.subTest(format=fmt, case="below_limit"):
                code, out, err = self.run_main(
                    "all", fmt, max_bytes=(size(expected) - 1) // len(report.FIXTURES))
                self.assertEqual((code, out, err.strip()), (1, "", "report_output_limit"))

    def test_within_limit_exit_code_depends_only_on_failed_checks(self):
        fixture = report.FIXTURES[0]
        base = self.single[fixture]
        clean = copy.deepcopy(base)
        for item in clean["checks"].values():
            if item["status"] == "failed":
                item["status"] = "not_run"
        failing = copy.deepcopy(clean)
        next(iter(failing["checks"].values()))["status"] = "failed"
        for fmt in ("json", "markdown"):
            for label, data, expected_code in (("clean", clean, 0), ("failed", failing, 1)):
                with self.subTest(format=fmt, case=label):
                    code, out, err = self.run_main(fixture, fmt, single=data)
                    self.assertEqual(code, expected_code)
                    self.assertEqual(out, render(data, fmt, fixture))
                    self.assertEqual(err, "")
        # --fixture all: a failed check in any report makes the exit code 1.
        all_clean = copy.deepcopy(self.all_report)
        for item in all_clean["reports"].values():
            for check in item["checks"].values():
                if check["status"] == "failed":
                    check["status"] = "not_run"
        all_failing = copy.deepcopy(all_clean)
        last = list(all_failing["reports"].values())[-1]
        next(iter(last["checks"].values()))["status"] = "failed"
        for fmt in ("json", "markdown"):
            for label, data, expected_code in (("clean", all_clean, 0),
                                               ("failed", all_failing, 1)):
                with self.subTest(format=fmt, case="all_" + label):
                    code, out, err = self.run_main("all", fmt, all_report=data)
                    self.assertEqual(code, expected_code)
                    self.assertEqual(out, render(data, fmt, "all"))
                    self.assertEqual(err, "")


def run_real_main(fixture, fmt):
    """Call the real, unstubbed `report.main()` and capture its stdout/stderr."""
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = report.main(["--fixture", fixture, "--format", fmt])
    return code, out.getvalue(), err.getvalue()


class OutputHeadroomTests(unittest.TestCase):
    """Real CLI output (unstubbed `report.main()` stdout) stays well under the limits."""

    def assert_headroom(self, fixture, fmt, limit):
        code, out, err = run_real_main(fixture, fmt)
        self.assertNotEqual(err.strip(), "report_output_limit")
        self.assertIn(code, (0, 1))
        self.assertTrue(out, "{} {} wrote no output".format(fixture, fmt))
        actual = size(out)
        self.assertLess(actual, limit * HEADROOM_RATIO,
                        "{} {} output is {} of {} bytes".format(fixture, fmt, actual, limit))

    def test_single_fixture_sizes_leave_headroom(self):
        for fixture in report.FIXTURES:
            for fmt in ("json", "markdown"):
                with self.subTest(fixture=fixture, format=fmt):
                    self.assert_headroom(fixture, fmt, report.MAX_REPORT_BYTES)

    def test_all_fixture_sizes_leave_headroom(self):
        limit = report.MAX_REPORT_BYTES * len(report.FIXTURES)
        for fmt in ("json", "markdown"):
            with self.subTest(format=fmt):
                self.assert_headroom("all", fmt, limit)


if __name__ == "__main__":
    unittest.main()
