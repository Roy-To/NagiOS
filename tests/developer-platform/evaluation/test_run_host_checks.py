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
                patch.object(runner, "repeat_command", side_effect=outputs):
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


if __name__ == "__main__":
    unittest.main()
