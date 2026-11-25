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
