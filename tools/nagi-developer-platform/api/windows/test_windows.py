"""Host tests of the production adapter and JSON rules (Python 3.9)."""

import copy
import hashlib
import io
import json
import os
from pathlib import Path
import random
import subprocess
import sys
import unittest
from unittest import mock

# Discovery may start at `api/` (README) or at `api/windows/` (runner); in the
# latter only this directory is on sys.path, so add `api/` for the shared
# `common` adapter and the `windows` package (no duplicate implementation).
_API_DIR = str(Path(__file__).resolve().parents[1])
if _API_DIR not in sys.path:
    sys.path.insert(0, _API_DIR)

from common import pt01_report as adapter  # noqa: E402
from windows import diagnose


ROOT = Path(__file__).resolve().parents[4]
FIXTURES = ROOT / "tests/developer-platform/api/windows"
EVALUATION = ROOT / "tests/developer-platform/evaluation"


def report(evidence="CreateFileW", **fields):
    row = dict(category="windows-api", path="sample.c", line=1,
               evidence=evidence, review_status="needs_review")
    row.update(fields)
    return {"schema_version": 1, "findings": [row]}


def encode(document):
    return json.dumps(document, ensure_ascii=True).encode("utf-8")


def run(document, **kwargs):
    return diagnose.diagnose_report(encode(document), allowed_paths=["sample.c"], **kwargs)


class AdapterTests(unittest.TestCase):
    def assert_invalid(self, document, code):
        with self.assertRaisesRegex(adapter.ReportError, "^" + code + "$"):
            adapter.adapt_report_v1(encode(document), allowed_paths=["sample.c"])

    def test_version_and_collection_types(self):
        for version in (True, False, None, "1", 1.0, 0, 2):
            with self.subTest(version=version):
                self.assert_invalid(dict(schema_version=version, findings=[]), "unsupported_report_version")
        for findings in (None, {}, "", 0):
            self.assert_invalid(dict(schema_version=1, findings=findings), "invalid_findings")
        self.assert_invalid([], "unsupported_report_version")
        self.assert_invalid({"schema_version": 1}, "invalid_findings")
        self.assert_invalid({"schema_version": 1, "findings": [None]}, "invalid_finding")

    def test_required_finding_fields_and_types(self):
        for field in ("category", "path", "line", "evidence", "review_status"):
            for value in (None, [], {}):
                document = report()
                document["findings"][0][field] = value
                with self.subTest(field=field, value=value), self.assertRaises(adapter.ReportError):
                    adapter.adapt_report_v1(encode(document), allowed_paths=["sample.c"])
            document = report()
            del document["findings"][0][field]
            with self.assertRaises(adapter.ReportError):
                adapter.adapt_report_v1(encode(document), allowed_paths=["sample.c"])

    def test_invalid_lines(self):
        for line in (True, False, 0, -1, 2.5, "1", 2147483648):
            self.assert_invalid(report(line=line), "invalid_finding_line")

    def test_unsafe_paths_and_explicit_scope(self):
        for path in ("/private/file.c", "../file.c", "a/../file.c", "./sample.c",
                     "a//file.c", "C:/file.c", "C:\\file.c", "\\\\server\\file.c",
                     "file:///tmp/file.c", "%2e%2e/file.c", "~user/file.c", "a\nfile.c", "\ud800.c", ""):
            with self.subTest(path=path):
                self.assert_invalid(report(path=path), "invalid_relative_path")
        self.assert_invalid(report(path="other.c"), "finding_outside_selected_paths")
        for paths in (None, "sample.c", {"sample.c": True}):
            with self.assertRaisesRegex(adapter.ReportError, "explicit_path_allowlist_required"):
                adapter.adapt_report_v1(encode(report()), allowed_paths=paths)
        with self.assertRaisesRegex(adapter.ReportError, "finding_outside_selected_paths"):
            adapter.adapt_report_v1(encode(report()), allowed_paths=[])

    def test_json_corruption_and_duplicate_keys(self):
        inputs = (b"", b"{", b"\xff", b'{}{}', b'{"schema_version":1,"schema_version":1}',
                  b'{"schema_version":1,"findings":[],"extra":NaN}',
                  b'{"schema_version":1,"findings":[],"extra":Infinity}',
                  b'{"schema_version":1,"findings":[],"extra":1e999}',
                  b'{"schema_version":1,"findings":[],"extra":{"a":1,"a":2}}')
        for data in inputs:
            with self.subTest(data=data), self.assertRaises(adapter.ReportError):
                adapter.adapt_report_v1(data, allowed_paths=[])
        with self.assertRaisesRegex(adapter.ReportError, "report_requires_bytes"):
            adapter.adapt_report_v1("{}", allowed_paths=[])

    def test_limits_bytes_findings_evidence_and_metadata(self):
        with self.assertRaisesRegex(adapter.ReportError, "report_byte_limit"):
            adapter.adapt_report_v1(b" " * (adapter.MAX_REPORT_BYTES + 1), allowed_paths=[])
        self.assert_invalid({"schema_version": 1, "findings": [{}] * (adapter.MAX_FINDINGS + 1)},
                            "finding_count_limit")
        self.assert_invalid(report("A" * (adapter.MAX_EVIDENCE_CHARS + 1)), "invalid_finding_evidence")
        self.assert_invalid(report("\ud800"), "invalid_finding_evidence")
        for evidence in ("", "  ", "CreateFileW\x00"):
            self.assert_invalid(report(evidence), "invalid_finding_evidence")
        for field in ("category", "review_status"):
            for value in ("x" * 129, "/private/path", "x\nsecret", ""):
                self.assert_invalid(report(**{field: value}), "invalid_finding_" + field)

    def test_ignored_metadata_has_depth_node_and_number_limits(self):
        nested = []
        for _ in range(adapter.MAX_DEPTH + 1):
            nested = [nested]
        document = dict(schema_version=1, findings=[], ignored=nested)
        self.assert_invalid(document, "json_structure_limit")
        self.assert_invalid(dict(schema_version=1, findings=[], ignored=[0] * adapter.MAX_VALUES),
                            "json_structure_limit")
        for data in (b"[" * 2000 + b"]" * 2000,
                     b'{"schema_version":1,"findings":[],"extra":' + b"9" * 2000 + b"}"):
            with self.assertRaises(adapter.ReportError):
                adapter.adapt_report_v1(data, allowed_paths=[])

    def test_bounded_stream_read_does_not_drain(self):
        stream = io.BytesIO(b" " * (adapter.MAX_REPORT_BYTES + 100))
        with self.assertRaisesRegex(adapter.ReportError, "report_byte_limit"):
            adapter.read_report(stream)
        self.assertEqual(adapter.MAX_REPORT_BYTES + 1, stream.tell())

    def test_utf8_crlf_and_immutable_normalization(self):
        document = report("ReadFile(buffer); // 日本語\r\n", path="src/読み込み.c")
        rows = adapter.adapt_report_v1(encode(document), allowed_paths=["src/読み込み.c"])
        self.assertEqual("src/読み込み.c", rows[0].path)
        self.assertIsInstance(rows, tuple)
        with self.assertRaises(AttributeError):
            rows[0].line = 10

    def test_extra_fields_ignored_and_not_forwarded(self):
        document = report()
        document["analysis"] = {"secret": "PRIVATE_METADATA", "command": "do not execute"}
        document["findings"][0]["sdk_override"] = "FAKE_API"
        output = diagnose.json_report(run(document))
        self.assertNotIn("PRIVATE_METADATA", output)
        self.assertNotIn("FAKE_API", output)


class WindowsRulesTests(unittest.TestCase):
    def test_handwritten_report_matches_existing_windows_source(self):
        fixture = FIXTURES / "read_prefix.report-v1.json"
        source = EVALUATION / "windows/read_prefix.c"
        before = {p: hashlib.sha256(p.read_bytes()).digest() for p in (fixture, source)}
        data = fixture.read_bytes()
        lines = source.read_text(encoding="utf-8").splitlines()
        for row in json.loads(data)["findings"]:
            self.assertEqual(lines[row["line"] - 1], row["evidence"])
        output = diagnose.diagnose_report(data, allowed_paths=["read_prefix.c"])
        self.assertEqual(["WIN-HEADER", "WIN-FILE-OPEN", "WIN-FILE-READ", "WIN-HANDLE-CLOSE"],
                         [r["rule_id"] for r in output["rows"]])
        self.assertTrue(all(r["availability"] == "unsupported" for r in output["rows"]))
        self.assertTrue(all(r["gaps"] and not r["alternatives"] for r in output["rows"]))
        for path, digest in before.items():
            self.assertEqual(digest, hashlib.sha256(path.read_bytes()).digest())

    def test_negative_control_matches_existing_source(self):
        data = (FIXTURES / "text_only.report-v1.json").read_bytes()
        lines = (EVALUATION / "controls/text_only.c").read_text().splitlines()
        for row in json.loads(data)["findings"]:
            self.assertEqual(lines[row["line"] - 1], row["evidence"])
        output = diagnose.diagnose_report(data, allowed_paths=["text_only.c"])
        self.assertTrue(all(r["rule_id"] is None for r in output["rows"]))

    def test_win32_crt_and_dotnet_representatives(self):
        examples = [("WriteFile(h, b)", "WIN-FILE-WRITE", "win32"),
                    ("_wfopen(p, mode)", "CRT-FILE", "crt"),
                    ("fopen_s(&f, p, mode)", "CRT-FILE", "crt"),
                    ("_snprintf(buf, size, fmt)", "CRT-TEXT", "crt"),
                    ("System.IO.File.ReadAllText(path)", "DOTNET-FILE", "dotnet"),
                    ("File.WriteAllText(path, text)", "DOTNET-FILE", "dotnet"),
                    ("System.Diagnostics.Process.Start(path)", "DOTNET-PROCESS", "dotnet"),
                    ("CreateThread(args)", "WIN-THREAD", "win32"),
                    ("CreateProcessW(args)", "WIN-PROCESS", "win32")]
        for evidence, rule_id, family in examples:
            with self.subTest(evidence=evidence):
                rows = run(report(evidence))["rows"]
                self.assertEqual(1, len(rows))
                self.assertEqual(rule_id, rows[0]["rule_id"])
                self.assertEqual(family, rows[0]["family"])
                self.assertEqual("lexical_candidate", rows[0]["detection"])
                self.assertEqual("unsupported", rows[0]["availability"])

    def test_windows_header_case_and_quoted_include(self):
        # NDP-04 review P3: Windows headers resolve case-insensitively, and the
        # quoted include form is as common as the angle form.
        for evidence in ("#include <windows.h>", "#include <Windows.h>", "#include <WINDOWS.H>",
                         '#include "windows.h"', '#include "Windows.h"', '#  include "WINDOWS.H" // x',
                         "#include <Windows.h>\r\n"):
            with self.subTest(evidence=evidence):
                rows = run(report(evidence, category="windows-header"))["rows"]
                self.assertEqual([("WIN-HEADER", ["windows.h"], "lexical_candidate")],
                                 [(r["rule_id"], r["original_apis"], r["detection"]) for r in rows])
        # Not the Windows header: comments, literals, other names, other paths.
        for evidence, detection in (("// #include <Windows.h>", "no_code_evidence"),
                                    ('/* #include "windows.h" */', "no_code_evidence"),
                                    ('"#include <windows.h>"', "no_code_evidence"),
                                    ('puts("#include \\"windows.h\\"");', "unknown_api"),
                                    ('#include "windowsx.h"', "unknown_api"),
                                    ('#include "mywindows.h"', "unknown_api"),
                                    ("#include <windows_h>", "unknown_api"),
                                    ("#define H windows", "unknown_api"),
                                    ('#include "windows" ".h"', "unknown_api")):
            with self.subTest(evidence=evidence):
                row = run(report(evidence, category="windows-header"))["rows"][0]
                self.assertEqual((detection, None), (row["detection"], row["rule_id"]))

    def test_windows_header_match_is_limited_to_bare_basename(self):
        # NDP-A001 (PROVISIONAL): a path-qualified include names a different file
        # (a local or third-party header), so it is not the bare Windows header.
        # The directory separator must not be erased by the lexer's barrier token.
        for evidence in ("#include <sub/windows.h>", "#include <sub\\windows.h>",
                         '#include "third_party/windows.h"', '#include "third_party\\Windows.H"',
                         "#include <sys/WINDOWS.H>", '#include "../windows.h"',
                         "#include <./windows.h>", "#include <windows.h/extra>",
                         '#include "a/b/windows.h"', "#include </windows.h>"):
            with self.subTest(evidence=evidence):
                row = run(report(evidence, category="windows-header"))["rows"][0]
                self.assertEqual(("unknown_api", None, []),
                                 (row["detection"], row["rule_id"], row["original_apis"]))
        # Other header names remain distinct from windows.h.
        for evidence in ('#include "windowsx.h"', '#include "mywindows.h"', "#include <windowsx.h>",
                         "#include <sub/windowsx.h>"):
            with self.subTest(evidence=evidence):
                row = run(report(evidence, category="windows-header"))["rows"][0]
                self.assertEqual(("unknown_api", None), (row["detection"], row["rule_id"]))
        # A bare include elsewhere in the same fragment is still reported (once).
        for evidence in ("#include <sub/windows.h>\n#include <windows.h>",
                         '#include "third_party/windows.h"\r\n#include "Windows.h"'):
            with self.subTest(evidence=evidence):
                rows = run(report(evidence, category="windows-header"))["rows"]
                self.assertEqual([("WIN-HEADER", ["windows.h"], "lexical_candidate")],
                                 [(r["rule_id"], r["original_apis"], r["detection"]) for r in rows])
        # Separator-only evidence is not code evidence.
        for evidence in ("/", "\\"):
            with self.subTest(evidence=evidence):
                row = run(report(evidence, category="windows-header"))["rows"][0]
                self.assertEqual("no_code_evidence", row["detection"])
        # The restriction is rule data, limited to the header rule; API names are unaffected.
        rules = diagnose.load_rules()
        self.assertEqual(["WIN-HEADER"], [r["id"] for r in rules["rules"] if r.get("basename_only")])
        self.assertEqual("WIN-FILE-READ", run(report("x / ReadFile(h)"))["rows"][0]["rule_id"])
        rules["rules"][0]["basename_only"] = False
        row = run(report("#include <sub/windows.h>", category="windows-header"), rules=rules)["rows"][0]
        self.assertEqual("WIN-HEADER", row["rule_id"])

    def test_case_folding_is_limited_to_rules_that_opt_in(self):
        # API names remain case-sensitive; only `case_insensitive` rule data folds.
        for evidence in ("createfilew(p)", "CREATEFILEW(p)", "readfile(h)"):
            with self.subTest(evidence=evidence):
                row = run(report(evidence))["rows"][0]
                self.assertEqual(("unknown_api", None), (row["detection"], row["rule_id"]))
        rules = diagnose.load_rules()
        self.assertEqual(["WIN-HEADER"], [r["id"] for r in rules["rules"] if r.get("case_insensitive")])

    def test_every_rule_and_alias_is_exercised(self):
        rules = diagnose.load_rules()
        ids = set()
        aliases = set()
        for rule in rules["rules"]:
            self.assertNotIn(rule["id"], ids)
            ids.add(rule["id"])
            for alias in rule["aliases"]:
                self.assertNotIn(alias, aliases)
                aliases.add(alias)
                rows = run(report(alias))["rows"]
                self.assertTrue(any(r["rule_id"] == rule["id"] and alias in r["original_apis"] for r in rows), alias)
            self.assertTrue(rule["reason"])
            self.assertTrue(rule["gaps"])

    def test_references_and_alternatives_exist_in_checked_in_sdk(self):
        for rule in diagnose.load_rules()["rules"]:
            for reference in rule["references"]:
                self.assertTrue((ROOT / reference).is_file(), reference)
                self.assertTrue(reference.startswith("sdk/"))
            for alternative in rule["alternatives"]:
                self.assertIn(alternative["reference"], rule["references"])
                text = (ROOT / alternative["reference"]).read_text()
                symbol = alternative["symbol"]
                if symbol == "AppStateBackend":
                    self.assertIn("pub trait AppStateBackend", text)
                elif symbol == "Application::presentation":
                    self.assertIn("impl Application", text)
                    self.assertIn("pub const fn presentation", text)
                else:
                    self.fail("Unverified SDK candidate: " + symbol)
                self.assertTrue(alternative["limits"])

    def test_contract_candidates_never_claim_runtime_equivalence(self):
        for evidence in ("RegOpenKeyExW(args)", "CreateWindowExW(args)", "System.Windows.Window"):
            row = run(report(evidence))["rows"][0]
            self.assertEqual("contract_only", row["availability"])
            self.assertEqual("PROVISIONAL", row["approval"])
            self.assertTrue(row["alternatives"][0]["limits"])
        self.assertEqual("dotnet", run(report("System.Windows.Window"))["rows"][0]["family"])

    def test_comments_literals_and_raw_strings_are_not_api_matches(self):
        snippets = ["// CreateFileW()\r\n", "/* ReadFile() */", '"CreateFileW"',
                    "L\"ReadFile()\"", "'CloseHandle'", 'u8"CreateFileW"',
                    'R"tag(CreateFileW("x")\nReadFile())tag"',
                    '@"ReadFile ""CreateFileW"""', '$@"CreateFileW {ReadFile()}"',
                    '@$"ReadFile {CreateFileW()}"', '$"CreateFileW({ReadFile()})"',
                    '"""ReadFile("x")\nCreateFileW()"""',
                    '$$"""{{CreateFileW()}}"""', '"ReadFile(\\\"x\\\")"',
                    '$"{Helper("CreateFileW")}"', '$@"{Helper("ReadFile")}"',
                    '@$"{Helper("CloseHandle")}"', '$"{Helper($"{Call("CreateFileW")}")}"',
                    "/* CreateFileW unclosed", '"ReadFile unclosed',
                    "// continued comment \\\nCreateFileW()",
                    "// continued comment \\\r\nReadFile()"]
        for evidence in snippets:
            with self.subTest(evidence=evidence):
                rows = run(report(evidence))["rows"]
                self.assertTrue(all(r["rule_id"] is None for r in rows), rows)
                self.assertTrue(all(r["detection"] == "no_code_evidence" for r in rows))

    def test_real_code_after_comment_or_string_still_detected(self):
        for evidence in ('"CreateFileW"; ReadFile(h);', '/* CreateFileW */ ReadFile(h);',
                         '$"{Helper("CreateFileW")}"; ReadFile(h);',
                         '// CreateFileW\r\nReadFile(h);'):
            rows = run(report(evidence))["rows"]
            self.assertEqual(["WIN-FILE-READ"], [r["rule_id"] for r in rows])

    def test_comment_and_literal_cannot_bridge_qualified_alias(self):
        for evidence in ('File /* ReadFile */ . ReadAllText(path)',
                         'File "ignored" . ReadAllText(path)'):
            self.assertIsNone(run(report(evidence))["rows"][0]["rule_id"])

    def test_result_records_do_not_share_mutable_source_or_rule_data(self):
        rules = diagnose.load_rules()
        result = run(report("CreateFileW; ReadFile;"), rules=rules)
        result["rows"][0]["source"]["path"] = "changed.c"
        result["rows"][0]["gaps"].append("changed")
        self.assertEqual("sample.c", result["rows"][1]["source"]["path"])
        self.assertNotIn("changed", diagnose.json_report(rules))

    def test_identifier_boundaries_and_case(self):
        for evidence in ("MyCreateFileW", "CreateFileW2", "createfilew", "前CreateFileW", "CreateFileW後"):
            self.assertIsNone(run(report(evidence))["rows"][0]["rule_id"])
        self.assertEqual("WIN-FILE-OPEN", run(report("int CreateFileW = 0;"))["rows"][0]["rule_id"])
        self.assertEqual("lexical_candidate", run(report("int CreateFileW = 0;"))["rows"][0]["detection"])

    def test_unknown_and_out_of_scope_do_not_disappear(self):
        unknown = run(report("UnmappedWindowsApi(h)"))["rows"][0]
        self.assertEqual("unknown_api", unknown["detection"])
        self.assertEqual("unknown", unknown["availability"])
        self.assertEqual([], unknown["alternatives"])
        self.assertIsNone(unknown["rule_id"])
        other = run(report("inotify_init1(0)", category="linux-api"))["rows"][0]
        self.assertEqual("not_classified", other["detection"])

    def test_unknown_status_and_review_approval_cannot_certify(self):
        output = run(report(review_status="approved"))
        self.assertEqual("approved", output["rows"][0]["source"]["review_status"])
        self.assertEqual("PROVISIONAL", output["rows"][0]["approval"])
        self.assertEqual("not_assessed", output["compatibility"])
        self.assertEqual("not_run", output["target_acceptance"])

    def test_raw_evidence_secrets_and_absolute_paths_not_reflected(self):
        evidence = 'ReadFile(h); // SECRET_TOKEN /private/source C:\\private\\file'
        output = run(report(evidence))
        serialized = diagnose.json_report(output)
        for text in ("SECRET_TOKEN", "/private/source", "private\\\\file"):
            self.assertNotIn(text, serialized)
        source = output["rows"][0]["source"]
        self.assertEqual(hashlib.sha256(evidence.encode()).hexdigest(), source["evidence_sha256"])

    def test_no_source_reads_or_execution_from_report(self):
        rules = diagnose.load_rules()
        document = report('ReadFile(h); // $(touch SHOULD_NOT_EXIST)')
        with mock.patch("pathlib.Path.open", side_effect=AssertionError("source read")), \
                mock.patch("subprocess.Popen", side_effect=AssertionError("execution")), \
                mock.patch("os.system", side_effect=AssertionError("execution")):
            self.assertEqual("WIN-FILE-READ", run(document, rules=rules)["rows"][0]["rule_id"])

    def test_deterministic_order_duplicate_preservation_and_input_immutability(self):
        document = report("ReadFile(h); CloseHandle(h);", line=8)
        document["findings"] += report("CreateFileW", line=2)["findings"] * 2
        rules = diagnose.load_rules()
        before = copy.deepcopy((document, rules))
        expected = diagnose.json_report(run(document, rules=rules))
        for seed in range(10):
            shuffled = copy.deepcopy(document)
            random.Random(seed).shuffle(shuffled["findings"])
            shuffled_rules = copy.deepcopy(rules)
            random.Random(seed).shuffle(shuffled_rules["rules"])
            self.assertEqual(expected, diagnose.json_report(run(shuffled, rules=shuffled_rules)))
        self.assertEqual(before, (document, rules))
        self.assertEqual(4, len(json.loads(expected)["rows"]))

    def test_trusted_adapter_and_rules_are_replaceable(self):
        def replacement(data, *, allowed_paths):
            self.assertEqual(b"future-format", data)
            self.assertEqual(["sample.c"], allowed_paths)
            return (adapter.Finding("windows-api", "sample.c", 1, "ReadFile", "pending"),)
        output = diagnose.diagnose_report(b"future-format", allowed_paths=["sample.c"], adapter=replacement)
        self.assertEqual("caller_supplied_adapter", output["input_adapter"])
        self.assertEqual("WIN-FILE-READ", output["rows"][0]["rule_id"])
        rules = diagnose.load_rules()
        rules["rules"] = []
        self.assertEqual("unknown_api", run(report(), rules=rules)["rows"][0]["detection"])

    def test_empty_report_has_no_compatibility_claim(self):
        output = run(dict(schema_version=1, findings=[]))
        self.assertEqual([], output["rows"])
        self.assertEqual("not_assessed", output["compatibility"])


class CliTests(unittest.TestCase):
    def invoke(self, data, *args):
        return subprocess.run([sys.executable, str(Path(diagnose.__file__)), *args], input=data,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              env=dict(os.environ, PYTHONDONTWRITEBYTECODE="1"), timeout=10)

    def test_fixture_cli_matches_library_and_is_deterministic(self):
        data = (FIXTURES / "read_prefix.report-v1.json").read_bytes()
        expected = diagnose.json_report(diagnose.diagnose_report(data, allowed_paths=["read_prefix.c"]))
        for _ in range(2):
            result = self.invoke(data, "--path", "read_prefix.c")
            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual(expected, result.stdout.decode())
            self.assertEqual(b"", result.stderr)

    def test_cli_failure_codes_are_bounded_and_do_not_echo_input(self):
        for data, code in ((b'{"SECRET":', "invalid_report_json"),
                           (encode(report(path="/private/SECRET")), "invalid_relative_path"),
                           (encode(report(path="other.c")), "finding_outside_selected_paths"),
                           (b" " * (adapter.MAX_REPORT_BYTES + 1), "report_byte_limit")):
            result = self.invoke(data, "--path", "sample.c")
            self.assertEqual(2, result.returncode)
            self.assertEqual(b"", result.stdout)
            self.assertEqual(code + "\n", result.stderr.decode())


if __name__ == "__main__":
    unittest.main()
