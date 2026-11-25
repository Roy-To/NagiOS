"""handover.md must record every migration-report fixture and link its source README.

A new name in `report.FIXTURES` fails here until `handover.md` lists it, so a
fixture added later cannot be left out of the handover record.
"""
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]
VALIDATION = ROOT / "tools" / "nagi-developer-platform" / "validation"
HANDOVER = ROOT / "docs" / "development" / "developer-platform" / "handover.md"
README_LINK = "../../../tools/nagi-developer-platform/validation/README.md"

sys.path.insert(0, str(VALIDATION))
try:
    import report
finally:
    sys.path.remove(str(VALIDATION))


def handover_text(path=HANDOVER):
    return Path(path).read_text(encoding="utf-8")


def missing_fixtures(text):
    # Whole backticked names only, so `ui` is not satisfied by `ui_unsupported`.
    return [name for name in report.FIXTURES if "`{}`".format(name) not in text]


class HandoverReportTests(unittest.TestCase):
    def test_fixture_list_is_not_empty(self):
        self.assertTrue(report.FIXTURES)

    def test_every_report_fixture_is_recorded(self):
        self.assertEqual(missing_fixtures(handover_text()), [])

    def test_missing_fixture_name_is_detected(self):
        text = handover_text()
        for name in report.FIXTURES:
            with self.subTest(fixture=name):
                self.assertIn(name, missing_fixtures(text.replace("`{}`".format(name), "")))

    def test_links_validation_readme_and_it_exists(self):
        text = handover_text()
        self.assertIn(README_LINK, text)
        self.assertTrue((HANDOVER.parent / README_LINK).resolve().is_file())

    def test_records_commands_limits_and_status(self):
        text = handover_text()
        self.assertIn("report.py --fixture all", text)
        self.assertIn("--format markdown", text)
        for status in ("passed", "failed", "not_run", "unsupported"):
            self.assertIn("`{}`".format(status), text)
        for phrase in ("Stage 3", "Nagi target", "実scanner", "実 `nagi-pkg`",
                       "PROVISIONAL", "contracts/"):
            self.assertIn(phrase, text)


if __name__ == "__main__":
    unittest.main()
