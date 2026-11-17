"""Host tests of the Linux rules over the shared adapter (Python 3.9).

These classify hand-written report fragments only. They never compile or run
POSIX/Linux code, so a pass says nothing about Linux behavior or Nagi targets.
"""

import copy
import hashlib
import json
import os
from pathlib import Path
import random
import subprocess
import sys
import unittest
from unittest import mock

from common import pt01_report as adapter
from linux import diagnose


ROOT = Path(__file__).resolve().parents[4]
FIXTURES = ROOT / "tests/developer-platform/api/linux"
EVALUATION = ROOT / "tests/developer-platform/evaluation"


def report(evidence="inotify_init1(0)", **fields):
    row = dict(category="linux-api", path="sample.c", line=1,
               evidence=evidence, review_status="needs_review")
    row.update(fields)
    return {"schema_version": 1, "findings": [row]}


def encode(document):
    return json.dumps(document, ensure_ascii=True).encode("utf-8")


def run(document, **kwargs):
    return diagnose.diagnose_report(encode(document), allowed_paths=["sample.c"], **kwargs)


def rule_ids(evidence, **fields):
    return [r["rule_id"] for r in run(report(evidence, **fields))["rows"]]


class FixtureTests(unittest.TestCase):
    def test_handwritten_report_matches_existing_linux_source(self):
        fixture = FIXTURES / "watch_changes.report-v1.json"
        source = EVALUATION / "linux/watch_changes.c"
        before = {p: hashlib.sha256(p.read_bytes()).digest() for p in (fixture, source)}
        data = fixture.read_bytes()
        lines = source.read_text(encoding="utf-8").splitlines()
        for row in json.loads(data)["findings"]:
            self.assertEqual(lines[row["line"] - 1], row["evidence"])
        output = diagnose.diagnose_report(data, allowed_paths=["watch_changes.c"])
        self.assertEqual([(2, "LNX-INOTIFY", ["sys/inotify.h"]),
                          (3, "POSIX-HEADER-FD", ["unistd.h"]),
                          (5, "LNX-INOTIFY", ["IN_CLOEXEC", "IN_NONBLOCK", "inotify_init1"]),
                          (7, "LNX-INOTIFY", ["IN_MODIFY", "inotify_add_watch"]),
                          (8, "POSIX-FD-CLOSE", ["close"])],
                         [(r["source"]["line"], r["rule_id"], r["original_apis"]) for r in output["rows"]])
        self.assertTrue(all(r["availability"] == "unsupported" for r in output["rows"]))
        self.assertTrue(all(r["gaps"] and not r["alternatives"] for r in output["rows"]))
        self.assertTrue(all(r["detection"] == "lexical_candidate" for r in output["rows"]))
        for path, digest in before.items():
            self.assertEqual(digest, hashlib.sha256(path.read_bytes()).digest())

    def test_negative_control_matches_existing_source(self):
        data = (FIXTURES / "text_only.report-v1.json").read_bytes()
        lines = (EVALUATION / "controls/text_only.c").read_text().splitlines()
        for row in json.loads(data)["findings"]:
            self.assertEqual(lines[row["line"] - 1], row["evidence"])
        output = diagnose.diagnose_report(data, allowed_paths=["text_only.c"])
        self.assertEqual(2, len(output["rows"]))
        self.assertTrue(all(r["rule_id"] is None and not r["original_apis"] for r in output["rows"]))
        # Same state names as the Windows diagnosis: the comment has no code; `return`
        # on the string line is code without a matching rule, so it stays unknown.
        self.assertEqual(["no_code_evidence", "unknown_api"], [r["detection"] for r in output["rows"]])

    def test_mixed_platform_fixture(self):
        data = (FIXTURES / "mixed_platform.report-v1.json").read_bytes()
        output = diagnose.diagnose_report(data, allowed_paths=["mixed.cpp"])
        by_line = {r["source"]["line"]: r for r in output["rows"]}
        self.assertEqual(5, len(output["rows"]))
        # Windows-origin evidence is outside these rules and is not guessed at.
        self.assertEqual(("not_classified", None), (by_line[3]["detection"], by_line[3]["rule_id"]))
        self.assertEqual(("POSIX-FILE-OPEN", ["global_scope_call"]),
                         (by_line[5]["rule_id"], by_line[5]["syntax_shapes"]))
        self.assertEqual(("unknown_api", None), (by_line[6]["detection"], by_line[6]["rule_id"]))
        self.assertEqual("LNX-QT", by_line[8]["rule_id"])
        self.assertEqual(["QtWidgets/QApplication"], by_line[8]["original_apis"])
        self.assertEqual(("unknown_api", None), (by_line[9]["detection"], by_line[9]["rule_id"]))


class RuleDataTests(unittest.TestCase):
    def test_bundled_rules_validate_and_are_unambiguous(self):
        rules = diagnose.validate_rules(diagnose.load_rules())
        for kind in ("headers", "calls", "identifiers"):
            seen = {}
            for rule in rules["rules"]:
                for name in rule[kind]:
                    self.assertNotIn(name, seen, (kind, name, rule["id"], seen.get(name)))
                    seen[name] = rule["id"]
        self.assertGreaterEqual(len(rules["rules"]), 15)

    def test_every_rule_and_alias_is_exercised(self):
        for rule in diagnose.load_rules()["rules"]:
            self.assertTrue(rule["reason"] and rule["gaps"])
            for header in rule["headers"]:
                with self.subTest(rule=rule["id"], header=header):
                    rows = run(report("#include <%s>" % header, category="posix-header"))["rows"]
                    self.assertEqual([rule["id"]], [r["rule_id"] for r in rows])
                    self.assertEqual([header], rows[0]["original_apis"])
            for prefix in rule["header_prefixes"]:
                rows = run(report("#include <%sexample.h>" % prefix))["rows"]
                self.assertEqual([rule["id"]], [r["rule_id"] for r in rows])
                self.assertEqual([prefix + "*"], rows[0]["original_apis"])
            for call in rule["calls"]:
                with self.subTest(rule=rule["id"], call=call):
                    rows = run(report("    %s(a, b);" % call))["rows"]
                    self.assertEqual([rule["id"]], [r["rule_id"] for r in rows])
                    self.assertEqual(([call], ["call"]), (rows[0]["original_apis"], rows[0]["syntax_shapes"]))
            for name in rule["identifiers"]:
                with self.subTest(rule=rule["id"], identifier=name):
                    rows = run(report("    x = %s;" % name))["rows"]
                    self.assertEqual([rule["id"]], [r["rule_id"] for r in rows])
                    self.assertEqual(["identifier"], rows[0]["syntax_shapes"])

    def test_references_and_alternatives_exist_in_checked_in_files(self):
        for rule in diagnose.load_rules()["rules"]:
            for reference in rule["references"]:
                self.assertTrue((ROOT / reference).is_file(), reference)
            for alternative in rule["alternatives"]:
                self.assertIn(alternative["reference"], rule["references"])
                text = (ROOT / alternative["reference"]).read_text()
                if alternative["symbol"] == "Application::presentation":
                    self.assertIn("impl Application", text)
                    self.assertIn("pub const fn presentation", text)
                else:
                    self.fail("Unverified SDK candidate: " + alternative["symbol"])
                self.assertTrue(alternative["limits"])

    def test_availability_never_claims_runtime_equivalence(self):
        for rule in diagnose.load_rules()["rules"]:
            self.assertIn(rule["availability"], ("unsupported", "contract_only", "unknown"))
            if rule["availability"] == "contract_only":
                self.assertEqual("toolkit_dependency", rule["classification"])
                self.assertTrue(all(a["relation"] == "presentation_identity_only" for a in rule["alternatives"]))
            else:
                self.assertEqual([], rule["alternatives"])
        for evidence in ("GtkWidget *w;", "QWidget w;"):
            row = run(report(evidence, category="gtk"))["rows"][0]
            self.assertEqual(("contract_only", "PROVISIONAL"), (row["availability"], row["approval"]))
            self.assertTrue(any("NDP-06" in g for g in row["gaps"]))

    def test_validate_rules_rejects_malformed_data(self):
        good = diagnose.load_rules()
        bad = []
        for mutate in (
                lambda r: r.pop("rules_version"),
                lambda r: r.update(rules="x"),
                lambda r: r["rules"][0].update(availability="supported"),
                lambda r: r["rules"][0].update(calls=["not valid"]),
                lambda r: r["rules"][0].update(headers=[], header_prefixes=[], calls=[], identifiers=[]),
                lambda r: r["rules"][1].update(id=r["rules"][0]["id"]),
                lambda r: r["rules"][0].update(identifiers=["a", "a"]),
                lambda r: r["rules"][0].update(reason="")):
            rules = copy.deepcopy(good)
            mutate(rules)
            bad.append(rules)
        for rules in bad:
            with self.assertRaisesRegex(ValueError, "^invalid_rules$"):
                diagnose.validate_rules(rules)
        with self.assertRaisesRegex(ValueError, "^invalid_rules$"):
            run(report(), rules=bad[0])


class ClassificationTests(unittest.TestCase):
    def test_posix_and_linux_representatives(self):
        examples = [
            ("    pid_t p = fork();", "POSIX-PROCESS", "posix"),
            ("pthread_create(&t, NULL, run, arg);", "POSIX-THREAD", "posix"),
            ("n = read(fd, buffer, size);", "POSIX-FILE-READ", "posix"),
            ("void *m = mmap(0, n, PROT_READ, MAP_PRIVATE, fd, 0);", "POSIX-MEMORY", "posix"),
            ("int s = socket(AF_INET, SOCK_STREAM, 0);", "POSIX-SOCKET", "posix"),
            ("epoll_ctl(ep, EPOLL_CTL_ADD, fd, &ev);", "LNX-EPOLL", "linux"),
            ("struct epoll_event ev;", "LNX-EPOLL", "linux"),
            ("int fd = timerfd_create(CLOCK_MONOTONIC, 0);", "LNX-FD-EVENTS", "linux"),
            ("#include <linux/input.h>", "LNX-KERNEL-INTERFACE", "linux"),
            ("ioctl(fd, request, &arg);", "LNX-KERNEL-INTERFACE", "linux"),
            ("GtkWidget *button = gtk_button_new();", "LNX-GTK", "gtk"),
            ("#include <gtk/gtk.h>", "LNX-GTK", "gtk"),
            ("g_signal_connect(button, \"clicked\", G_CALLBACK(on_click), NULL);", "LNX-GTK", "gtk"),
            ("g_main_loop_run(loop);", "LNX-GLIB", "glib"),
            ("class Panel : public QWidget {", "LNX-QT", "qt"),
            ("    Q_OBJECT", "LNX-QT", "qt"),
            ("#include <QApplication>", "LNX-QT", "qt"),
        ]
        for evidence, expected, family in examples:
            with self.subTest(evidence=evidence):
                rows = run(report(evidence))["rows"]
                self.assertIn(expected, [r["rule_id"] for r in rows])
                row = [r for r in rows if r["rule_id"] == expected][0]
                self.assertEqual((family, "lexical_candidate"), (row["family"], row["detection"]))

    def test_one_fragment_can_match_several_rules_without_losing_any(self):
        rows = run(report("int fd = open(p, O_RDONLY); read(fd, b, n); close(fd);"))["rows"]
        self.assertEqual(["POSIX-FD-CLOSE", "POSIX-FILE-OPEN", "POSIX-FILE-READ"],
                         sorted(r["rule_id"] for r in rows))

    def test_known_names_match_regardless_of_category(self):
        for category in ("windows-api", "dotnet", "unrelated", "linux-api"):
            self.assertEqual(["LNX-INOTIFY"], rule_ids("inotify_init(0)", category=category))

    def test_unknown_candidates_stay_unknown(self):
        row = run(report("    kqueue_like_create(fd);"))["rows"][0]
        self.assertEqual(("unknown_api", None, "unknown", "unknown"),
                         (row["detection"], row["rule_id"], row["family"], row["availability"]))
        self.assertEqual([], row["original_apis"])
        other = run(report("    kqueue_like_create(fd);", category="windows-api"))["rows"][0]
        self.assertEqual("not_classified", other["detection"])
        self.assertEqual("no_code_evidence", run(report("// only text"))["rows"][0]["detection"])

    def test_same_name_user_functions_and_members_are_not_posix_calls(self):
        for evidence in ("file.open(path);", "stream->read(buffer);", "Reader::read(buffer);",
                         "ns::close(fd);", "std::vector<int>::close(x);", "obj->write (x);",
                         "open_file(path);", "reopen(fd);", "my_read(fd);", "read_all(fd);",
                         "int opened = 1;", "void (*read_cb)(int);", "open = 0;", "x = &read;"):
            with self.subTest(evidence=evidence):
                row = run(report(evidence, category="posix-api"))["rows"][0]
                self.assertEqual(("unknown_api", None), (row["detection"], row["rule_id"]))

    def test_global_scope_call_is_still_a_candidate(self):
        for evidence in ("::open(p, 0);", "int fd = ::open(p, 0);", "f(::close(fd));"):
            row = run(report(evidence))["rows"][0]
            self.assertEqual(("lexical_candidate", ["global_scope_call"]),
                             (row["detection"], row["syntax_shapes"]), evidence)

    def test_global_scope_call_after_expression_keyword(self):
        # A keyword before `::` is not a namespace; the global call must stay known.
        for evidence, rule in (("return ::open(p, 0);", "POSIX-FILE-OPEN"),
                               ("return ::close(fd);", "POSIX-FD-CLOSE"),
                               ("co_return ::read(fd, b, n);", "POSIX-FILE-READ"),
                               ("else ::close(fd);", "POSIX-FD-CLOSE"),
                               ("return(::close(fd));", "POSIX-FD-CLOSE")):
            with self.subTest(evidence=evidence):
                row = run(report(evidence, category="posix-api"))["rows"][0]
                self.assertEqual((rule, "lexical_candidate", ["global_scope_call"]),
                                 (row["rule_id"], row["detection"], row["syntax_shapes"]))
        # Real qualifiers before `::` still hide the unqualified C name.
        for evidence in ("return ns::close(fd);", "return Reader::read(b);", "ret ::close(fd);"):
            with self.subTest(evidence=evidence):
                row = run(report(evidence, category="posix-api"))["rows"][0]
                self.assertEqual(("unknown_api", None), (row["detection"], row["rule_id"]))

    def test_local_definitions_are_demoted_not_hidden(self):
        for evidence in ("static ssize_t read(int fd, void *b, size_t n) {", "int open(const char *p);",
                         "ssize_t write(int, const void *, size_t);"):
            row = run(report(evidence))["rows"][0]
            self.assertEqual(("declaration_candidate", ["declaration_or_definition"]),
                             (row["detection"], row["syntax_shapes"]), evidence)
            self.assertIsNotNone(row["rule_id"])
        for evidence in ("return read(fd, b, n);", "else close(fd);", "x = read(fd);", "(void)close(fd);",
                         "if (close(fd) < 0) {", "case 1: close(fd);"):
            self.assertEqual("lexical_candidate", run(report(evidence))["rows"][0]["detection"], evidence)
        mixed = run(report("int read(int fd); read(0);"))["rows"][0]
        self.assertEqual(("lexical_candidate", ["call", "declaration_or_definition"]),
                         (mixed["detection"], mixed["syntax_shapes"]))

    def test_comments_strings_and_raw_strings_are_not_api_matches(self):
        snippets = ["// inotify_init1()\r\n", "/* epoll_wait() */", '"epoll_wait"', "'e'",
                    'L"inotify_init"', 'u8"fork()"', 'R"x(fork() close(fd))x"', 'R"(open(p))"',
                    "// \\\ninotify_init1();", "/* unterminated epoll_wait()", '"unterminated fork()',
                    '#include "not closed', "x = 1'000; // fork()", 'printf("%s", "pthread_create(");']
        for snippet in snippets:
            with self.subTest(snippet=snippet):
                row = run(report(snippet))["rows"][0]
                self.assertIsNone(row["rule_id"])

    def test_comment_includes_and_includes_in_literals_are_ignored(self):
        for snippet in ('// #include <sys/inotify.h>', '/* #include <sys/epoll.h> */',
                        '"#include <sys/epoll.h>"'):
            self.assertIsNone(run(report(snippet))["rows"][0]["rule_id"], snippet)
        for snippet in ('#include <sys/epoll.h>', '  #  include <sys/epoll.h>',
                        '#include "sys/epoll.h"', '#include <sys/epoll.h> // note',
                        '/* c */ #include <sys/epoll.h>'):
            self.assertEqual(["LNX-EPOLL"], rule_ids(snippet), snippet)

    def test_comments_inside_include_directive_are_whitespace(self):
        for snippet in ('#include /* comment */ <sys/epoll.h>', '#/* c */include <sys/epoll.h>',
                        '# /* a */ include /* b */ <sys/epoll.h>', '#include/**/"sys/epoll.h"',
                        '#include /* a\n b */ <sys/epoll.h>', '#include /* ** */ <sys/epoll.h>'):
            with self.subTest(snippet=snippet):
                row = run(report(snippet))["rows"][0]
                self.assertEqual(("LNX-EPOLL", ["include"], ["sys/epoll.h"]),
                                 (row["rule_id"], row["syntax_shapes"], row["original_apis"]))
        # Includes inside literals/comments stay ignored, and a gap comment never
        # stretches across code to fabricate an include.
        for snippet in ('"#include /* c */ <sys/epoll.h>"', '// #include /* c */ <sys/epoll.h>',
                        "R\"(#include /* c */ <sys/epoll.h>)\"", '#include /* unterminated <sys/epoll.h>',
                        '# /* a */ define X /* b */ include <sys/epoll.h>'):
            with self.subTest(snippet=snippet):
                self.assertIsNone(run(report(snippet))["rows"][0]["rule_id"])

    def test_real_code_after_comment_or_string_still_detected(self):
        for snippet in ("/* x */ fork();", 'puts("a"); fork();', "// c\nfork();", "'x'; fork();",
                        "fork /* gap */ ();", "\\\nfork();"):
            self.assertEqual(["POSIX-PROCESS"], rule_ids(snippet), snippet)

    def test_identifier_boundaries_and_case(self):
        for evidence in ("Open(p)", "READ(fd)", "xread(fd)", "read2(fd)", "read_(fd)", "gtk_init2(0)",
                         "GTKWidget w;", "myGtkWidget w;", "QWidgets w;", "XQObject o;", "IN_MODIFYX"):
            self.assertIsNone(run(report(evidence))["rows"][0]["rule_id"], evidence)
        self.assertEqual(["POSIX-FILE-OPEN"], rule_ids("open ( p ) ;"))

    def test_ambiguous_names_are_deliberately_not_rules(self):
        # Qt signal connect(), fopen and C library names are not POSIX descriptor rules.
        for evidence in ("connect(button, SIGNAL(clicked()), this, SLOT(go()));", "fopen(p, \"r\")",
                         "send(msg);", "signal(SIGINT, handler);"):
            self.assertIsNone(run(report(evidence))["rows"][0]["rule_id"], evidence)

    def test_dependency_name_in_non_code_fragments_is_not_matched(self):
        # Manifest/build lines use other syntax; those need PT-01 dependency data.
        for evidence in ("find_package(Qt6 REQUIRED COMPONENTS Widgets)",
                         "pkg_check_modules(GTK REQUIRED gtk4)"):
            row = run(report(evidence, category="qt"))["rows"][0]
            self.assertEqual(("unknown_api", None), (row["detection"], row["rule_id"]))


class SafetyTests(unittest.TestCase):
    def test_invalid_and_out_of_scope_reports_fail_closed(self):
        for data, code in ((b'{"SECRET":', "invalid_report_json"),
                           (encode(report(path="/private/SECRET")), "invalid_relative_path"),
                           (encode(report(path="other.c")), "finding_outside_selected_paths"),
                           (encode(dict(schema_version=2, findings=[])), "unsupported_report_version"),
                           (encode(report(line=0)), "invalid_finding_line"),
                           (encode(report(evidence="  ")), "invalid_finding_evidence"),
                           (encode(report(evidence="fork\x00()")), "invalid_finding_evidence"),
                           (b'{"schema_version":1,"findings":[],"findings":[]}', "duplicate_json_key"),
                           (b"\xff\xfe", "invalid_report_json"),
                           (" ".encode() * (adapter.MAX_REPORT_BYTES + 1), "report_byte_limit")):
            with self.subTest(code=code), self.assertRaisesRegex(adapter.ReportError, "^" + code + "$"):
                diagnose.diagnose_report(data, allowed_paths=["sample.c"])
        with self.assertRaisesRegex(adapter.ReportError, "explicit_path_allowlist_required"):
            diagnose.diagnose_report(encode(report()), allowed_paths=None)

    def test_oversized_but_valid_evidence_is_bounded(self):
        evidence = "fork();" + " " * (adapter.MAX_EVIDENCE_CHARS - 7)
        self.assertEqual(["POSIX-PROCESS"], rule_ids(evidence))
        with self.assertRaisesRegex(adapter.ReportError, "invalid_finding_evidence"):
            run(report(evidence + " "))
        # Pathological but in-budget evidence must finish quickly and not match.
        for text in ("/*" * 2000, '"' * 4000, "'" * 4000, "R\"" * 2000, "\\\n" * 1500,
                     "(" * 4000, "a::" * 1300, "#include <" * 400):
            with self.subTest(length=len(text)):
                rows = run(report(text))["rows"]
                self.assertEqual([None], [r["rule_id"] for r in rows])

    def test_broken_strings_do_not_crash(self):
        for evidence in ("� fork();", "fork　();", "あ = open(p);", "\U0001f600 close(fd);",
                         "ｆｏｒｋ();"):
            with self.subTest(evidence=evidence):
                self.assertEqual(1, len(run(report(evidence))["rows"]))
        self.assertEqual(["POSIX-PROCESS"], rule_ids("あ = fork();"))

    def test_result_does_not_reflect_evidence_secrets_or_absolute_paths(self):
        secrets = ("SECRET_TOKEN_9", "/Users/someone/private", "unknown_secret_function")
        evidence = "unknown_secret_function(); // SECRET_TOKEN_9 /Users/someone/private"
        output = run(report(evidence))
        serialized = diagnose.json_report(output)
        for text in secrets:
            self.assertNotIn(text, serialized)
        self.assertEqual(hashlib.sha256(evidence.encode()).hexdigest(),
                         output["rows"][0]["source"]["evidence_sha256"])

    def test_no_source_reads_or_execution_from_report(self):
        document = report("fork(); // $(touch SHOULD_NOT_EXIST)")
        rules = diagnose.load_rules()  # bundled data is read before the guards apply
        with mock.patch("pathlib.Path.open", side_effect=AssertionError("source read")), \
                mock.patch("subprocess.Popen", side_effect=AssertionError("execution")), \
                mock.patch("os.system", side_effect=AssertionError("execution")):
            self.assertEqual("POSIX-PROCESS", run(document, rules=rules)["rows"][0]["rule_id"])

    def test_review_status_and_authority_words_cannot_certify(self):
        for status in ("approved", "compatible", "pass", "owner_override"):
            row = run(report("fork();", review_status=status))["rows"][0]
            self.assertEqual(status, row["source"]["review_status"])
            self.assertEqual(("PROVISIONAL", "unsupported"), (row["approval"], row["availability"]))
        output = run(report("fork();"))
        self.assertEqual(("not_assessed", "not_run", "host_report_analysis"),
                         (output["compatibility"], output["target_acceptance"], output["scope"]))
        self.assertTrue(any("unchanged" in text for text in output["limitations"]))

    def test_empty_report_has_no_compatibility_claim(self):
        output = run(dict(schema_version=1, findings=[]))
        self.assertEqual([], output["rows"])
        self.assertEqual("not_assessed", output["compatibility"])


class DeterminismTests(unittest.TestCase):
    def test_order_duplicates_and_input_immutability(self):
        document = report("read(fd, b, n); close(fd);", line=8)
        document["findings"] += report("inotify_init1(0)", line=2)["findings"] * 2
        rules = diagnose.load_rules()
        before = copy.deepcopy((document, rules))
        expected = diagnose.json_report(run(document, rules=rules))
        for seed in range(10):
            shuffled = copy.deepcopy(document)
            random.Random(seed).shuffle(shuffled["findings"])
            shuffled_rules = copy.deepcopy(rules)
            random.Random(seed).shuffle(shuffled_rules["rules"])
            for rule in shuffled_rules["rules"]:
                random.Random(seed).shuffle(rule["calls"])
            self.assertEqual(expected, diagnose.json_report(run(shuffled, rules=shuffled_rules)))
        self.assertEqual(before, (document, rules))
        self.assertEqual(4, len(json.loads(expected)["rows"]))

    def test_repeated_runs_and_result_records_are_independent(self):
        data = (FIXTURES / "watch_changes.report-v1.json").read_bytes()
        first = diagnose.diagnose_report(data, allowed_paths=["watch_changes.c"])
        second = diagnose.diagnose_report(data, allowed_paths=["watch_changes.c"])
        self.assertEqual(diagnose.json_report(first), diagnose.json_report(second))
        first["rows"][0]["gaps"].append("mutated")
        first["rows"][0]["source"]["line"] = 99
        self.assertEqual(diagnose.json_report(second),
                         diagnose.json_report(diagnose.diagnose_report(data, allowed_paths=["watch_changes.c"])))

    def test_trusted_adapter_and_rules_are_replaceable(self):
        def replacement(data, *, allowed_paths):
            self.assertEqual(b"future-format", data)
            self.assertEqual(["sample.c"], allowed_paths)
            return (adapter.Finding("linux-api", "sample.c", 1, "epoll_wait(a)", "pending"),)
        output = diagnose.diagnose_report(b"future-format", allowed_paths=["sample.c"], adapter=replacement)
        self.assertEqual("caller_supplied_adapter", output["input_adapter"])
        self.assertEqual("LNX-EPOLL", output["rows"][0]["rule_id"])
        rules = diagnose.load_rules()
        rules["rules"] = []
        self.assertEqual("unknown_api", run(report(), rules=rules)["rows"][0]["detection"])
        rules = diagnose.load_rules()
        rules["rules"] = [r for r in rules["rules"] if r["id"] == "POSIX-PROCESS"]
        rules["rules"][0]["calls"].append("custom_spawn")
        self.assertEqual("POSIX-PROCESS", run(report("custom_spawn(a)"), rules=rules)["rows"][0]["rule_id"])


class CliTests(unittest.TestCase):
    def invoke(self, data, *args):
        return subprocess.run([sys.executable, str(Path(diagnose.__file__)), *args], input=data,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              env=dict(os.environ, PYTHONDONTWRITEBYTECODE="1"), timeout=10)

    def test_fixture_cli_matches_library_and_is_deterministic(self):
        data = (FIXTURES / "watch_changes.report-v1.json").read_bytes()
        expected = diagnose.json_report(diagnose.diagnose_report(data, allowed_paths=["watch_changes.c"]))
        for _ in range(2):
            result = self.invoke(data, "--path", "watch_changes.c")
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
