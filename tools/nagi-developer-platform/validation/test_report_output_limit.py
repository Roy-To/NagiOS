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
import pathlib
import re
import tempfile
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


README = pathlib.Path(__file__).with_name("README.md")
SECTION = "Report output limit and size headroom"
TABLE_HEADER = re.compile(
    r"^\|\s*`--fixture`\s*\|\s*limit \(bytes\)\s*\|\s*JSON bytes \(% of limit\)\s*\|"
    r"\s*Markdown bytes \(% of limit\)\s*\|$")
TABLE_SEPARATOR = re.compile(r"^\|(\s*:?-{3,}:?\s*\|){4}$")
# Any backquoted name is captured (e.g. `ui-invalid`), so unknown names are reported
# as unknown rows instead of being skipped. Byte cells must parse but are not pinned.
SIZE_CELL = r"\s*\d+ \(\d+(?:\.\d+)?%\)\s*"
TABLE_ROW = re.compile(
    r"^\|\s*`(?P<name>[^`|]+)`\s*\|\s*(?P<limit>\d+)\s*\|" + SIZE_CELL + r"\|"
    + SIZE_CELL + r"\|$")


def size_table_rows(readme_text):
    """Return `(rows, problems)` for the README size table.

    `rows` is a list of `(name, limit)` in table order. As in GFM, the table runs from
    the separator row to the first blank line (or the end of the text), and its rows
    may omit the leading and/or trailing `|`. Every such line must be a well-formed
    data row; any other line is reported as `unparsable row`.
    """
    start = readme_text.find(SECTION)
    if start < 0:
        return [], ["section '{}' not found".format(SECTION)]
    lines = [line.strip() for line in readme_text[start:].splitlines()]
    header = next((i for i, line in enumerate(lines) if TABLE_HEADER.match(line)), None)
    if header is None:
        return [], ["size table header not found"]
    if header + 1 >= len(lines) or not TABLE_SEPARATOR.match(lines[header + 1]):
        return [], ["size table separator row missing"]
    rows, problems = [], []
    for line in lines[header + 2:]:
        if not line:
            break  # the table ended
        # Review P2 (round 2): a GFM row without the outer pipes is still a row.
        normalized = line if line.startswith("|") else "| " + line
        normalized = normalized if normalized.endswith("|") else normalized + " |"
        match = TABLE_ROW.match(normalized)
        if match:
            rows.append((match.group("name"), int(match.group("limit"))))
        else:
            problems.append("unparsable row: " + line)
    return rows, problems


def size_table_problems(readme_text):
    """Compare the README size table with `report.FIXTURES` (rows and limits only).

    Byte counts are host/environment dependent and are deliberately not checked.
    Returns a list of problem strings; empty means the table is in sync.
    """
    parsed, problems = size_table_rows(readme_text)
    rows = {}
    for name, limit in parsed:
        if name in rows:
            problems.append("duplicate row: " + name)
        rows[name] = limit
    expected = {name: report.MAX_REPORT_BYTES for name in report.FIXTURES}
    expected["all"] = report.MAX_REPORT_BYTES * len(report.FIXTURES)
    for name in sorted(set(expected) - set(rows)):
        problems.append("missing row: " + name)
    for name in sorted(set(rows) - set(expected)):
        problems.append("unknown row: " + name)
    for name in sorted(set(expected) & set(rows)):
        if rows[name] != expected[name]:
            problems.append("limit mismatch for {}: {} != {}".format(
                name, rows[name], expected[name]))
    return problems


class ReadmeSizeTableSyncTests(unittest.TestCase):
    """The README size table lists every `report.FIXTURES` fixture and `all` (NDP-A026)."""

    @classmethod
    def setUpClass(cls):
        cls.readme = README.read_text(encoding="utf-8")

    def check_copy(self, text):
        """Run the check against a temporary copy of the README, not the real file."""
        with tempfile.TemporaryDirectory() as tmp:
            copy_path = pathlib.Path(tmp) / "README.md"
            copy_path.write_text(text, encoding="utf-8")
            return size_table_problems(copy_path.read_text(encoding="utf-8"))

    def drop_row(self, name):
        lines = self.readme.splitlines(keepends=True)
        kept = []
        for line in lines:
            match = TABLE_ROW.match(line.strip())
            if not (match and match.group("name") == name):
                kept.append(line)
        self.assertEqual(len(kept), len(lines) - 1, "row {} not found exactly once".format(name))
        return "".join(kept)

    def test_readme_table_matches_fixtures_and_limits(self):
        self.assertIn("ui_invalid", report.FIXTURES)
        self.assertEqual(size_table_problems(self.readme), [])

    def test_table_without_ui_invalid_row_fails(self):
        problems = self.check_copy(self.drop_row("ui_invalid"))
        self.assertIn("missing row: ui_invalid", problems)

    def test_table_without_any_fixture_row_fails(self):
        for name in (*report.FIXTURES, "all"):
            with self.subTest(fixture=name):
                self.assertIn("missing row: " + name, self.check_copy(self.drop_row(name)))

    def test_table_with_extra_or_wrong_limit_row_fails(self):
        extra = self.readme.replace(
            "  | `all` |", "  | `ghost` | 131072 | 1 (0.0%) | 1 (0.0%) |\n  | `all` |", 1)
        self.assertNotEqual(extra, self.readme)
        self.assertIn("unknown row: ghost", self.check_copy(extra))
        all_limit = report.MAX_REPORT_BYTES * len(report.FIXTURES)
        # Review P3 (round 2): derive the wrong value so it differs for any fixture count.
        wrong = self.readme.replace("| `all` | {} |".format(all_limit),
                                    "| `all` | {} |".format(all_limit + 1), 1)
        self.assertNotEqual(wrong, self.readme)
        self.assertTrue(any(p.startswith("limit mismatch for all")
                            for p in self.check_copy(wrong)))

    def test_table_with_hyphenated_extra_row_fails(self):
        # Review P2 (round 1): a name outside [a-z_0-9] must not be skipped silently.
        extra = self.readme.replace(
            "  | `all` |",
            "  | `ui-invalid` | 131072 | 4592 (3.5%) | 2851 (2.2%) |\n  | `all` |", 1)
        self.assertNotEqual(extra, self.readme)
        problems = self.check_copy(extra)
        self.assertIn("unknown row: ui-invalid", problems)
        self.assertNotEqual(problems, [])

    def test_table_rows_without_outer_pipes_are_checked(self):
        # Review P2 (round 2): GFM rows may omit the leading and/or trailing `|`.
        for extra in ("  `ghost` | 131072 | 1 (0.0%) | 1 (0.0%) |",
                      "  | `ghost` | 131072 | 1 (0.0%) | 1 (0.0%)",
                      "  `ghost` | 131072 | 1 (0.0%) | 1 (0.0%)"):
            with self.subTest(row=extra):
                text = self.readme.replace(
                    "  | `all` | {} |".format(report.MAX_REPORT_BYTES * len(report.FIXTURES)),
                    extra + "\n  | `all` | {} |".format(
                        report.MAX_REPORT_BYTES * len(report.FIXTURES)), 1)
                self.assertNotEqual(text, self.readme)
                self.assertIn("unknown row: ghost", self.check_copy(text))
        # A row after `all` (the reviewer's reproduction) is still inside the table.
        lines = self.readme.splitlines(keepends=True)
        index = next(i for i, line in enumerate(lines) if line.strip().startswith("| `all` |"))
        lines.insert(index + 1, "  `ghost` | 131072 | 1 (0.0%) | 1 (0.0%) |\n")
        self.assertIn("unknown row: ghost", self.check_copy("".join(lines)))
        # A known row written without outer pipes is parsed, not reported as missing.
        dropped = self.drop_row("ui_invalid")
        bare = dropped.replace(
            "  | `all` |", "  `ui_invalid` | 131072 | 4592 (3.5%) | 2851 (2.2%)\n  | `all` |", 1)
        self.assertNotEqual(bare, dropped)
        self.assertEqual(self.check_copy(bare), [])

    def test_table_with_non_row_line_before_blank_fails(self):
        # A non-table line before the blank line is still part of the GFM table.
        text = self.readme.replace("  | `all` |", "  ghost\n  | `all` |", 1)
        self.assertNotEqual(text, self.readme)
        self.assertIn("unparsable row: ghost", self.check_copy(text))

    def test_table_with_unparsable_row_fails(self):
        for bad in ("  | ui_invalid | 131072 | 4592 (3.5%) | 2851 (2.2%) |",
                    "  | `ghost` | 131072 |",
                    "  | `ghost` | many | 1 (0.0%) | 1 (0.0%) |",
                    "  | `ghost` | 131072 | 1 | 1 (0.0%) |"):
            with self.subTest(row=bad):
                text = self.readme.replace("  | `all` |", bad + "\n  | `all` |", 1)
                self.assertNotEqual(text, self.readme)
                problems = self.check_copy(text)
                self.assertIn("unparsable row: " + bad.strip(), problems)

    def test_table_without_header_or_separator_fails(self):
        header = ("| `--fixture` | limit (bytes) | JSON bytes (% of limit) "
                  "| Markdown bytes (% of limit) |")
        self.assertIn(header, self.readme)
        no_header = self.readme.replace(header, "| fixture | limit | json | md |", 1)
        self.assertIn("size table header not found", self.check_copy(no_header))
        no_separator = self.readme.replace("|---|---:|---:|---:|", "", 1)
        self.assertNotEqual(no_separator, self.readme)
        problems = self.check_copy(no_separator)
        self.assertIn("size table separator row missing", problems)
        self.assertIn("missing row: ui_invalid", problems)


if __name__ == "__main__":
    unittest.main()
