"""handover.md must record every migration-report fixture and link its source README.

A new name in `report.FIXTURES` fails here until `handover.md` lists it, so a
fixture added later cannot be left out of the handover record.
"""
from pathlib import Path
import io
import re
import shutil
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
VALIDATION = ROOT / "tools" / "nagi-developer-platform" / "validation"
HANDOVER = ROOT / "docs" / "development" / "developer-platform" / "handover.md"
README_LINK = "../../../tools/nagi-developer-platform/validation/README.md"
SECTION_HEADING = "## 6. 一通しの移植レポート"

sys.path.insert(0, str(VALIDATION))
try:
    import report
finally:
    sys.path.remove(str(VALIDATION))


def handover_text(path=HANDOVER):
    return Path(path).read_text(encoding="utf-8")


def report_section(text):
    """Return the migration-report section, from its heading to the next `## `."""
    start = text.find(SECTION_HEADING)
    if start < 0:
        return ""
    end = text.find("\n## ", start + len(SECTION_HEADING))
    return text[start:] if end < 0 else text[start:end]


def missing_fixtures(text):
    # Whole backticked names only, so `ui` is not satisfied by `ui_unsupported`.
    return [name for name in report.FIXTURES if "`{}`".format(name) not in text]


def readme_link_targets(text):
    """Relative README destinations used as real Markdown links `[label](target)`."""
    pattern = r"\[[^\]\n]+\]\((" + re.escape(README_LINK) + r")(?:#[^)\s]*)?\)"
    return re.findall(pattern, text)


class HandoverConsistency(unittest.TestCase):
    """The consistency checks; `handover_path` is swapped for the temp-copy check."""

    handover_path = HANDOVER

    def section(self):
        return report_section(handover_text(self.handover_path))

    def test_fixture_list_is_not_empty(self):
        self.assertTrue(report.FIXTURES)

    def test_every_report_fixture_is_recorded(self):
        self.assertEqual(missing_fixtures(self.section()), [])

    def test_section_links_validation_readme_and_it_exists(self):
        targets = readme_link_targets(self.section())
        self.assertTrue(targets, "no Markdown link to {}".format(README_LINK))
        for target in targets:
            self.assertTrue((Path(self.handover_path).parent / target).resolve().is_file())

    def test_records_commands_limits_and_status(self):
        text = self.section()
        commands = [line for line in text.splitlines()
                    if "validation/report.py --fixture" in line]
        self.assertGreaterEqual(len(commands), 2)
        self.assertIn("report.py --fixture all", text)
        self.assertIn("--format markdown", text)
        for status in ("passed", "failed", "not_run", "unsupported"):
            self.assertIn("`{}`".format(status), text)
        for phrase in ("Stage 3", "Nagi target", "host限定", "実scanner", "実 `nagi-pkg`",
                       "未検証", "PROVISIONAL", "contracts/"):
            self.assertIn(phrase, text)


UNVERIFIED_HEADING = "## 3. 未検証事項"
RESOLVED_MARKER = "**解消済みのP3**"
REMAINING_MARKER = "**いま残る点**"
OLD_REMAINING_MARKER = "**残るP3**"
# (task id, old wording that must not reappear in the remaining list, relative link targets)
TOOLS = "../../../tools/nagi-developer-platform/"
RESOLVED_P3 = (
    ("NDP-A001", ("`<sub/windows.h>` が `windows.h` に一致し得る",),
     (TOOLS + "api/windows/README.md", TOOLS + "api/windows/test_windows.py")),
    ("NDP-A002", ("`0o700` 厳密判定がumaskで誤判定し得る",),
     (TOOLS + "generator/README.md", TOOLS + "generator/test_generate.py")),
    ("NDP-A003", ("Stage 2が診断出力を返さない",),
     (TOOLS + "validation/README.md", TOOLS + "validation/test_stage2_diagnostics.py")),
)


def markdown_link_destinations(text):
    """Exact destinations of `[label](dest)` links, without any `#fragment`."""
    return re.findall(r"\[[^\]\n]+\]\(([^)\s#]+)(?:#[^)\s]*)?\)", text)


def p3_parts(text):
    """Split section 3's P3 notes into (resolved, remaining) text."""
    start = text.find(UNVERIFIED_HEADING)
    end = text.find("\n## ", start + len(UNVERIFIED_HEADING)) if start >= 0 else -1
    section = "" if start < 0 else (text[start:] if end < 0 else text[start:end])
    resolved = section.find(RESOLVED_MARKER)
    remaining = section.find(REMAINING_MARKER)
    if resolved < 0 or remaining < resolved:
        return "", "", section
    return section[resolved:remaining], section[remaining:], section


class ResolvedP3Consistency(unittest.TestCase):
    """Items recorded as resolved must be linked and not listed again as remaining."""

    handover_path = HANDOVER

    def parts(self):
        return p3_parts(handover_text(self.handover_path))

    def test_resolved_and_remaining_are_separate_lists(self):
        resolved, remaining, section = self.parts()
        self.assertTrue(resolved, "missing {}".format(RESOLVED_MARKER))
        self.assertTrue(remaining, "missing {}".format(REMAINING_MARKER))
        self.assertNotIn(OLD_REMAINING_MARKER, section)

    def test_each_resolved_item_is_recorded_with_existing_links(self):
        resolved, _, _ = self.parts()
        destinations = markdown_link_destinations(resolved)
        for task, _, targets in RESOLVED_P3:
            with self.subTest(task=task):
                self.assertIn(task, resolved)
                for target in targets:
                    self.assertIn(target, destinations)
        for destination in destinations:
            with self.subTest(link=destination):
                self.assertTrue((Path(self.handover_path).parent / destination).resolve().is_file(),
                                destination)

    def test_resolved_items_are_not_listed_as_remaining(self):
        _, remaining, _ = self.parts()
        self.assertTrue(remaining)
        for task, old_wordings, _ in RESOLVED_P3:
            with self.subTest(task=task):
                for wording in old_wordings:
                    self.assertNotIn(wording, remaining)

    def test_prior_self_declaration_stays_listed_as_remaining(self):
        _, remaining, _ = self.parts()
        self.assertIn("`prior`", remaining)
        self.assertIn("自己申告", remaining)


class ResolvedP3MutationTests(unittest.TestCase):
    """Edit a temporary copy of handover.md and confirm the resolved-P3 checks fail."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, str(self.tmp))
        self.copy = self.tmp / HANDOVER.relative_to(ROOT)
        self.copy.parent.mkdir(parents=True)
        for _, _, targets in RESOLVED_P3:
            for target in targets:
                path = (HANDOVER.parent / target).resolve()
                dest = (self.copy.parent / target).resolve()
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(str(path), str(dest))

    def failed(self, text):
        self.copy.write_text(text, encoding="utf-8")
        case = type("Copy", (ResolvedP3Consistency,), {"handover_path": self.copy})
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(case)
        result = unittest.TextTestRunner(stream=io.StringIO(), verbosity=0).run(suite)
        # A subTest failure is reported on a wrapper whose id carries " (key=value)",
        # and values may contain dots (link paths); take the method name from the case.
        return sorted({getattr(t, "test_case", t)._testMethodName
                       for t, _ in result.failures + result.errors})

    def test_unmodified_copy_passes(self):
        self.assertEqual(self.failed(handover_text()), [])

    def test_old_wording_in_remaining_list_fails(self):
        for _, old_wordings, _ in RESOLVED_P3:
            for wording in old_wordings:
                with self.subTest(wording=wording):
                    text = handover_text().replace(
                        REMAINING_MARKER, REMAINING_MARKER + "\n  - " + wording, 1)
                    self.assertEqual(self.failed(text), ["test_resolved_items_are_not_listed_as_remaining"])

    def test_missing_link_target_fails(self):
        target = self.copy.parent / RESOLVED_P3[0][2][0]
        target.resolve().unlink()
        self.assertEqual(self.failed(handover_text()), ["test_each_resolved_item_is_recorded_with_existing_links"])

    def test_link_changed_to_a_different_file_fails(self):
        # The original target still exists in the copy; only an exact match catches this.
        target = RESOLVED_P3[0][2][1]
        text = handover_text().replace("]({})".format(target), "]({}.bak)".format(target), 1)
        self.assertNotEqual(text, handover_text())
        self.assertEqual(self.failed(text), ["test_each_resolved_item_is_recorded_with_existing_links"])

    def test_dropping_prior_item_fails(self):
        text = handover_text().replace("自己申告", "申告")
        self.assertEqual(self.failed(text), ["test_prior_self_declaration_stays_listed_as_remaining"])


def run_consistency_on(path):
    """Run every consistency test against `path` and return the unittest result."""
    case = type("HandoverCopy", (HandoverConsistency,), {"handover_path": Path(path)})
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(case)
    return unittest.TextTestRunner(stream=io.StringIO(), verbosity=0).run(suite)


class HandoverMutationTests(unittest.TestCase):
    """Edit a temporary copy of handover.md and confirm the normal checks fail."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, str(self.tmp))
        # Same depth below the copy root, so the relative README link still resolves.
        self.copy = self.tmp / HANDOVER.relative_to(ROOT)
        self.copy.parent.mkdir(parents=True)
        readme = self.tmp / "tools" / "nagi-developer-platform" / "validation" / "README.md"
        readme.parent.mkdir(parents=True)
        shutil.copyfile(str(VALIDATION / "README.md"), str(readme))

    def write_copy(self, text):
        self.copy.write_text(text, encoding="utf-8")

    def failed_tests(self, result):
        return sorted(test.id().rsplit(".", 1)[-1] for test, _ in result.failures + result.errors)

    def test_unmodified_copy_passes(self):
        self.write_copy(handover_text())
        result = run_consistency_on(self.copy)
        self.assertTrue(result.wasSuccessful(), self.failed_tests(result))

    def test_removed_fixture_name_fails_on_temp_copy(self):
        text = handover_text()
        for name in report.FIXTURES:
            with self.subTest(fixture=name):
                self.write_copy(text.replace("`{}`".format(name), ""))
                result = run_consistency_on(self.copy)
                self.assertEqual(self.failed_tests(result), ["test_every_report_fixture_is_recorded"])

    def test_broken_markdown_link_fails_on_temp_copy(self):
        self.write_copy(handover_text().replace("](" + README_LINK, "] (" + README_LINK))
        result = run_consistency_on(self.copy)
        self.assertEqual(self.failed_tests(result), ["test_section_links_validation_readme_and_it_exists"])

    def test_missing_readme_target_fails_on_temp_copy(self):
        self.write_copy(handover_text())
        (self.tmp / "tools" / "nagi-developer-platform" / "validation" / "README.md").unlink()
        result = run_consistency_on(self.copy)
        self.assertEqual(self.failed_tests(result), ["test_section_links_validation_readme_and_it_exists"])


if __name__ == "__main__":
    unittest.main()
