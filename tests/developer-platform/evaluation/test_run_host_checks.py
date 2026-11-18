"""Failure-path checks for the host evaluation orchestrator (subprocesses mocked)."""
import copy
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import run_host_checks as runner


class HostEvaluationTests(unittest.TestCase):
    def test_timeout_and_nonzero_cannot_pass(self):
        with patch.object(runner.subprocess, "run", side_effect=subprocess.TimeoutExpired("tool", 1)):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.run_command(["tool"], runner.ROOT, "portable_run")
            self.assertEqual((caught.exception.phase, caught.exception.code),
                             ("portable_run", "timeout"))
        result = subprocess.CompletedProcess(["tool"], 7, stdout=b"PASS", stderr=b"")
        with patch.object(runner.subprocess, "run", return_value=result):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.run_command(["tool"], runner.ROOT, "portable_build")
            self.assertEqual(caught.exception.code, "exit_7")

    def test_changed_stdout_cannot_pass(self):
        with patch.object(runner, "run_command", side_effect=[(b"one", 1), (b"two", 1)]):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.repeat_command(["tool"], runner.ROOT, "ui_analysis", 2)
            self.assertEqual(caught.exception.code, "nondeterministic_stdout")

    def test_invalid_repeat_count_never_starts_process(self):
        with patch.object(runner.subprocess, "run") as process:
            for count in (0, 1, 21):
                with self.subTest(count=count), self.assertRaises(runner.EvaluationError):
                    runner.evaluate(runner.ROOT, count)
            process.assert_not_called()

    def test_missing_compiler_never_starts_process(self):
        with patch.object(runner.shutil, "which", return_value=None), \
                patch.object(runner.subprocess, "run") as process:
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.evaluate(runner.ROOT, 2)
            self.assertEqual(caught.exception.code, "clang_unavailable")
            process.assert_not_called()

    def test_input_mutation_is_detected_even_after_command_failure(self):
        with tempfile.TemporaryDirectory(prefix="ndp-runner-integrity-") as directory:
            root = Path(directory)
            for path in runner.INPUTS:
                (root / path).parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(runner.ROOT / path, root / path)
            changed = root / runner.INPUTS[0]

            def fail_after_mutation(*args, **kwargs):
                changed.write_bytes(b"changed during failed command")
                raise runner.EvaluationError("portable_build", "exit_1")

            with patch.object(runner.shutil, "which", return_value="clang"), \
                    patch.object(runner, "run_command", side_effect=fail_after_mutation):
                with self.assertRaises(runner.EvaluationError) as caught:
                    runner.evaluate(root, 2)
            self.assertEqual((caught.exception.phase, caught.exception.code),
                             ("input_integrity", "source_changed"))
            self.assertEqual(changed.read_bytes(), b"changed during failed command")


class UIExpectationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Start with the actual CLI report, then inject stable semantic regressions.
        result = subprocess.run(
            [sys.executable, "-B", str(runner.ROOT / runner.ANALYZER), "--root",
             str(runner.ROOT / runner.FIXTURES / "ui"), "task_panel.ui"],
            capture_output=True, check=True, timeout=10,
        )
        cls.facts = json.loads(result.stdout)

    def assert_report_rejected(self, facts):
        output = json.dumps(facts).encode("utf-8")
        with patch.object(runner.shutil, "which", return_value="clang"), \
                patch.object(runner, "run_command", return_value=(b"clang version", 1)), \
                patch.object(runner, "repeat_command", side_effect=[(b"PASS", {}), (output, {})]):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.evaluate(runner.ROOT, 3)
        self.assertEqual((caught.exception.phase, caught.exception.code),
                         ("ui_analysis", "fixture_expectation_mismatch"))

    def test_stable_but_empty_ui_report_fails(self):
        facts = copy.deepcopy(self.facts)
        for key in ("widgets", "layouts", "tab_order", "connections"):
            facts[key] = []
        self.assert_report_rejected(facts)

    def test_correct_counts_with_wrong_identity_order_or_event_fail(self):
        for change in ("widget", "layout", "tab_order", "connection"):
            with self.subTest(change=change):
                facts = copy.deepcopy(self.facts)
                if change == "widget":
                    facts["widgets"][1]["name"] = "wrongName"
                elif change == "layout":
                    facts["layouts"][0]["class"] = "QGridLayout"
                elif change == "tab_order":
                    facts["tab_order"].reverse()
                else:
                    facts["connections"][0]["slot"] = "wrongSlot()"
                self.assert_report_rejected(facts)


class GTKExpectationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.reports = {}
        for name in ("task_panel", "gtk3_task_panel", "gtk4_task_panel"):
            result = subprocess.run(
                [sys.executable, "-B", str(runner.ROOT / runner.ANALYZER), "--root",
                 str(runner.ROOT / runner.FIXTURES / "ui"), name + ".ui"],
                capture_output=True, check=True, timeout=10,
            )
            cls.reports[name] = json.loads(result.stdout)

    def reject(self, phase, code, gtk3=None, gtk4=None):
        def encode(name, replacement):
            if replacement is None:
                replacement = self.reports[name]
            if isinstance(replacement, bytes):
                return replacement
            return json.dumps(replacement).encode("utf-8")

        outputs = [(b"PASS", {}), (encode("task_panel", None), {}),
                   (encode("gtk3_task_panel", gtk3), {}),
                   (encode("gtk4_task_panel", gtk4), {})]
        with patch.object(runner.shutil, "which", return_value="clang"), \
                patch.object(runner, "run_command", return_value=(b"clang version", 1)), \
                patch.object(runner, "repeat_command", side_effect=outputs):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.evaluate(runner.ROOT, 3)
        self.assertEqual((caught.exception.phase, caught.exception.code), (phase, code))

    def test_unmodified_real_reports_pass_the_mocked_orchestration(self):
        outputs = [(b"PASS", {}), *[
            (json.dumps(self.reports[name]).encode("utf-8"), {})
            for name in ("task_panel", "gtk3_task_panel", "gtk4_task_panel")]]
        with patch.object(runner.shutil, "which", return_value="clang"), \
                patch.object(runner, "run_command", return_value=(b"clang version", 1)), \
                patch.object(runner, "repeat_command", side_effect=outputs), \
                patch.object(runner, "check_stage2", return_value={}), \
                patch.object(runner, "check_api", return_value={}), \
                patch.object(runner, "check_generator", return_value={}), \
                patch.object(runner, "check_xaml", return_value={}), \
                patch.object(runner, "check_manifest_validation", return_value={}):
            result = runner.evaluate(runner.ROOT, 3)
        self.assertEqual(result["gtk3_ui_analysis"]["facts"]["unknown"], 1)
        self.assertEqual(result["gtk4_ui_analysis"]["facts"]["widgets"], 5)

    def test_stable_but_empty_gtk_reports_fail(self):
        for name, phase in (("gtk3_task_panel", "gtk3_ui_analysis"),
                            ("gtk4_task_panel", "gtk4_ui_analysis")):
            with self.subTest(name=name):
                facts = copy.deepcopy(self.reports[name])
                for key in ("widgets", "layouts", "connections", "bindings", "unknown",
                            "resources"):
                    facts[key] = []
                kwargs = {"gtk3" if name.startswith("gtk3") else "gtk4": facts}
                self.reject(phase, "fixture_expectation_mismatch", **kwargs)

    def test_malformed_gtk_report_fails_as_invalid(self):
        self.reject("gtk3_ui_analysis", "invalid_report", gtk3=b"not json")
        self.reject("gtk4_ui_analysis", "invalid_report", gtk4=b"{}")

    def test_correct_counts_with_wrong_gtk3_facts_fail(self):
        def widget(facts):
            facts["widgets"][3]["name"] = "wrongName"

        def parent(facts):
            facts["widgets"][3]["parent_widget"] = "taskWindow"

        def layout(facts):
            facts["layouts"][0]["class"] = "GtkGrid"

        def packing(facts):
            items = facts["layouts"][0]["items"]
            items[0]["packing"][1]["values"][0]["text"] = "9"

        def order(facts):
            facts["layouts"][0]["items"].reverse()

        def connection(facts):
            facts["connections"][0]["receiver"] = "body"

        def unknown(facts):
            facts["unknown"][0]["class"] = "GtkLabel"

        def binding(facts):
            facts["bindings"][0]["owner"] = "title"

        def requires(facts):
            facts["resources"][0]["attributes"]["version"] = "4.0"

        for change in (widget, parent, layout, packing, order, connection, unknown,
                       binding, requires):
            with self.subTest(change=change.__name__):
                facts = copy.deepcopy(self.reports["gtk3_task_panel"])
                change(facts)
                self.reject("gtk3_ui_analysis", "fixture_expectation_mismatch", gtk3=facts)

    def test_correct_counts_with_wrong_gtk4_facts_fail(self):
        def widget(facts):
            facts["widgets"][4]["class"] = "GtkLinkButton"

        def layout(facts):
            facts["layouts"][0]["class"] = "GtkBox"

        def position(facts):
            values = facts["layouts"][0]["items"][2]["objects"][0]["properties"][1]["values"]
            values[-1]["text"] = "1"

        def order(facts):
            facts["layouts"][0]["items"].reverse()

        def connection(facts):
            facts["connections"][0]["slot"] = "wrong_handler"

        def binding(facts):
            facts["bindings"][0]["tag"] = "property"

        def unknown(facts):
            facts["unknown"] = [{"class": "AtkObject", "kind": "widget", "name": "x"}]
            facts["widgets"].pop()

        for change in (widget, layout, position, order, connection, binding, unknown):
            with self.subTest(change=change.__name__):
                facts = copy.deepcopy(self.reports["gtk4_task_panel"])
                change(facts)
                self.reject("gtk4_ui_analysis", "fixture_expectation_mismatch", gtk4=facts)

    def test_gtk_command_failures_keep_their_phase(self):
        for phase in ("gtk3_ui_analysis", "gtk4_ui_analysis"):
            for code in ("nondeterministic_stdout", "exit_1", "timeout"):
                with self.subTest(phase=phase, code=code):
                    qt = json.dumps(self.reports["task_panel"]).encode("utf-8")
                    outputs = [(b"PASS", {}), (qt, {})]
                    if phase == "gtk4_ui_analysis":
                        outputs.append((json.dumps(self.reports["gtk3_task_panel"]).encode("utf-8"), {}))
                    outputs.append(runner.EvaluationError(phase, code))
                    with patch.object(runner.shutil, "which", return_value="clang"), \
                            patch.object(runner, "run_command", return_value=(b"clang version", 1)), \
                            patch.object(runner, "repeat_command", side_effect=outputs):
                        with self.assertRaises(runner.EvaluationError) as caught:
                            runner.evaluate(runner.ROOT, 3)
                    self.assertEqual((caught.exception.phase, caught.exception.code), (phase, code))

    def test_gtk_input_mutation_is_detected(self):
        gtk_inputs = [path for path in runner.INPUTS if "gtk" in path.name]
        self.assertEqual([path.name for path in gtk_inputs],
                         ["gtk3_task_panel.ui", "gtk4_task_panel.ui"])
        for gtk_input in gtk_inputs:
            with self.subTest(source=gtk_input.name), tempfile.TemporaryDirectory(
                    prefix="ndp-runner-gtk-integrity-") as directory:
                root = Path(directory)
                for path in runner.INPUTS:
                    (root / path).parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(runner.ROOT / path, root / path)
                changed = root / gtk_input

                def mutate_then_fail(*args, **kwargs):
                    changed.write_bytes(changed.read_bytes() + b"<!-- changed -->")
                    raise runner.EvaluationError("gtk_phase", "exit_1")

                with patch.object(runner.shutil, "which", return_value="clang"), \
                        patch.object(runner, "run_command", side_effect=mutate_then_fail):
                    with self.assertRaises(runner.EvaluationError) as caught:
                        runner.evaluate(root, 2)
                self.assertEqual((caught.exception.phase, caught.exception.code),
                                 ("input_integrity", "source_changed"))


def cli_output(command, stdin_path=None):
    """Actual stdout of a trusted tool, used as the base for injected regressions."""
    data = (runner.ROOT / stdin_path).read_bytes() if stdin_path else None
    return subprocess.run(
        [sys.executable, "-B", *command], cwd=runner.ROOT, input=data,
        capture_output=True, check=True, timeout=60).stdout


class RunCommandStdinTests(unittest.TestCase):
    def test_fixture_bytes_are_fed_without_a_shell(self):
        result = subprocess.CompletedProcess(["tool"], 0, stdout=b"{}", stderr=b"")
        report = runner.API_REPORTS / "windows/read_prefix.report-v1.json"
        with patch.object(runner.subprocess, "run", return_value=result) as process:
            runner.run_command(["tool"], runner.ROOT, "api", stdin_path=report)
        kwargs = process.call_args.kwargs
        self.assertEqual(kwargs["input"], (runner.ROOT / report).read_bytes())
        self.assertNotIn("shell", kwargs)

    def test_unreadable_stdin_fixture_cannot_pass(self):
        with patch.object(runner.subprocess, "run") as process:
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.run_command(["tool"], runner.ROOT, "api",
                                   stdin_path=Path("missing/report.json"))
        self.assertEqual((caught.exception.phase, caught.exception.code),
                         ("api", "command_unavailable"))
        process.assert_not_called()


class Stage2ExpectationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if runner.shutil.which("clang") is None:
            raise unittest.SkipTest("clang is not installed; real Stage 2 run skipped")
        cls.output = cli_output([str(runner.STAGE2), "--target", runner.STAGE2_TARGET,
                                 "--fixture-dir", str(runner.FIXTURES / "portable")])
        cls.report = json.loads(cls.output)

    def reject(self, facts, code="fixture_expectation_mismatch"):
        with patch.object(runner, "repeat_command",
                          return_value=(json.dumps(facts).encode("utf-8"), {})):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.check_stage2(runner.ROOT, 3)
        self.assertEqual((caught.exception.phase, caught.exception.code),
                         ("stage2_build_test", code))

    def test_real_stage2_passes_three_times_with_identical_stdout(self):
        info = runner.check_stage2(runner.ROOT, 3)
        self.assertEqual(len(info["samples_ms"]), 3)
        self.assertTrue(info["stdout_identical"])
        self.assertEqual(info["steps_passed"], ["compile", "run_test"])

    def test_real_report_is_accepted_by_the_validator(self):
        with patch.object(runner, "repeat_command", return_value=(self.output, {})):
            self.assertEqual(runner.check_stage2(runner.ROOT, 3)["stage"], 2)

    def test_stable_but_wrong_reports_fail(self):
        def status(facts):
            facts["status"] = "failed"

        def reason(facts):
            facts["reason"] = "test_failed"

        def target(facts):
            facts["target"] = "other"

        def scope(facts):
            facts["scope"] = "nagi_target"

        def no_steps(facts):
            facts["steps"] = []

        def only_compile(facts):
            facts["steps"] = facts["steps"][:1]

        def test_step_not_run(facts):
            facts["steps"][1].update(status="not_run", exit_code=None)

        def nonzero_exit(facts):
            facts["steps"][1]["exit_code"] = 1

        for change in (status, reason, target, scope, no_steps, only_compile,
                       test_step_not_run, nonzero_exit):
            with self.subTest(change=change.__name__):
                facts = copy.deepcopy(self.report)
                change(facts)
                self.reject(facts)

    def test_malformed_reports_fail_as_invalid(self):
        for output in (b"not json", b"[]", b'{"steps": 5}'):
            with self.subTest(output=output), patch.object(
                    runner, "repeat_command", return_value=(output, {})):
                with self.assertRaises(runner.EvaluationError) as caught:
                    runner.check_stage2(runner.ROOT, 3)
                self.assertIn(caught.exception.code, ("invalid_report",
                                                      "fixture_expectation_mismatch"))
                self.assertEqual(caught.exception.phase, "stage2_build_test")


class ApiExpectationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.outputs = {}
        for phase, tool, report, path, _ in runner.API_CASES:
            cls.outputs[phase] = cli_output(
                [str(runner.API_TOOLS / tool / "diagnose.py"), "--path", path],
                runner.API_REPORTS / report)

    def case(self, phase):
        return next(item for item in runner.API_CASES if item[0] == phase)

    def reject(self, phase, data, code="fixture_expectation_mismatch"):
        _, _, _, path, expected = self.case(phase)
        with self.assertRaises(runner.EvaluationError) as caught:
            runner.validate_api(phase, data, path, expected)
        self.assertEqual(caught.exception.code, code)

    def test_real_reports_pass_three_times_with_identical_stdout(self):
        results = runner.check_api(runner.ROOT, 3)
        self.assertEqual(sorted(results), sorted(case[0] for case in runner.API_CASES))
        for phase, info in results.items():
            with self.subTest(phase=phase):
                self.assertTrue(info["stdout_identical"])
                self.assertEqual(len(info["samples_ms"]), 3)
                self.assertEqual(info["rows"], len(self.case(phase)[4]))

    def test_every_case_covers_a_distinct_expectation(self):
        # Guard against a table whose rows were emptied to make a run pass.
        self.assertTrue(all(case[4] for case in runner.API_CASES))
        self.assertEqual(len({case[0] for case in runner.API_CASES}), len(runner.API_CASES))
        self.assertIn("lexical_candidate", {row[2] for case in runner.API_CASES
                                            for row in case[4]})

    def test_stable_but_wrong_rows_fail(self):
        def empty(data):
            data["rows"] = []

        def drop_last(data):
            data["rows"].pop()

        def line(data):
            data["rows"][0]["source"]["line"] += 1

        def rule(data):
            data["rows"][0]["rule_id"] = "WIN-FILE-READ"

        def detection(data):
            data["rows"][0]["detection"] = "unknown_api"

        def api(data):
            data["rows"][0]["original_apis"] = ["Other"]

        def reorder(data):
            data["rows"].reverse()

        for change in (empty, drop_last, line, rule, detection, api, reorder):
            with self.subTest(change=change.__name__):
                data = json.loads(self.outputs["api_windows_read_prefix"])
                change(data)
                self.reject("api_windows_read_prefix", data)

    def test_negative_control_must_not_claim_a_known_api(self):
        for phase in ("api_windows_text_only", "api_linux_text_only"):
            with self.subTest(phase=phase):
                data = json.loads(self.outputs[phase])
                data["rows"][1].update(rule_id="WIN-FILE-OPEN", detection="lexical_candidate",
                                       original_apis=["CreateFileW"])
                self.reject(phase, data)
                data = json.loads(self.outputs[phase])
                data["rows"][1]["classification"] = "file_io"
                self.reject(phase, data)

    def test_envelope_changes_fail(self):
        def compatibility(data):
            data["compatibility"] = "compatible"

        def target(data):
            data["target_acceptance"] = "passed"

        def scope(data):
            data["scope"] = "nagi_target"

        def version(data):
            data["diagnostic_version"] = 2

        def approval(data):
            data["rows"][0]["approval"] = "CONFIRMED"

        def path(data):
            data["rows"][0]["source"]["path"] = "other.c"

        def evidence(data):
            data["rows"][0]["source"]["evidence_scope"] = "verified_source"

        for change in (compatibility, target, scope, version, approval, path, evidence):
            with self.subTest(change=change.__name__):
                data = json.loads(self.outputs["api_linux_watch_changes"])
                change(data)
                self.reject("api_linux_watch_changes", data)

    def test_malformed_reports_fail_as_invalid(self):
        for output in (b"not json", b"[]", b"{}"):
            with self.subTest(output=output), patch.object(
                    runner, "repeat_command", return_value=(output, {})):
                with self.assertRaises(runner.EvaluationError) as caught:
                    runner.check_api(runner.ROOT, 3)
                self.assertEqual((caught.exception.phase, caught.exception.code),
                                 ("api_windows_read_prefix", "invalid_report"))

    def test_second_case_failure_keeps_its_own_phase(self):
        good = self.outputs["api_windows_read_prefix"]
        wrong = json.loads(self.outputs["api_windows_text_only"])
        wrong["rows"].pop()
        with patch.object(runner, "repeat_command", side_effect=[
                (good, {}), (json.dumps(wrong).encode("utf-8"), {})]):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.check_api(runner.ROOT, 3)
        self.assertEqual((caught.exception.phase, caught.exception.code),
                         ("api_windows_text_only", "fixture_expectation_mismatch"))


class GeneratorExpectationTests(unittest.TestCase):
    def run_with(self, mutate=None, outputs=None):
        """Run the real generator CLI; optionally alter stdout or the written tree."""
        real = runner.run_command
        calls = []

        def wrapper(command, root, phase, **kwargs):
            output, elapsed = real(command, root, phase, **kwargs)
            calls.append(command)
            target = Path(command[command.index("--root") + 1])
            if mutate is not None:
                output = mutate(len(calls), target, output)
            return output, elapsed

        with patch.object(runner, "run_command", side_effect=wrapper):
            return runner.check_generator(runner.ROOT, 3), calls

    def assert_fails(self, code, mutate):
        with self.assertRaises(runner.EvaluationError) as caught:
            self.run_with(mutate)
        self.assertEqual((caught.exception.phase, caught.exception.code),
                         ("generator_scaffold", code))

    def test_real_generator_passes_three_times_into_temporary_output_only(self):
        before = runner.fingerprint(runner.ROOT)
        info, calls = self.run_with()
        self.assertEqual(len(calls), 3)
        self.assertTrue(info["stdout_identical"] and info["tree_identical"])
        self.assertEqual(info["files"], runner.GENERATOR_FILES)
        # Every call targets a throwaway directory outside the checkout.
        for command in calls:
            root = Path(command[command.index("--root") + 1]).resolve()
            self.assertNotIn(runner.ROOT.resolve(), (root, *root.parents))
        self.assertEqual(runner.fingerprint(runner.ROOT), before)
        self.assertFalse((runner.ROOT / "samples/level-zero").exists())

    def test_changed_stdout_between_runs_fails(self):
        self.assert_fails("nondeterministic_output",
                          lambda n, target, out: out + b" " if n == 2 else out)

    def test_changed_tree_between_runs_fails(self):
        def change(n, target, out):
            if n == 3:
                (target / "samples/level-zero/README.md").write_bytes(b"different")
            return out
        self.assert_fails("nondeterministic_output", change)

    def test_extra_file_outside_output_directory_fails(self):
        def stray(n, target, out):
            (target / "stray.txt").write_text("x", encoding="utf-8")
            return out
        self.assert_fails("fixture_expectation_mismatch", stray)

    def test_missing_or_extra_generated_file_fails(self):
        def missing(n, target, out):
            (target / "samples/level-zero/UNSUPPORTED_APIS.md").unlink()
            return out

        def extra(n, target, out):
            (target / "samples/level-zero/extra.txt").write_text("x", encoding="utf-8")
            return out
        self.assert_fails("fixture_expectation_mismatch", missing)
        self.assert_fails("fixture_expectation_mismatch", extra)

    def test_wrong_manifest_content_fails(self):
        for change in (
                lambda m: m.update(id="com.example.other"),
                lambda m: m.update(requestedCapabilities=["files.read"]),
                lambda m: m.update(origin="first-party"),
                lambda m: m["displayName"].update({"ja-JP": "?"})):
            with self.subTest(change=change):
                def mutate(n, target, out, change=change):
                    path = target / "samples/level-zero/manifest.json"
                    manifest = json.loads(path.read_text(encoding="utf-8"))
                    change(manifest)
                    path.write_text(json.dumps(manifest, ensure_ascii=False), encoding="utf-8")
                    return out
                self.assert_fails("fixture_expectation_mismatch", mutate)

    def test_wrong_status_report_fails(self):
        for change in (lambda d: d.update(status="planned"), lambda d: d.update(built=True),
                       lambda d: d.update(validated=True),
                       lambda d: d.update(output_dir="samples/other"),
                       lambda d: d.update(created=[])):
            with self.subTest(change=change):
                def mutate(n, target, out, change=change):
                    data = json.loads(out)
                    change(data)
                    return json.dumps(data).encode("utf-8")
                self.assert_fails("fixture_expectation_mismatch", mutate)

    def test_malformed_stdout_fails_as_invalid(self):
        self.assert_fails("invalid_report", lambda n, target, out: b"not json")
        self.assert_fails("invalid_report", lambda n, target, out: b"[]")

    def test_timeout_and_nonzero_exit_keep_the_phase(self):
        with patch.object(runner.subprocess, "run",
                          side_effect=subprocess.TimeoutExpired("tool", 1)):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.check_generator(runner.ROOT, 2)
        self.assertEqual((caught.exception.phase, caught.exception.code),
                         ("generator_scaffold", "timeout"))
        result = subprocess.CompletedProcess(["tool"], 1, stdout=b"{}", stderr=b"")
        with patch.object(runner.subprocess, "run", return_value=result):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.check_generator(runner.ROOT, 2)
        self.assertEqual((caught.exception.phase, caught.exception.code),
                         ("generator_scaffold", "exit_1"))


class XamlExpectationTests(unittest.TestCase):
    SOURCES = ("xaml_minimal.xaml", "wpf_task_panel.xaml", "winui_task_panel.xaml")

    @classmethod
    def setUpClass(cls):
        cls.reports = {}
        for name in cls.SOURCES:
            cls.reports[name] = json.loads(cli_output(
                [str(runner.ANALYZER), "--root", str(runner.FIXTURES / "ui"), name]))

    def run_with(self, replacements=None, **kwargs):
        """check_xaml over real reports; `replacements` maps a source to other stdout."""
        outputs = []
        for name in self.SOURCES:
            data = (replacements or {}).get(name, self.reports[name])
            outputs.append((data if isinstance(data, bytes) else json.dumps(data).encode("utf-8"),
                            {}))
        with patch.object(runner, "repeat_command", side_effect=kwargs.get("side", outputs)):
            return runner.check_xaml(runner.ROOT, 3)

    def assert_rejected(self, name, facts, code="fixture_expectation_mismatch"):
        phase = {s: p for p, s, _ in runner.XAML_CASES}[name]
        with self.assertRaises(runner.EvaluationError) as caught:
            self.run_with({name: facts})
        self.assertEqual((caught.exception.phase, caught.exception.code), (phase, code))

    def test_real_xaml_reports_pass_three_times_with_identical_stdout(self):
        before = runner.fingerprint(runner.ROOT)
        result = runner.check_xaml(runner.ROOT, 3)
        self.assertEqual(sorted(result), sorted(p for p, _, _ in runner.XAML_CASES))
        for info in result.values():
            self.assertTrue(info["stdout_identical"])
            self.assertEqual(len(info["samples_ms"]), 3)
        self.assertEqual(result["wpf_xaml_analysis"]["facts"]["widgets"], 7)
        self.assertEqual(result["winui_xaml_analysis"]["facts"]["unresolved_references"], 1)
        self.assertEqual(runner.fingerprint(runner.ROOT), before)

    def test_unmodified_reports_pass_the_mocked_orchestration(self):
        self.assertEqual(len(self.run_with()), 3)

    def test_stable_but_empty_reports_fail(self):
        for name in self.SOURCES:
            with self.subTest(name=name):
                facts = copy.deepcopy(self.reports[name])
                for key in ("widgets", "layouts", "tab_order", "connections", "bindings",
                            "unknown", "unresolved_references", "custom_widgets", "resources"):
                    facts[key] = []
                self.assert_rejected(name, facts)

    def test_wrong_identity_structure_or_event_fails(self):
        changes = {
            "wpf_task_panel.xaml": (
                lambda f: f["widgets"][4].update(name="wrongName"),
                lambda f: f["widgets"][5].update(parent_widget="grid"),
                lambda f: f["layouts"][0].update({"class": "StackPanel"}),
                lambda f: f["layouts"][0]["items"][1]["attributes"].update({"Grid.Row": "5"}),
                lambda f: f["tab_order"].reverse(),
                lambda f: f["connections"][0].update(slot="Other_Click"),
                lambda f: f["connections"][0].update(execution="run"),
                lambda f: f["bindings"][0].update(execution="run"),
                lambda f: f["bindings"][1].update(tag="x:Bind"),
                lambda f: f["unknown"].clear(),
                lambda f: f["unresolved_references"].append({"kind": "resource_key", "name": "k"}),
                lambda f: f.update(form_class="Synthetic.Other"),
                lambda f: f.update(format="gtk-builder-ui"),
                lambda f: f.update(source="other.xaml")),
            "winui_task_panel.xaml": (
                lambda f: f["unresolved_references"].clear(),
                lambda f: f["unknown"].pop(),
                lambda f: f["connections"].reverse(),
                lambda f: f["bindings"][1].update(property="IsEnabled"),
                lambda f: f["widgets"][1].update(parent_widget="panel")),
            "xaml_minimal.xaml": (
                lambda f: f["widgets"][0].update(**{"class": "TextBox"}),
                lambda f: f["widgets"][0].update(name="other")),
        }
        for name, items in changes.items():
            for index, change in enumerate(items):
                with self.subTest(name=name, change=index):
                    facts = copy.deepcopy(self.reports[name])
                    change(facts)
                    self.assert_rejected(name, facts)

    def test_malformed_reports_fail_as_invalid(self):
        for bad in (b"not json", b"[]", b"{}", json.dumps({"format": "xaml-ui"}).encode()):
            with self.subTest(bad=bad):
                self.assert_rejected("wpf_task_panel.xaml", bad, "invalid_report")

    def test_changed_stdout_between_runs_fails_with_the_xaml_phase(self):
        with patch.object(runner, "run_command", side_effect=[(b"one", 1), (b"two", 1)]):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.check_xaml(runner.ROOT, 2)
        self.assertEqual((caught.exception.phase, caught.exception.code),
                         ("xaml_minimal_analysis", "nondeterministic_stdout"))

    def test_timeout_and_nonzero_exit_keep_the_phase(self):
        with patch.object(runner.subprocess, "run",
                          side_effect=subprocess.TimeoutExpired("tool", 1)):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.check_xaml(runner.ROOT, 2)
        self.assertEqual((caught.exception.phase, caught.exception.code),
                         ("xaml_minimal_analysis", "timeout"))
        result = subprocess.CompletedProcess(["tool"], 3, stdout=b"{}", stderr=b"")
        with patch.object(runner.subprocess, "run", return_value=result):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.check_xaml(runner.ROOT, 2)
        self.assertEqual((caught.exception.phase, caught.exception.code),
                         ("xaml_minimal_analysis", "exit_3"))

    def test_later_case_failure_keeps_its_own_phase(self):
        side = [(json.dumps(self.reports[self.SOURCES[0]]).encode(), {}),
                (json.dumps(self.reports[self.SOURCES[1]]).encode(), {}),
                runner.EvaluationError("winui_xaml_analysis", "timeout")]
        with self.assertRaises(runner.EvaluationError) as caught:
            self.run_with(side=side)
        self.assertEqual((caught.exception.phase, caught.exception.code),
                         ("winui_xaml_analysis", "timeout"))

    def test_xaml_input_mutation_is_detected_even_after_failure(self):
        xaml_inputs = [path for path in runner.INPUTS if path.suffix == ".xaml"]
        self.assertEqual(len(xaml_inputs), 3)
        for xaml_input in xaml_inputs:
            with self.subTest(source=xaml_input.name), tempfile.TemporaryDirectory(
                    prefix="ndp-runner-xaml-integrity-") as directory:
                root = Path(directory)
                for path in runner.INPUTS:
                    (root / path).parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(runner.ROOT / path, root / path)
                changed = root / xaml_input

                def mutate_then_fail(*args, **kwargs):
                    changed.write_bytes(changed.read_bytes() + b"<!-- changed -->")
                    raise runner.EvaluationError("xaml_phase", "exit_1")

                with patch.object(runner.shutil, "which", return_value="clang"), \
                        patch.object(runner, "run_command", side_effect=mutate_then_fail):
                    with self.assertRaises(runner.EvaluationError) as caught:
                        runner.evaluate(root, 2)
                self.assertEqual((caught.exception.phase, caught.exception.code),
                                 ("input_integrity", "source_changed"))
                self.assertTrue(changed.read_bytes().endswith(b"<!-- changed -->"))


@unittest.skipIf(runner.validator_unavailable(runner.ROOT),
                 "prepared out/ndp-host nagi-pkg is not available (reported as not_run)")
class ManifestValidationTests(unittest.TestCase):
    PHASE = runner.MANIFEST_PHASE

    def run_with(self, mutate=None, repeats=3):
        """Run the real generator and adapter; optionally alter stdout or the written files."""
        real = runner.run_command
        calls = []

        def wrapper(command, root, phase, **kwargs):
            output, elapsed = real(command, root, phase, **kwargs)
            if str(runner.VALIDATION_ADAPTER) in command:
                directory = Path(command[command.index("--root") + 1])
                calls.append(directory)
                if mutate is not None:
                    output = mutate(len(calls), directory, output)
            return output, elapsed

        with patch.object(runner, "run_command", side_effect=wrapper):
            return runner.check_manifest_validation(runner.ROOT, repeats), calls

    def assert_fails(self, code, mutate):
        with self.assertRaises(runner.EvaluationError) as caught:
            self.run_with(mutate)
        self.assertEqual((caught.exception.phase, caught.exception.code), (self.PHASE, code))

    @staticmethod
    def on_call(number, change):
        """Alter the report of adapter call `number` (1 valid, 2 control, ... per repeat)."""
        def mutate(n, directory, out):
            if n != number:
                return out
            data = json.loads(out)
            change(data)
            return json.dumps(data).encode("utf-8")
        return mutate

    def test_real_validator_accepts_the_scaffold_three_times_and_rejects_the_control(self):
        before = runner.fingerprint(runner.ROOT)
        info, calls = self.run_with()
        self.assertEqual(len(calls), 6)  # (scaffold, control) x 3
        self.assertEqual(info["status"], "passed")
        self.assertEqual(info["negative_control"], "rejected")
        self.assertTrue(info["stdout_identical"] and info["tree_identical"])
        self.assertEqual(info["sdk_contract"], [1, 0])
        self.assertEqual(len(info["samples_ms"]), 3)
        for directory in calls:  # always a throwaway directory outside the checkout
            self.assertNotIn(runner.ROOT.resolve(), (directory.resolve(),
                                                    *directory.resolve().parents))
        self.assertEqual(runner.fingerprint(runner.ROOT), before)
        self.assertFalse((runner.ROOT / "samples/level-zero").exists())

    def test_accept_everything_validator_is_detected_by_the_negative_control(self):
        accepted = {"app_id": "1", "captured_output_bytes": 1, "check": "manifest",
                    "exit_code": 0, "reason": "metadata_valid", "scope": "host_metadata_only",
                    "sdk_contract": [1, 0], "stage": 1, "status": "passed"}
        self.assert_fails("fixture_expectation_mismatch",
                          lambda n, d, out: json.dumps(accepted).encode() if n == 2 else out)

    def test_rejected_or_altered_report_for_the_scaffold_fails(self):
        changes = (
            lambda d: d.update(status="failed", reason="validator_rejected", exit_code=2),
            lambda d: d.update(reason="other"),
            lambda d: d.update(exit_code=1),
            lambda d: d.update(sdk_contract=[2, 0]),
            lambda d: d.update(app_id="not-digits"),
            lambda d: d.update(scope="nagi_target"),
            lambda d: d.update(stage=2),
            lambda d: d.update(check="package"),
            lambda d: d.update(extra=True),
            lambda d: d.pop("sdk_contract"),
        )
        for index, change in enumerate(changes):
            with self.subTest(change=index):
                self.assert_fails("fixture_expectation_mismatch", self.on_call(1, change))

    def test_altered_control_report_fails(self):
        changes = (
            lambda d: d.update(reason="validator_failed"),
            lambda d: d.update(exit_code=1),
            lambda d: d.update(status="unavailable"),
            lambda d: d.update(app_id="1"),
        )
        for index, change in enumerate(changes):
            with self.subTest(change=index):
                self.assert_fails("fixture_expectation_mismatch", self.on_call(2, change))

    def test_changed_report_between_runs_fails(self):
        self.assert_fails("nondeterministic_output",
                          self.on_call(3, lambda d: d.update(app_id="1234")))
        self.assert_fails("nondeterministic_output",
                          self.on_call(4, lambda d: d.update(captured_output_bytes=1)))

    def test_validator_that_rewrites_the_manifest_fails(self):
        def rewrite(n, directory, out):
            if n == 1:
                (directory / "manifest.json").write_bytes(b"{}")
            return out
        self.assert_fails("manifest_changed_by_validator", rewrite)

    def test_malformed_reports_fail_as_invalid(self):
        for bad in (b"not json", b"[]"):
            with self.subTest(bad=bad):
                self.assert_fails("invalid_report", lambda n, d, out, bad=bad: bad)

    def test_timeout_and_unexpected_exit_codes_keep_the_phase(self):
        real = subprocess.run

        def make_fake(code):
            def fake(command, *args, **kwargs):
                if str(runner.VALIDATION_ADAPTER) not in command:
                    return real(command, *args, **kwargs)  # the real generator
                if code is None:
                    raise subprocess.TimeoutExpired(command, 1)
                return subprocess.CompletedProcess(command, code, stdout=b"{}", stderr=b"")
            return fake

        for code, expected in ((None, "timeout"), (2, "exit_2"), (124, "exit_124")):
            with self.subTest(code=code):
                with patch.object(runner.subprocess, "run", side_effect=make_fake(code)):
                    with self.assertRaises(runner.EvaluationError) as caught:
                        runner.check_manifest_validation(runner.ROOT, 2)
                self.assertEqual((caught.exception.phase, caught.exception.code),
                                 (self.PHASE, expected))

    def test_generator_failure_keeps_the_phase(self):
        with patch.object(runner.subprocess, "run",
                          side_effect=subprocess.TimeoutExpired("tool", 1)):
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.check_manifest_validation(runner.ROOT, 2)
        self.assertEqual((caught.exception.phase, caught.exception.code), (self.PHASE, "timeout"))


class ManifestValidatorAvailabilityTests(unittest.TestCase):
    def make_root(self, directory, mode=0o755, symlink=False):
        root = Path(directory)
        target = root / runner.VALIDATOR
        target.parent.mkdir(parents=True)
        if symlink:
            real = root / "real-tool"
            real.write_bytes(b"#!/bin/sh\n")
            real.chmod(0o755)
            target.symlink_to(real)
        else:
            target.write_bytes(b"#!/bin/sh\n")
            target.chmod(mode)
        return root

    def test_missing_validator_is_not_run_and_starts_no_process(self):
        with tempfile.TemporaryDirectory(prefix="ndp-runner-novalidator-") as directory:
            with patch.object(runner.subprocess, "run") as process:
                result = runner.check_manifest_validation(Path(directory), 3)
        self.assertEqual(result["status"], "not_run")
        self.assertEqual(result["reason"], "validator_unavailable")
        self.assertNotIn("stdout_identical", result)
        process.assert_not_called()

    def test_symlinked_or_non_executable_validator_is_not_run(self):
        for kwargs in ({"mode": 0o644}, {"symlink": True}):
            with self.subTest(kwargs=kwargs), tempfile.TemporaryDirectory(
                    prefix="ndp-runner-badvalidator-") as directory:
                root = self.make_root(directory, **kwargs)
                with patch.object(runner.subprocess, "run") as process:
                    result = runner.check_manifest_validation(root, 2)
                self.assertEqual((result["status"], result["reason"]),
                                 ("not_run", "validator_unavailable"))
                process.assert_not_called()

    def test_unavailable_validator_is_listed_as_not_run_never_as_passed(self):
        stubs = {"check_stage2": {}, "check_api": {}, "check_generator": {}, "check_xaml": {}}
        patches = [patch.object(runner.shutil, "which", return_value="clang"),
                   patch.object(runner, "run_command", return_value=(b"clang version", 1)),
                   patch.object(runner, "repeat_command", return_value=(b"{}", {})),
                   patch.object(runner, "validator_unavailable", return_value=True),
                   patch.object(runner, "UI_CASES", tuple(
                       (phase, source, lambda facts: {}) for phase, source, _ in runner.UI_CASES))]
        patches += [patch.object(runner, name, return_value=value) for name, value in stubs.items()]
        for item in patches:
            item.start()
            self.addCleanup(item.stop)
        result = runner.evaluate(runner.ROOT, 2)
        self.assertEqual(result[runner.MANIFEST_PHASE]["status"], "not_run")
        self.assertIn(runner.MANIFEST_PHASE, result["not_run"])
        self.assertIn("sdk_validator", result["not_run"])

    def test_ran_validator_is_removed_from_not_run(self):
        result = NewPhaseOrchestrationTests.evaluate_with(
            self, check_manifest_validation={"status": "passed"})
        self.assertNotIn(runner.MANIFEST_PHASE, result["not_run"])
        self.assertNotIn("sdk_validator", result["not_run"])
        self.assertIn("generated_crate_build", result["not_run"])

    def test_validator_binary_change_is_detected_as_input_change(self):
        with tempfile.TemporaryDirectory(prefix="ndp-runner-validator-integrity-") as directory:
            root = self.make_root(directory)
            for path in runner.INPUTS:
                (root / path).parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(runner.ROOT / path, root / path)
            self.assertIn(runner.VALIDATOR.as_posix(), runner.fingerprint(root))

            def swap_validator(*args, **kwargs):
                (root / runner.VALIDATOR).write_bytes(b"#!/bin/sh\nexit 0\n")
                return {"status": "passed"}

            patches = [patch.object(runner.shutil, "which", return_value="clang"),
                       patch.object(runner, "run_command", return_value=(b"clang version", 1)),
                       patch.object(runner, "repeat_command", return_value=(b"{}", {})),
                       patch.object(runner, "UI_CASES", tuple(
                           (phase, source, lambda facts: {})
                           for phase, source, _ in runner.UI_CASES)),
                       patch.object(runner, "check_stage2", return_value={}),
                       patch.object(runner, "check_api", return_value={}),
                       patch.object(runner, "check_generator", return_value={}),
                       patch.object(runner, "check_xaml", return_value={}),
                       patch.object(runner, "check_manifest_validation",
                                    side_effect=swap_validator)]
            for item in patches:
                item.start()
                self.addCleanup(item.stop)
            with self.assertRaises(runner.EvaluationError) as caught:
                runner.evaluate(root, 2)
        self.assertEqual((caught.exception.phase, caught.exception.code),
                         ("input_integrity", "source_changed"))


class NewPhaseOrchestrationTests(unittest.TestCase):
    PHASES = ("stage2_build_test", "api_windows_read_prefix", "generator_scaffold")

    def evaluate_with(self, **stubs):
        quiet = {"check_stage2": {}, "check_api": {}, "check_generator": {},
                 "check_xaml": {}, "check_manifest_validation": {}}
        quiet.update(stubs)
        patches = [patch.object(runner.shutil, "which", return_value="clang"),
                   patch.object(runner, "run_command", return_value=(b"clang version", 1)),
                   patch.object(runner, "repeat_command", return_value=(b"{}", {}))]
        patches += [patch.object(runner, name, **(
            {"side_effect": value} if callable(value) or isinstance(value, Exception)
            else {"return_value": value})) for name, value in quiet.items()]
        # The portable and UI phases are not under test: accept their stub output.
        patches += [patch.object(runner, "UI_CASES", tuple(
            (phase, source, lambda facts: {}) for phase, source, _ in runner.UI_CASES))]
        for item in patches:
            item.start()
            self.addCleanup(item.stop)
        return runner.evaluate(runner.ROOT, 3)

    def test_each_new_phase_failure_is_reported_with_its_phase_and_stops_later_phases(self):
        names = {"stage2_build_test": "check_stage2", "api_windows_read_prefix": "check_api",
                 "generator_scaffold": "check_generator", "wpf_xaml_analysis": "check_xaml",
                 runner.MANIFEST_PHASE: "check_manifest_validation"}
        order = ["check_stage2", "check_api", "check_generator", "check_xaml",
                 "check_manifest_validation"]
        for phase, name in names.items():
            for code in ("timeout", "exit_1", "nondeterministic_stdout",
                         "fixture_expectation_mismatch"):
                with self.subTest(phase=phase, code=code):
                    later = {n: runner.EvaluationError("never", "ran")
                             for n in order[order.index(name) + 1:]}
                    with self.assertRaises(runner.EvaluationError) as caught:
                        self.evaluate_with(**{name: runner.EvaluationError(phase, code)},
                                           **later)
                    self.assertEqual((caught.exception.phase, caught.exception.code),
                                     (phase, code))

    def test_all_new_phases_are_reported_on_success(self):
        result = self.evaluate_with(check_stage2={"stage": 2}, check_api={"a": {}},
                                    check_generator={"files": []},
                                    check_xaml={"wpf_xaml_analysis": {}},
                                    check_manifest_validation={"status": "passed"})
        self.assertEqual(result["status"], "PASS")
        for key in ("stage2_build_test", "api_rules", "generator_scaffold",
                    "wpf_xaml_analysis", runner.MANIFEST_PHASE):
            self.assertIn(key, result)
        for item in ("scanner_output_to_api_rules", "generated_crate_build",
                     "stage2_third_party_build", "stage3_nagi_target"):
            self.assertIn(item, result["not_run"])

    def test_new_inputs_are_fingerprinted_for_every_tool_and_fixture(self):
        for group in (runner.API_INPUTS, runner.GENERATOR_INPUTS, runner.STAGE2_INPUTS,
                      runner.XAML_INPUTS):
            self.assertTrue(set(group) <= set(runner.INPUTS))
            for path in group:
                self.assertTrue((runner.ROOT / path).is_file(), path)
        # Every approved API fixture, every rules file and every template is covered.
        self.assertEqual(len(runner.API_INPUTS), 10)
        self.assertEqual(len(runner.GENERATOR_INPUTS), 8)
        self.assertEqual(len(runner.STAGE2_INPUTS), 3)
        self.assertEqual(len(runner.XAML_INPUTS), 3)
        # The adapter that drives the real validator is already a fingerprinted input.
        self.assertIn(runner.VALIDATION_ADAPTER, runner.STAGE2_INPUTS)

    def test_input_mutation_after_a_passing_phase_is_detected(self):
        for group, index in ((runner.API_INPUTS, 0), (runner.API_INPUTS, 6),
                             (runner.GENERATOR_INPUTS, 3), (runner.GENERATOR_INPUTS, 7),
                             (runner.STAGE2_INPUTS, 1)):
            changed_path = group[index]
            with self.subTest(source=changed_path.as_posix()), tempfile.TemporaryDirectory(
                    prefix="ndp-runner-new-integrity-") as directory:
                root = Path(directory)
                for path in runner.INPUTS:
                    (root / path).parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(runner.ROOT / path, root / path)

                def mutate_then_pass(*args, changed=root / changed_path, **kwargs):
                    changed.write_bytes(changed.read_bytes() + b"\n")
                    return {}

                with patch.object(runner.shutil, "which", return_value="clang"), \
                        patch.object(runner, "run_command", return_value=(b"clang version", 1)), \
                        patch.object(runner, "repeat_command", return_value=(b"{}", {})), \
                        patch.object(runner, "UI_CASES", tuple(
                            (phase, source, lambda facts: {})
                            for phase, source, _ in runner.UI_CASES)), \
                        patch.object(runner, "check_stage2", side_effect=mutate_then_pass), \
                        patch.object(runner, "check_api", return_value={}), \
                        patch.object(runner, "check_generator", return_value={}), \
                        patch.object(runner, "check_xaml", return_value={}), \
                        patch.object(runner, "check_manifest_validation", return_value={}):
                    with self.assertRaises(runner.EvaluationError) as caught:
                        runner.evaluate(root, 2)
                self.assertEqual((caught.exception.phase, caught.exception.code),
                                 ("input_integrity", "source_changed"))


if __name__ == "__main__":
    unittest.main()
