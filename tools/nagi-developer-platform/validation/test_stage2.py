import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import stage2
import validate

REPO = validate.REPO
FIXTURE = REPO / "tests/developer-platform/evaluation/portable"
RELATIVE = "tests/developer-platform/evaluation/portable"
HAVE_CLANG = shutil.which("clang") is not None
needs_clang = unittest.skipUnless(HAVE_CLANG, "clang is required for Stage 2 real-compiler tests")


def listing(directory):
    return sorted((p.name, p.read_bytes()) for p in Path(directory).iterdir())


class Copy(unittest.TestCase):
    """A temporary checkout-shaped tree holding a modifiable copy of the fixture."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="ndp-stage2-test-")
        self.addCleanup(self._tmp.cleanup)
        self.base = Path(os.path.realpath(self._tmp.name))
        self.fixture = self.base / RELATIVE
        self.fixture.mkdir(parents=True)
        self.approval_key = ("portable-summary", Path(RELATIVE).parts)
        approved = stage2._APPROVED_FIXTURE_SHA256[self.approval_key]
        for name, digest in approved.items():
            data = validate.read_manifest(REPO, Path(RELATIVE) / name)
            self.assertEqual(hashlib.sha256(data).hexdigest(), digest, name)
            (self.fixture / name).write_bytes(data)

    def run2(self, **kwargs):
        kwargs.setdefault("base", self.base)
        kwargs.setdefault("cwd", self.base)
        return stage2.run_stage2("portable-summary", **kwargs)

    def run2_with_test_edits(self, **kwargs):
        """Approve edits made by a test to the private, verified copy from setUp."""
        names = stage2._APPROVED_FIXTURE_SHA256[self.approval_key]
        digests = {name: hashlib.sha256((self.fixture / name).read_bytes()).hexdigest()
                   for name in names}
        with patch.dict(stage2._APPROVED_FIXTURE_SHA256, {self.approval_key: digests}):
            return self.run2(**kwargs)

    def edit(self, name, old, new):
        path = self.fixture / name
        text = path.read_text()
        self.assertIn(old, text)
        path.write_text(text.replace(old, new))

    def script(self, body):
        path = self.base / "fake-clang"
        path.write_text("#!/bin/sh\n" + body + "\n")
        path.chmod(0o755)
        return str(path)


@needs_clang
class RealClangTests(Copy):
    def test_real_fixture_builds_and_passes_without_touching_it(self):
        before = listing(FIXTURE)
        result = stage2.run_stage2("portable-summary", cwd=REPO)
        self.assertEqual((result["status"], result["reason"]), ("passed", "build_and_test_passed"))
        self.assertEqual([(s["name"], s["status"], s["exit_code"]) for s in result["steps"]],
                         [("compile", "passed", 0), ("run_test", "passed", 0)])
        self.assertEqual(result["scope"], "host_build_test_fixture_only")
        self.assertEqual(listing(FIXTURE), before)
        text = json.dumps(result)
        self.assertNotIn(str(REPO), text)
        self.assertNotIn("nagi-ndp-stage2-", text)
        self.assertNotIn("PASS portable", text)  # raw output is never copied

    def test_cli_success_and_exit_value(self):
        done = subprocess.run([sys.executable, "-B", str(Path(stage2.__file__)),
                               "--fixture-dir", RELATIVE],
                              cwd=REPO, capture_output=True, timeout=120)
        self.assertEqual((done.returncode, done.stderr), (0, b""))
        self.assertEqual(json.loads(done.stdout)["status"], "passed")

    def test_fixed_vector_and_allowlisted_environment(self):
        seen = []
        real = validate._run

        def spy(command, cwd, timeout, cancel=None, output_limit=0, env=None):
            seen.append((list(command), env))
            return real(command, cwd, timeout, cancel, env=env)

        with patch.dict(os.environ, {"CPATH": "/evil", "CCC_OVERRIDE_OPTIONS": "+-DEVIL",
                                     "SECRET_TOKEN": "secret-marker"}), \
                patch.object(validate, "_run", side_effect=spy):
            self.assertEqual(self.run2()["status"], "passed")
        compile_command, compile_env = seen[0]
        self.assertEqual(compile_command[1:], ["-std=c11", "-Wall", "-Wextra", "-Werror", "-O0",
                                               "-fno-color-diagnostics", "-o", "summary_test",
                                               "summary.c", "summary_test.c"])
        self.assertTrue(Path(compile_command[0]).name.startswith("clang"))
        for _, env in seen:
            self.assertNotIn("CPATH", env)
            self.assertNotIn("CCC_OVERRIDE_OPTIONS", env)
            self.assertNotIn("SECRET_TOKEN", env)
        self.assertEqual(len(seen[1][0]), 1)  # the test binary runs with no arguments

    def test_compile_uses_the_snapshot_even_if_the_source_changes_after_read(self):
        source = self.fixture / "summary.c"
        original = source.read_bytes()
        modified = original + b"\n#error changed after snapshot\n"
        read = validate.read_manifest

        def read_then_change(root, relative):
            data = read(root, relative)
            if Path(root) / relative == source:
                source.write_bytes(modified)
            return data

        with patch.object(validate, "read_manifest", side_effect=read_then_change):
            result = self.run2()
        self.assertEqual(source.read_bytes(), modified)
        self.assertEqual((result["status"], result["reason"]), ("passed", "build_and_test_passed"))

    def test_compile_error_stops_before_test(self):
        self.edit("summary.c", "return 0;", "return 0 +;")
        result = self.run2_with_test_edits()
        self.assertEqual((result["status"], result["reason"]), ("failed", "compile_failed"))
        self.assertEqual(result["steps"][0]["exit_code"], 1)
        self.assertEqual(result["steps"][1]["status"], "not_run")
        self.assertNotIn(str(self.base), json.dumps(result))

    def test_warning_is_an_error(self):
        self.edit("summary.c", "struct task_summary result", "int unused; struct task_summary result")
        self.assertEqual(self.run2_with_test_edits()["reason"], "compile_failed")

    def test_failing_test_binary_is_failed_with_signal_exit_kept(self):
        self.edit("summary.c", "case 1: ++result.done;", "case 1: ++result.open;")
        result = self.run2_with_test_edits()
        self.assertEqual((result["status"], result["reason"]), ("failed", "test_failed"))
        self.assertNotEqual(result["steps"][1]["exit_code"], 0)
        self.assertIsNotNone(result["steps"][1]["exit_code"])

    def test_test_exit_zero_without_expected_output_cannot_pass(self):
        self.edit("summary_test.c", "puts(\"PASS portable summary", "puts(\"nothing portable summary")
        result = self.run2_with_test_edits()
        self.assertEqual((result["status"], result["reason"]), ("failed", "invalid_tool_output"))

    def test_test_timeout_kills_binary(self):
        self.edit("summary_test.c", "struct task_summary out = {7, 8, 9};",
                  "struct task_summary out = {7, 8, 9}; for (;;) {}")
        start = time.monotonic()
        result = self.run2_with_test_edits(test_timeout=0.3)
        self.assertEqual((result["status"], result["steps"][1]["reason"]), ("timeout", "timeout"))
        self.assertLess(time.monotonic() - start, 30)

    def test_excess_test_output_is_bounded(self):
        self.edit("summary_test.c", "struct task_summary out = {7, 8, 9};",
                  "struct task_summary out = {7, 8, 9}; for (;;) fputs(\"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n\", stdout);")
        result = self.run2_with_test_edits(test_timeout=5)
        self.assertEqual(result["status"], "output_limit")
        self.assertLessEqual(result["steps"][1]["captured_output_bytes"], validate.MAX_OUTPUT)

    def test_planted_build_scripts_and_unlisted_files_are_never_used(self):
        marker = self.base / "marker"
        for name in ("Makefile", "build.sh", "CMakeLists.txt", "evil.c"):
            (self.fixture / name).write_text(f"all:\n\ttouch {marker}\n#!/bin/sh\ntouch {marker}\n"
                                             "this is not C\n")
        (self.fixture / "build.sh").chmod(0o755)
        result = self.run2()
        self.assertEqual(result["status"], "passed")
        self.assertFalse(marker.exists())

    def test_symlinked_source_is_rejected_without_running(self):
        real_source = self.base / "outside.c"
        shutil.move(str(self.fixture / "summary.c"), str(real_source))
        (self.fixture / "summary.c").symlink_to(real_source)
        with patch.object(validate.subprocess, "Popen", side_effect=AssertionError("must not run")):
            result = self.run2()
        self.assertEqual((result["status"], result["reason"]), ("not_run", "input_unavailable"))

    def test_symlinked_fixture_directory_is_rejected_without_running(self):
        moved = self.base / "elsewhere"
        shutil.move(str(self.fixture), str(moved))
        self.fixture.symlink_to(moved, target_is_directory=True)
        with patch.object(validate.subprocess, "Popen", side_effect=AssertionError("must not run")):
            self.assertEqual(self.run2()["reason"], "input_not_allowed")

    def test_scratch_is_removed(self):
        before = {p for p in Path(tempfile.gettempdir()).glob("nagi-ndp-stage2-*")}
        self.assertEqual(self.run2()["status"], "passed")
        self.edit("summary.c", "return 0;", "return 0 +;")
        self.assertEqual(self.run2_with_test_edits()["status"], "failed")
        self.assertEqual({p for p in Path(tempfile.gettempdir()).glob("nagi-ndp-stage2-*")}, before)


class ProcessControlTests(Copy):
    """Fake compiler scripts exercise orchestration only; they are not the real tool."""

    def test_missing_compiler_is_unavailable(self):
        result = self.run2(clang="/not/a/clang")
        self.assertEqual((result["status"], result["reason"]), ("unavailable", "clang_unavailable"))

    def test_compile_timeout(self):
        start = time.monotonic()
        result = self.run2(clang=self.script("sleep 30"), compile_timeout=0.3)
        self.assertEqual((result["status"], result["reason"]), ("timeout", "timeout"))
        self.assertEqual(result["steps"][1]["status"], "not_run")
        self.assertLess(time.monotonic() - start, 10)

    def test_cancel_during_compile_and_before_start(self):
        event = threading.Event()
        timer = threading.Timer(0.3, event.set)
        timer.start()
        try:
            result = self.run2(clang=self.script("sleep 30"), cancel=event)
        finally:
            timer.cancel()
        self.assertEqual((result["status"], result["steps"][1]["status"]), ("cancelled", "not_run"))
        with patch.object(validate.subprocess, "Popen", side_effect=AssertionError("must not run")):
            self.assertEqual(self.run2(cancel=event)["status"], "cancelled")

    def test_compiler_success_without_binary_cannot_pass(self):
        result = self.run2(clang=self.script("exit 0"))
        self.assertEqual((result["status"], result["reason"]), ("failed", "binary_missing"))

    def test_excess_compiler_output_is_bounded(self):
        result = self.run2(clang=self.script("while :; do echo secret-marker-secret-marker; done"))
        self.assertEqual(result["status"], "output_limit")
        self.assertNotIn("secret-marker", json.dumps(result))

    def test_compiler_nonzero_exit_is_kept(self):
        result = self.run2(clang=self.script("echo diagnostic >&2; exit 5"))
        self.assertEqual((result["reason"], result["steps"][0]["exit_code"]), ("compile_failed", 5))
        # Raw compiler text is still never copied; only the additive summary field (whose
        # name contains the word) may appear, and it must be empty for text without locations.
        self.assertEqual(result["diagnostics"]["items"], [])
        self.assertNotIn("diagnostic", json.dumps({k: v for k, v in result.items() if k != "diagnostics"}))


class RejectionTests(Copy):
    def assertRejected(self, reason, *args, **kwargs):
        with patch.object(validate.subprocess, "Popen", side_effect=AssertionError("must not run")):
            kwargs.setdefault("base", self.base)
            kwargs.setdefault("cwd", self.base)
            result = stage2.run_stage2(*args, **kwargs)
        self.assertEqual((result["status"], result["reason"]), ("not_run", reason))
        self.assertEqual(result["steps"], [])
        return result

    def test_changed_regular_source_or_header_is_rejected_before_compile(self):
        for name in ("summary.c", "summary_test.c", "summary.h"):
            with self.subTest(name=name):
                path = self.fixture / name
                original = path.read_bytes()
                path.write_bytes(original + b"\n/* altered fixture */\n")
                try:
                    result = self.assertRejected("fixture_digest_mismatch", "portable-summary",
                                                 clang=sys.executable)
                    self.assertNotIn("diagnostics", result)
                finally:
                    path.write_bytes(original)

    def test_unknown_or_hostile_target_ids_are_not_run_or_echoed(self):
        for target in ("missing", "portable-summary; touch x", "$(touch x)", "../portable-summary",
                       "", None, 7, ["portable-summary"]):
            with self.subTest(target=target):
                result = self.assertRejected("unknown_target", target)
                self.assertNotIn("target", result)

    def test_input_directory_must_equal_approved_directory(self):
        other = self.base / "tests/developer-platform/validation"
        other.mkdir(parents=True)
        for directory in ("tests/developer-platform/validation", "tests", ".", "..", "/", str(self.fixture / ".."),
                          RELATIVE + "/../../validation", "missing", str(other)):
            with self.subTest(directory=directory):
                self.assertRejected("input_not_allowed", "portable-summary", fixture_dir=directory)
        # The approved directory is accepted by relative and absolute spelling.
        self.assertEqual(stage2.run_stage2("portable-summary", fixture_dir=RELATIVE, cwd=self.base,
                                           base=self.base, clang="/not/a/clang")["reason"],
                         "clang_unavailable")
        self.assertEqual(stage2.run_stage2("portable-summary", fixture_dir=str(self.fixture), cwd=self.base,
                                           base=self.base, clang="/not/a/clang")["reason"],
                         "clang_unavailable")

    def test_cwd_must_be_inside_the_checkout(self):
        with tempfile.TemporaryDirectory(prefix="ndp-outside-") as outside:
            for cwd in (outside, "/", self.base.parent, "/not/a/directory"):
                with self.subTest(cwd=cwd):
                    self.assertRejected("cwd_not_allowed", "portable-summary", cwd=cwd)
            self.assertRejected("cwd_not_allowed", "portable-summary", cwd=outside, fixture_dir=str(self.fixture))

    def test_prior_stage_must_have_passed(self):
        for prior in ({"stage": 1, "status": "failed"}, {"stage": 1, "status": "not_run"},
                      {"stage": 1, "status": "unavailable"}, {"stage": 1, "status": "timeout"},
                      {"stage": 1, "status": "cancelled"}, {"stage": 2, "status": "passed"},
                      {"status": "passed"}, "passed", [], [{"stage": 1, "status": "passed"},
                                                         {"stage": 1, "status": "failed"}]):
            with self.subTest(prior=prior):
                self.assertRejected("prior_stage_not_passed", "portable-summary", prior=prior)

    @needs_clang
    def test_prior_stage_passed_allows_the_run(self):
        result = self.run2(prior={"stage": 1, "status": "passed"})
        self.assertEqual(result["status"], "passed")

    def test_invalid_timeouts(self):
        for timeout in (0, -1, 61, float("nan"), float("inf"), None, "5"):
            for name in ("compile_timeout", "test_timeout"):
                with self.subTest(name=name, timeout=timeout):
                    self.assertRejected("invalid_timeout", "portable-summary", **{name: timeout})

    def test_stage3_is_never_connected_and_never_passes(self):
        with patch.object(validate.subprocess, "Popen", side_effect=AssertionError("must not run")), \
                patch.object(validate, "read_manifest", side_effect=AssertionError("must not read")):
            for args in ((), ("portable-summary",), ("anything", "/tmp", "x")):
                result = stage2.run_stage3(*args)
                self.assertEqual((result["status"], result["reason"]), ("not_run", "stage_not_connected"))
        done = subprocess.run([sys.executable, "-B", str(Path(stage2.__file__)), "--stage", "3"],
                              cwd=self.base, capture_output=True, timeout=10)
        self.assertEqual(done.returncode, 3)
        self.assertEqual(json.loads(done.stdout)["status"], "not_run")

    def test_cli_rejects_unknown_target_and_failed_stage1_without_running(self):
        script = str(Path(stage2.__file__))
        done = subprocess.run([sys.executable, "-B", script, "--target", f"x;touch {self.base}/marker"],
                              cwd=REPO, capture_output=True, timeout=10)
        self.assertEqual((done.returncode, json.loads(done.stdout)["reason"]), (3, "unknown_target"))
        self.assertFalse((self.base / "marker").exists())
        done = subprocess.run([sys.executable, "-B", script, "--stage1-root", str(self.base),
                               "--stage1-manifest", "missing.json"],
                              cwd=REPO, capture_output=True, timeout=10)
        result = json.loads(done.stdout)
        self.assertEqual((done.returncode, result["reason"]), (3, "prior_stage_not_passed"))
        self.assertEqual(result["prior"][0]["status"], "failed")
        self.assertNotIn(str(self.base), done.stdout.decode())


class ContractDataTests(unittest.TestCase):
    def test_shipped_contract_matches_the_code_and_the_fixture(self):
        document = json.loads(stage2.CONTRACT.read_bytes())
        stages = {entry["stage"]: entry for entry in document["stages"]}
        self.assertEqual((stages[1]["connected"], stages[2]["connected"], stages[3]["connected"]),
                         (True, True, False))
        self.assertFalse(stages[3]["may_pass"])
        self.assertEqual(stages[3]["results"], ["not_run"])
        self.assertEqual(document["status"], "PROVISIONAL")
        targets = stage2.load_targets()
        self.assertEqual(list(targets), ["portable-summary"])
        target = targets["portable-summary"]
        for name in (*target.sources, *target.headers):
            self.assertTrue((FIXTURE / name).is_file(), name)
        self.assertEqual(REPO.joinpath(*target.directory), FIXTURE)

    def entry(self, **changes):
        base = {"id": "t", "directory": RELATIVE, "sources": ["summary.c"], "headers": [],
                "flags": ["-Wall"], "output": "out", "expect_stdout_prefix": "PASS"}
        return {**base, **changes}

    def test_unsafe_contract_entries_are_refused(self):
        for changes in ({"flags": ["-Xclang"]}, {"flags": ["-include", "x.h"]}, {"flags": ["-I/tmp"]},
                        {"flags": ["@/tmp/args"]}, {"flags": ["-fplugin=x"]},
                        {"sources": ["../summary.c"]}, {"sources": ["/etc/passwd"]}, {"sources": ["a b.c"]},
                        {"sources": ["a;b.c"]}, {"sources": []}, {"output": "../out"}, {"output": "summary.c"},
                        {"directory": "tests/developer-platform/validation"},
                        {"directory": "tests/developer-platform/evaluation"},
                        {"directory": "tests/developer-platform/evaluation/../../.."},
                        {"directory": "/tests/developer-platform/evaluation/x"},
                        {"expect_stdout_prefix": ""}, {"id": "a/b"}):
            with self.subTest(changes=changes), self.assertRaises(stage2.ContractError):
                stage2.Target(self.entry(**changes))
        with self.assertRaises(stage2.ContractError):
            stage2.Target({"id": "t"})

    def test_broken_contract_file_runs_nothing(self):
        with tempfile.TemporaryDirectory(prefix="ndp-contract-") as directory:
            path = Path(directory) / "stages.json"
            for text in ("not json", "{}", json.dumps({"stage2_targets": [self.entry(flags=["-Xclang"])]})):
                path.write_text(text)
                with self.subTest(text=text[:20]), self.assertRaises(stage2.ContractError):
                    stage2.load_targets(path)
        with patch.object(stage2, "load_targets", side_effect=stage2.ContractError("contract_unreadable")), \
                patch.object(validate.subprocess, "Popen", side_effect=AssertionError("must not run")):
            result = stage2.run_stage2("portable-summary", cwd=REPO)
        self.assertEqual((result["status"], result["reason"]), ("not_run", "contract_unreadable"))


if __name__ == "__main__":
    unittest.main()
