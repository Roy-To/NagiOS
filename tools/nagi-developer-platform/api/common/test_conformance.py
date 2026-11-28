"""Conformance tests of the shared adapter against PR #91 schema facts (Python 3.9).

The fixtures are hand-assembled from the scanner's documented output rules, not
scanner output; see tests/developer-platform/api/common/README.md.
"""

import copy
import json
from pathlib import Path
import sys
import unittest

_API_DIR = str(Path(__file__).resolve().parents[1])
if _API_DIR not in sys.path:
    sys.path.insert(0, _API_DIR)

from common import pt01_conformance as conformance  # noqa: E402
from common import pt01_report as adapter  # noqa: E402
from windows import diagnose  # noqa: E402

ROOT = Path(__file__).resolve().parents[4]
COMMON = ROOT / "tests/developer-platform/api/common"
OLD_FIXTURES = [ROOT / "tests/developer-platform/api/windows/read_prefix.report-v1.json",
                ROOT / "tests/developer-platform/api/windows/text_only.report-v1.json",
                ROOT / "tests/developer-platform/api/linux/watch_changes.report-v1.json"]
NAMES = ("pr91_win32_candidates", "pr91_truncated", "pr91_no_findings")


def raw(name):
    return (COMMON / (name + ".report-v1.json")).read_bytes()


def doc(name="pr91_win32_candidates"):
    return json.loads(raw(name).decode("utf-8"))


def enc(document):
    return json.dumps(document, ensure_ascii=True).encode("utf-8")


def codes(document):
    return conformance.check_document(document)


def finding(line=1, category="process", evidence="matched tokens: CreateProcess",
            path="a.c", review_status="requires-review"):
    return dict(category=category, path=path, line=line, evidence=evidence,
                review_status=review_status)


class FixtureTests(unittest.TestCase):
    def test_pr91_fixtures_are_conformant(self):
        for name in NAMES:
            with self.subTest(name=name):
                self.assertEqual((), conformance.check_report(raw(name)))

    def test_strict_adapter_projects_fixture(self):
        rows = adapter.adapt_report_v1(raw("pr91_win32_candidates"),
                                       allowed_paths=["win_worker.c"], strict=True)
        self.assertEqual(7, len(rows))
        self.assertEqual({"requires-review"}, {r.review_status for r in rows})
        self.assertEqual((2, 3, 4, 5, 6, 7, 8), tuple(r.line for r in rows))
        self.assertEqual("matched tokens: socket(, connect(", rows[2].evidence)

    def test_pre_conformance_fixtures_do_not_match_pr91(self):
        for path in OLD_FIXTURES:
            with self.subTest(fixture=path.name):
                found = conformance.check_report(path.read_bytes())
                self.assertIn("unknown_category", found)
                self.assertIn("unknown_review_status", found)
                self.assertIn("evidence_format", found)
                with self.assertRaisesRegex(adapter.ReportError, "^pr91_conformance_violation$"):
                    adapter.adapt_report_v1(path.read_bytes(), allowed_paths=[
                        r["path"] for r in json.loads(path.read_bytes())["findings"]], strict=True)

    def test_lenient_default_is_unchanged(self):
        document = doc()
        document["findings"][0]["category"] = "windows-api"
        document["findings"][0]["review_status"] = "needs_review"
        rows = adapter.adapt_report_v1(enc(document), allowed_paths=["win_worker.c"])
        self.assertEqual("windows-api", rows[0].category)
        with self.assertRaisesRegex(adapter.ReportError, "^pr91_conformance_violation$"):
            adapter.adapt_report_v1(enc(document), allowed_paths=["win_worker.c"], strict=True)


class RequiredFieldTests(unittest.TestCase):
    def test_missing_top_level_fields(self):
        for name in ("schema_version", "detected_languages", "manifests", "dependencies",
                     "analysis", "findings", "limitations"):
            with self.subTest(name=name):
                document = doc()
                del document[name]
                self.assertIn("missing_field:" + name, codes(document))

    def test_missing_finding_and_analysis_fields(self):
        for name in ("category", "path", "line", "evidence", "review_status"):
            with self.subTest(name=name):
                document = doc()
                del document["findings"][0][name]
                self.assertIn("finding_missing_field", codes(document))
        for name in ("files_scanned", "directories_visited", "entries_visited",
                     "bytes_read", "truncated", "conclusion"):
            with self.subTest(name=name):
                document = doc()
                del document["analysis"][name]
                self.assertIn("analysis_missing_field", codes(document))

    def test_missing_field_is_rejected_by_strict_adapter_too(self):
        document = doc()
        del document["analysis"]
        with self.assertRaisesRegex(adapter.ReportError, "^pr91_conformance_violation$"):
            adapter.adapt_report_v1(enc(document), allowed_paths=["win_worker.c"], strict=True)

    def test_non_object_root_and_version(self):
        for value in ([], "x", 1, None):
            self.assertEqual(("root_not_object",), codes(value))
        for version in (2, 0, "1", 1.0, True, None):
            with self.subTest(version=version):
                document = doc()
                document["schema_version"] = version
                self.assertIn("schema_version", codes(document))


class EnumAndTypeTests(unittest.TestCase):
    def mutate(self, **fields):
        document = doc()
        document["findings"][0].update(fields)
        return codes(document)

    def test_unknown_enums(self):
        self.assertIn("unknown_category", self.mutate(category="windows-api"))
        self.assertIn("unknown_category", self.mutate(category="Process"))
        self.assertIn("unknown_category", self.mutate(category=["process"]))  # unhashable
        self.assertIn("unknown_category", self.mutate(category=None))
        self.assertIn("unknown_review_status", self.mutate(review_status="needs_review"))
        self.assertIn("unknown_review_status", self.mutate(review_status="approved"))
        self.assertIn("unknown_review_status", self.mutate(review_status={"a": 1}))
        document = doc()
        document["detected_languages"] = ["c", "cobol"]
        self.assertIn("invalid_type_or_enum:detected_languages", codes(document))
        document = doc()
        document["analysis"]["conclusion"] = "compatible"
        self.assertIn("invalid_type_or_enum:analysis", codes(document))
        document = doc()
        document["dependencies"] = [{"name": "x", "source": "y", "review_status": "ok"}]
        self.assertIn("invalid_type_or_enum:dependencies", codes(document))

    def test_line_null_zero_and_wrong_types(self):
        for value in (None, 0, -1, "1", 1.0, 1.5, True, [], {}):
            with self.subTest(line=value):
                self.assertIn("invalid_line", self.mutate(line=value))
                document = doc()
                document["findings"][0]["line"] = value
                with self.assertRaisesRegex(adapter.ReportError, "^invalid_finding_line$"):
                    adapter.adapt_report_v1(enc(document), allowed_paths=["win_worker.c"])

    def test_path_evidence_and_type_errors(self):
        for value in (None, 7, "", [], {}):
            self.assertIn("invalid_path_type", self.mutate(path=value))
        for value in (None, 7, "", "CreateProcess", {"tokens": []}, ["x"], "matched tokens:"):
            with self.subTest(evidence=value):
                self.assertIn("evidence_format", self.mutate(evidence=value))
        # token is valid for another category but not this one
        self.assertIn("evidence_token_not_in_category",
                      self.mutate(evidence="matched tokens: CreateThread"))
        self.assertIn("evidence_token_not_in_category",
                      self.mutate(evidence="matched tokens: CreateProcess, bogus"))
        self.assertIn("evidence_token_not_in_category",
                      self.mutate(evidence="matched tokens: CreateProcess,  system("))
        # a finding row of the wrong JSON type
        document = doc()
        document["findings"].append("x")
        self.assertIn("invalid_type_or_enum:findings", codes(document))
        document = doc()
        document["findings"] = {}
        self.assertIn("invalid_type_or_enum:findings", codes(document))

    def test_analysis_types(self):
        for name, value in (("truncated", 0), ("truncated", "false"), ("files_scanned", -1),
                            ("bytes_read", 1.5), ("entries_visited", None),
                            ("directories_visited", True)):
            with self.subTest(name=name, value=value):
                document = doc()
                document["analysis"][name] = value
                self.assertIn("invalid_type_or_enum:analysis", codes(document))

    def test_extra_fields_are_reported_but_not_reflected(self):
        document = doc()
        document["secret-looking-key"] = 1
        document["analysis"]["extra"] = 1
        document["findings"][0]["extra"] = 1
        found = codes(document)
        for code in ("unexpected_field:root", "unexpected_field:analysis",
                     "unexpected_field:finding"):
            self.assertIn(code, found)
        self.assertNotIn("secret", "".join(found))

    def test_codes_never_reflect_report_content(self):
        document = doc()
        document["findings"][0]["category"] = "TOP-SECRET-/Users/someone"
        document["findings"][0]["evidence"] = "password=hunter2"
        text = "".join(codes(document))
        for needle in ("SECRET", "Users", "hunter2"):
            self.assertNotIn(needle, text)


class SemanticTests(unittest.TestCase):
    def test_order_and_duplicates(self):
        document = doc()
        document["findings"].reverse()
        self.assertIn("findings_not_sorted_unique", codes(document))
        document = doc()
        document["findings"].append(copy.deepcopy(document["findings"][-1]))
        self.assertIn("findings_not_sorted_unique", codes(document))

    def test_conclusion_must_match_state(self):
        document = doc("pr91_no_findings")
        document["analysis"]["conclusion"] = "requires-review"
        self.assertIn("conclusion_inconsistent", codes(document))
        document = doc()
        document["analysis"]["conclusion"] = "no-known-boundary-detected"
        self.assertIn("conclusion_inconsistent", codes(document))
        document = doc("pr91_no_findings")
        document["analysis"]["truncated"] = True  # truncated alone requires review
        self.assertIn("conclusion_inconsistent", codes(document))

    def test_required_limitations(self):
        document = doc()
        document["limitations"] = document["limitations"][1:]
        self.assertIn("required_limitation_missing", codes(document))
        document["limitations"] = [None, 1]
        self.assertIn("required_limitation_missing", codes(document))

    def test_replacement_schema_changes_the_verdict(self):
        schema = conformance.load_schema()
        schema["categories"]["windows-api"] = ["CreateFileW"]
        document = doc()
        document["findings"][0].update(category="windows-api",
                                       evidence="matched tokens: CreateFileW")
        self.assertIn("unknown_category", codes(document))
        self.assertEqual((), conformance.check_document(document, schema=schema))

    def test_schema_data_lists_every_pr91_category(self):
        self.assertEqual({"filesystem", "process", "threading", "networking",
                          "dynamic-loading", "gui-framework", "os-contract"},
                         set(conformance.load_schema()["categories"]))

    def test_completeness_flag(self):
        self.assertEqual((False, "requires-review"), conformance.completeness(raw("pr91_win32_candidates")))
        self.assertEqual((True, "requires-review"), conformance.completeness(raw("pr91_truncated")))
        self.assertEqual((False, "no-known-boundary-detected"),
                         conformance.completeness(raw("pr91_no_findings")))
        self.assertEqual((None, None), conformance.completeness(b'{"analysis": {}}'))
        self.assertEqual((None, None), conformance.completeness(b"[]"))
        with self.assertRaisesRegex(adapter.ReportError, "^invalid_report_json$"):
            conformance.completeness(b"{")


class ScannerEmissionTests(unittest.TestCase):
    """Things the scanner can emit that the bounded adapter must still handle."""

    def big(self, count):
        document = doc("pr91_no_findings")
        document["findings"] = [finding(line=i + 1) for i in range(count)]
        document["analysis"]["conclusion"] = "requires-review"
        return document

    def test_scanner_finding_cap_is_accepted(self):
        data = enc(self.big(10000))
        self.assertLessEqual(len(data), adapter.MAX_REPORT_BYTES)
        self.assertEqual((), conformance.check_report(data))
        rows = adapter.adapt_report_v1(data, allowed_paths=["a.c"], strict=True)
        self.assertEqual(10000, len(rows))

    def test_one_past_cap_is_still_bounded(self):
        with self.assertRaisesRegex(adapter.ReportError, "^finding_count_limit$"):
            adapter.adapt_report_v1(enc(self.big(10001)), allowed_paths=["a.c"])

    def test_paths_the_scanner_may_emit_but_the_adapter_refuses(self):
        # POSIX file names may contain these; the adapter fails closed (stricter on purpose).
        for path in ("a:b.c", "a\\b.c", "100%.c", "~tmp.c", "dir/./x.c", "a\nb.c"):
            with self.subTest(path=path):
                document = doc("pr91_no_findings")
                document["findings"] = [finding(path=path)]
                document["analysis"]["conclusion"] = "requires-review"
                self.assertEqual((), conformance.check_document(document))
                with self.assertRaisesRegex(adapter.ReportError, "^invalid_relative_path$"):
                    adapter.adapt_report_v1(enc(document), allowed_paths=[path])

    def test_lossy_non_utf8_name_and_non_ascii_path_are_accepted(self):
        for path in ("src/�.c", "src/読み込み.c"):
            document = doc("pr91_no_findings")
            document["findings"] = [finding(path=path)]
            document["analysis"]["conclusion"] = "requires-review"
            rows = adapter.adapt_report_v1(enc(document), allowed_paths=[path], strict=True)
            self.assertEqual(path, rows[0].path)


class WindowsDiagnosisOnPr91ShapeTests(unittest.TestCase):
    def run_fixture(self, name, paths):
        def strict(data, *, allowed_paths):
            return adapter.adapt_report_v1(data, allowed_paths=allowed_paths, strict=True)
        return diagnose.diagnose_report(raw(name), allowed_paths=paths, adapter=strict)

    def test_reachable_windows_tokens_and_unclassified_rest(self):
        rows = self.run_fixture("pr91_win32_candidates", ["win_worker.c"])["rows"]
        by_line = {r["source"]["line"]: r for r in rows}
        self.assertEqual(7, len(rows))
        self.assertEqual(("WIN-THREAD", ["CreateThread"]),
                         (by_line[5]["rule_id"], by_line[5]["original_apis"]))
        for line in (6, 8):
            self.assertEqual(("WIN-PROCESS", ["CreateProcess"]),
                             (by_line[line]["rule_id"], by_line[line]["original_apis"]))
        # No bundled rule for these tokens; real categories are not the provisional Windows ones.
        for line in (2, 3, 4, 7):
            self.assertIsNone(by_line[line]["rule_id"])
            self.assertEqual("not_classified", by_line[line]["detection"])
        for row in rows:
            self.assertEqual("report_fragment_only", row["source"]["evidence_scope"])
            self.assertEqual("requires-review", row["source"]["review_status"])
            self.assertEqual("PROVISIONAL", row["approval"])

    def test_win32_file_apis_are_not_reachable_from_pr91_evidence(self):
        # PR #91 evidence is a fixed token list; none is a Win32 file/registry/UI name,
        # so those rules cannot fire from a conformant report.
        tokens = {t for toks in conformance.load_schema()["categories"].values() for t in toks}
        rules = diagnose.load_rules()["rules"]
        reachable = {r["id"] for r in rules for t in tokens
                     if any(diagnose._matches(diagnose.evidence_tokens(t), a,
                                              r.get("case_insensitive") is True,
                                              r.get("basename_only") is True)
                            for a in r["aliases"])}
        self.assertEqual({"WIN-PROCESS", "WIN-THREAD"}, reachable)


if __name__ == "__main__":
    unittest.main()
