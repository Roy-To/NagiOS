#!/usr/bin/env python3
"""Repeat the fixed NDP host fixtures; never run the platform inputs."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[3]
FIXTURES = Path("tests/developer-platform/evaluation")
ANALYZER = Path("tools/nagi-developer-platform/ui/analysis/analyze.py")
INPUTS = tuple(FIXTURES / name for name in (
    "portable/summary.c", "portable/summary.h", "portable/summary_test.c",
    "windows/read_prefix.c", "linux/watch_changes.c", "controls/text_only.c",
    "ui/task_panel.ui", "ui/gtk3_task_panel.ui", "ui/gtk4_task_panel.ui",
)) + (ANALYZER,)


class EvaluationError(Exception):
    def __init__(self, phase, code):
        self.phase = phase
        self.code = code
        super().__init__(phase + ": " + code)


def fingerprint(root):
    return {path.as_posix(): hashlib.sha256((root / path).read_bytes()).hexdigest()
            for path in INPUTS}


def run_command(command, root, phase, timeout=10):
    start = time.perf_counter()
    try:
        result = subprocess.run(
            command, cwd=root, env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
            capture_output=True, timeout=timeout, check=False,
        )
    except subprocess.TimeoutExpired:
        raise EvaluationError(phase, "timeout") from None
    except OSError:
        raise EvaluationError(phase, "command_unavailable") from None
    elapsed_ms = (time.perf_counter() - start) * 1000
    if result.returncode != 0:
        # Forward actual diagnostics, but keep the machine-readable error stable.
        if result.stderr:
            print(result.stderr.decode("utf-8", errors="replace"), file=sys.stderr)
        raise EvaluationError(phase, "exit_" + str(result.returncode))
    return result.stdout, elapsed_ms


def repeat_command(command, root, phase, repeats):
    first = None
    samples = []
    for _ in range(repeats):
        output, elapsed_ms = run_command(command, root, phase)
        if first is not None and output != first:
            raise EvaluationError(phase, "nondeterministic_stdout")
        first = output
        samples.append(elapsed_ms)
    return first, {
        "samples_ms": samples,
        "median_ms": statistics.median(samples),
        "stdout_identical": True,
    }


COUNT_KEYS = ("widgets", "layouts", "tab_order", "connections", "unknown",
              "unresolved_references")


def pairs(items):
    return [(item["name"], item["class"]) for item in items]


def count_facts(facts):
    return {key: len(facts[key]) for key in COUNT_KEYS}


def mismatch(phase):
    raise EvaluationError(phase, "fixture_expectation_mismatch")


def validate_qt(facts):
    phase = "ui_analysis"
    counts = count_facts(facts)
    expected_counts = {"widgets": 3, "layouts": 1, "tab_order": 2, "connections": 1,
                       "unknown": 0, "unresolved_references": 0}
    # Expectations come from the fixed synthetic XML, not from a second scanner.
    if (counts != expected_counts
            or facts["source"] != "task_panel.ui"
            or facts["form_class"] != "TaskPanel"
            or pairs(facts["widgets"]) != [("TaskPanel", "QWidget"),
                                           ("taskName", "QLineEdit"),
                                           ("addButton", "QPushButton")]
            or pairs(facts["layouts"]) != [("layout", "QVBoxLayout")]
            or facts["tab_order"] != ["taskName", "addButton"]
            or facts["connections"] != [{"sender": "addButton",
                "signal": "clicked()", "receiver": "TaskPanel",
                "slot": "addTask()", "execution": "not_run"}]):
        mismatch(phase)
    return counts


def layout_positions(layout):
    """Ordered (child, ((key, text), ...)) from GTK 3 packing or GTK 4 layout data."""
    result = []
    for item in layout["items"]:
        keys = [(prop["name"], value["text"]) for prop in item["packing"]
                for value in prop["values"]]
        for obj in item["objects"]:
            for prop in obj["properties"]:
                if prop["name"] == "layout":
                    keys += [(value["attributes"]["name"], value["text"])
                             for value in prop["values"] if value["kind"] == "property"]
            result.append((obj["name"], tuple(keys)))
    return result


def validate_gtk(facts, phase, source, version, widgets, layout, positions,
                 receiver, unknown, owner_binding):
    counts = count_facts(facts)
    resource = facts["resources"]
    bindings = [(item["owner"], item["tag"], item["execution"])
                for item in facts["bindings"]]
    if (facts["format"] != "gtk-builder-ui" or facts["source"] != source
            or counts != {"widgets": len(widgets), "layouts": 1, "tab_order": 0,
                          "connections": 1, "unknown": len(unknown),
                          "unresolved_references": 0}
            or [(w["name"], w["class"], w["parent_widget"]) for w in facts["widgets"]]
            != widgets
            or pairs(facts["layouts"]) != [layout]
            or layout_positions(facts["layouts"][0]) != positions
            or facts["connections"] != [{
                "attributes": {"handler": "on_add_task", "name": "clicked",
                               **({"object": receiver, "swapped": "no"} if receiver else {})},
                "execution": "not_run", "receiver": receiver, "sender": "addButton",
                "signal": "clicked", "slot": "on_add_task"}]
            or [(item["name"], item["class"]) for item in facts["unknown"]] != unknown
            or bindings != [owner_binding]
            or [(r["kind"], r["attributes"]) for r in resource]
            != [("requires", {"lib": version[0], "version": version[1]})]
            or facts["tab_order"] != []):
        mismatch(phase)
    return counts


def validate_gtk3(facts):
    return validate_gtk(
        facts, "gtk3_ui_analysis", "gtk3_task_panel.ui", ("gtk+", "3.24"),
        [("taskWindow", "GtkWindow", None), ("body", "GtkBox", "taskWindow"),
         ("title", "GtkLabel", "body"), ("taskName", "GtkEntry", "body"),
         ("entryAccessible", "AtkObject", "taskName"), ("addButton", "GtkButton", "body")],
        ("body", "GtkBox"),
        [("title", (("expand", "False"), ("position", "0"))),
         ("taskName", (("expand", "True"), ("position", "1"))),
         ("addButton", (("position", "2"),))],
        "taskWindow", [("entryAccessible", "AtkObject")],
        ("taskName", "property", "not_run"))


def validate_gtk4(facts):
    return validate_gtk(
        facts, "gtk4_ui_analysis", "gtk4_task_panel.ui", ("gtk", "4.0"),
        [("taskWindow", "GtkWindow", None), ("grid", "GtkGrid", "taskWindow"),
         ("title", "GtkLabel", "grid"), ("taskName", "GtkEntry", "grid"),
         ("addButton", "GtkButton", "grid")],
        ("grid", "GtkGrid"),
        [("title", (("column", "0"), ("row", "0"))),
         ("taskName", (("column", "1"), ("row", "0"))),
         ("addButton", (("column", "1"), ("row", "1"), ("column-span", "2")))],
        "", [], ("taskName", "binding", "not_run"))


UI_CASES = (
    ("ui_analysis", "task_panel.ui", validate_qt),
    ("gtk3_ui_analysis", "gtk3_task_panel.ui", validate_gtk3),
    ("gtk4_ui_analysis", "gtk4_task_panel.ui", validate_gtk4),
)


def evaluate(root, repeats):
    if not 2 <= repeats <= 20:
        raise EvaluationError("arguments", "repeats_must_be_2_to_20")
    compiler = shutil.which("clang")
    if compiler is None:
        raise EvaluationError("portable_build", "clang_unavailable")
    before = fingerprint(root)
    try:
        version, _ = run_command([compiler, "--version"], root, "compiler_version")
        with tempfile.TemporaryDirectory(prefix="nagi-ndp-host-") as directory:
            binary = str(Path(directory) / "summary-test")
            flags = ["-std=c11", "-Wall", "-Wextra", "-Werror", "-pedantic"]
            _, build_ms = run_command(
                [compiler] + flags + [str(FIXTURES / "portable/summary.c"),
                                      str(FIXTURES / "portable/summary_test.c"),
                                      "-o", binary], root, "portable_build", timeout=30,
            )
            _, control_ms = run_command(
                [compiler] + flags + ["-fsyntax-only", str(FIXTURES / "controls/text_only.c")],
                root, "control_syntax", timeout=30,
            )
            _, portable = repeat_command([binary], root, "portable_run", repeats)
            ui_reports = {}
            for phase, source, validate in UI_CASES:
                output, ui = repeat_command(
                    [sys.executable, "-B", str(ANALYZER), "--root",
                     str(FIXTURES / "ui"), source], root, phase, repeats,
                )
                try:
                    ui["facts"] = validate(json.loads(output))
                except (ValueError, KeyError, TypeError, AttributeError):
                    raise EvaluationError(phase, "invalid_report") from None
                ui_reports[phase] = ui
    finally:
        # Check even when a build, run or comparison fails. Never restore a changed input.
        if fingerprint(root) != before:
            raise EvaluationError("input_integrity", "source_changed")
    return {
        "status": "PASS", "scope": "synthetic_host_fixtures_only", "repeats": repeats,
        "environment": {"os": platform.system(), "machine": platform.machine(),
                        "python": platform.python_version(),
                        "compiler": version.decode("utf-8").splitlines()[0]},
        "portable_build_ms": build_ms, "control_syntax_ms": control_ms,
        "portable_run": portable, **ui_reports,
        "timing_scope": "subprocess wall time including startup; not human work or speedup",
        "inputs_unchanged": True,
        "not_run": ["windows_input", "linux_input", "gtk_runtime", "manual_comparison", "llm_comparison",
                    "sdk_validator", "ui_rendering", "nagi_target"],
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeats", type=int, default=3, help="2 to 20 runs (default: 3)")
    args = parser.parse_args(argv)
    try:
        result = evaluate(ROOT, args.repeats)
    except EvaluationError as error:
        print(json.dumps({"status": "FAIL", "phase": error.phase, "error": error.code}),
              file=sys.stderr)
        return 1
    except OSError:
        print(json.dumps({"status": "FAIL", "error": "source_unavailable"}), file=sys.stderr)
        return 1
    print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
