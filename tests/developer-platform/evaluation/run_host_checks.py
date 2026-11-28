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

# NDP-04/05 API rules, NDP-03 generator and NDP-09s2 Stage 2 (trusted tools, read-only inputs).
API_TOOLS = Path("tools/nagi-developer-platform/api")
API_REPORTS = Path("tests/developer-platform/api")
GENERATOR_DIR = Path("tools/nagi-developer-platform/generator")
GENERATOR = GENERATOR_DIR / "generate.py"
STAGE2 = Path("tools/nagi-developer-platform/validation/stage2.py")
STAGE2_TARGET = "portable-summary"
API_INPUTS = tuple(
    API_REPORTS / name for name in (
        "windows/read_prefix.report-v1.json", "windows/text_only.report-v1.json",
        "linux/watch_changes.report-v1.json", "linux/text_only.report-v1.json",
        "linux/mixed_platform.report-v1.json")
) + tuple(API_TOOLS / name for name in (
    "windows/diagnose.py", "windows/rules.json", "linux/diagnose.py", "linux/rules.json",
    "common/pt01_report.py"))
GENERATOR_INPUTS = tuple(GENERATOR_DIR / name for name in (
    "generate.py", "contract/contract.py", "contract/level0.json",
    "contract/templates/Cargo.toml.tmpl", "contract/templates/UNSUPPORTED_APIS.md.tmpl",
    "contract/templates/README.md.tmpl", "contract/templates/lib.rs.tmpl"
)) + (Path("sdk/rust/schemas/app-manifest.schema.json"),)
STAGE2_INPUTS = tuple(Path("tools/nagi-developer-platform/validation") / name for name in (
    "stage2.py", "stages.json", "validate.py"))
# NDP-06d XAML fixtures (static source only) and the NDP-09 Stage 1 adapter over the real nagi-pkg.
XAML_INPUTS = tuple(FIXTURES / "ui" / name for name in (
    "xaml_minimal.xaml", "wpf_task_panel.xaml", "winui_task_panel.xaml"))
INPUTS = INPUTS + API_INPUTS + GENERATOR_INPUTS + STAGE2_INPUTS + XAML_INPUTS
VALIDATION_ADAPTER = Path("tools/nagi-developer-platform/validation/validate.py")
# Prepared by a separate developer action (never built or modified here); optional input.
VALIDATOR = Path("out/ndp-host/target/debug/nagi-pkg")


class EvaluationError(Exception):
    def __init__(self, phase, code):
        self.phase = phase
        self.code = code
        super().__init__(phase + ": " + code)


def fingerprint(root):
    result = {path.as_posix(): hashlib.sha256((root / path).read_bytes()).hexdigest()
              for path in INPUTS}
    validator = root / VALIDATOR
    if validator.is_file():
        # The prepared validator is a trusted tool; it must not change during a run either.
        result[VALIDATOR.as_posix()] = hashlib.sha256(validator.read_bytes()).hexdigest()
    return result


def run_command(command, root, phase, timeout=10, stdin_path=None, allowed_codes=(0,)):
    start = time.perf_counter()
    options = {}
    try:
        if stdin_path is not None:
            # A fixed fixture file is fed to the tool; no shell and no redirection.
            options["input"] = (root / stdin_path).read_bytes()
        result = subprocess.run(
            command, cwd=root, env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
            capture_output=True, timeout=timeout, check=False, **options,
        )
    except subprocess.TimeoutExpired:
        raise EvaluationError(phase, "timeout") from None
    except OSError:
        raise EvaluationError(phase, "command_unavailable") from None
    elapsed_ms = (time.perf_counter() - start) * 1000
    if result.returncode not in allowed_codes:
        # Forward actual diagnostics, but keep the machine-readable error stable.
        if result.stderr:
            print(result.stderr.decode("utf-8", errors="replace"), file=sys.stderr)
        raise EvaluationError(phase, "exit_" + str(result.returncode))
    return result.stdout, elapsed_ms


def repeat_command(command, root, phase, repeats, timeout=10, stdin_path=None):
    first = None
    samples = []
    for _ in range(repeats):
        output, elapsed_ms = run_command(command, root, phase, timeout=timeout,
                                         stdin_path=stdin_path)
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


def parse_json_object(phase, output):
    try:
        data = json.loads(output)
    except ValueError:
        raise EvaluationError(phase, "invalid_report") from None
    if not isinstance(data, dict):
        raise EvaluationError(phase, "invalid_report")
    return data


# --- NDP-09s2 Stage 2 (self-made portable C fixture only) -------------------------

def validate_stage2(data):
    steps = data.get("steps")
    if (data.get("stage") != 2 or data.get("status") != "passed"
            or data.get("reason") != "build_and_test_passed"
            or data.get("target") != STAGE2_TARGET
            or data.get("scope") != "host_build_test_fixture_only"
            or not isinstance(steps, list)
            or [(s.get("name"), s.get("status"), s.get("exit_code")) for s in steps]
            != [("compile", "passed", 0), ("run_test", "passed", 0)]):
        mismatch("stage2_build_test")


def check_stage2(root, repeats):
    phase = "stage2_build_test"
    output, info = repeat_command(
        [sys.executable, "-B", str(STAGE2), "--target", STAGE2_TARGET,
         "--fixture-dir", str(FIXTURES / "portable")], root, phase, repeats, timeout=90)
    try:
        validate_stage2(parse_json_object(phase, output))
    except (KeyError, TypeError, AttributeError):
        raise EvaluationError(phase, "invalid_report") from None
    info["stage"] = 2
    info["steps_passed"] = ["compile", "run_test"]
    return info


# --- NDP-04/05 API rules over handwritten PT-01 report v1 fixtures -----------------

# Expectations are written from the evidence text of each fixed report fixture:
# (line, rule id, detection, matched aliases). They are not copied from a second scanner.
API_CASES = (
    ("api_windows_read_prefix", "windows", "windows/read_prefix.report-v1.json", "read_prefix.c", [
        (2, "WIN-HEADER", "lexical_candidate", ["windows.h"]),
        (4, "WIN-FILE-OPEN", "lexical_candidate", ["CreateFileW"]),
        (7, "WIN-FILE-READ", "lexical_candidate", ["ReadFile"]),
        (8, "WIN-HANDLE-CLOSE", "lexical_candidate", ["CloseHandle"])]),
    ("api_windows_text_only", "windows", "windows/text_only.report-v1.json", "text_only.c", [
        (1, None, "no_code_evidence", []),
        (3, None, "unknown_api", [])]),
    ("api_linux_watch_changes", "linux", "linux/watch_changes.report-v1.json", "watch_changes.c", [
        (2, "LNX-INOTIFY", "lexical_candidate", ["sys/inotify.h"]),
        (3, "POSIX-HEADER-FD", "lexical_candidate", ["unistd.h"]),
        (5, "LNX-INOTIFY", "lexical_candidate", ["IN_CLOEXEC", "IN_NONBLOCK", "inotify_init1"]),
        (7, "LNX-INOTIFY", "lexical_candidate", ["IN_MODIFY", "inotify_add_watch"]),
        (8, "POSIX-FD-CLOSE", "lexical_candidate", ["close"])]),
    ("api_linux_text_only", "linux", "linux/text_only.report-v1.json", "text_only.c", [
        (1, None, "no_code_evidence", []),
        (3, None, "unknown_api", [])]),
    ("api_linux_mixed_platform", "linux", "linux/mixed_platform.report-v1.json", "mixed.cpp", [
        (3, None, "not_classified", []),
        (5, "POSIX-FILE-OPEN", "lexical_candidate", ["open"]),
        (6, None, "unknown_api", []),
        (8, "LNX-QT", "lexical_candidate", ["QtWidgets/QApplication"]),
        (9, None, "unknown_api", [])]),
)


def validate_api(phase, data, path, expected):
    rows = data["rows"]
    if (data["diagnostic_version"] != 1 or data["scope"] != "host_report_analysis"
            or data["compatibility"] != "not_assessed"
            or data["target_acceptance"] != "not_run"
            or [(r["source"]["line"], r["rule_id"], r["detection"], r["original_apis"])
                for r in rows] != expected
            or any(r["source"]["path"] != path or r["approval"] != "PROVISIONAL"
                   or r["source"]["evidence_scope"] != "report_fragment_only" for r in rows)
            # A negative control or an unmatched row must never claim a known API.
            or any(r["rule_id"] is None and (r["original_apis"] or r["classification"] != "unresolved")
                   for r in rows)):
        mismatch(phase)
    return len(rows)


def check_api(root, repeats):
    results = {}
    for phase, tool, report, path, expected in API_CASES:
        output, info = repeat_command(
            [sys.executable, "-B", str(API_TOOLS / tool / "diagnose.py"), "--path", path],
            root, phase, repeats, stdin_path=API_REPORTS / report)
        try:
            info["rows"] = validate_api(phase, parse_json_object(phase, output), path, expected)
        except (KeyError, TypeError, AttributeError):
            raise EvaluationError(phase, "invalid_report") from None
        results[phase] = info
    return results


# --- NDP-03 generator into a temporary output only -----------------------------------

GENERATOR_ARGS = ("create", "--app-id", "com.example.level-zero", "--package-name",
                  "level-zero", "--name-en", "Level Zero", "--name-ja", "レベルゼロ",
                  "--output-dir", "samples/level-zero")
GENERATOR_FILES = ["Cargo.toml", "README.md", "UNSUPPORTED_APIS.md", "manifest.json",
                   "src/lib.rs"]


def tree_digest(base):
    return {path.relative_to(base).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(base.rglob("*")) if path.is_file()}


def validate_generated(phase, data, digest, manifest_text):
    created = sorted(item[len("samples/level-zero/"):] for item in data.get("created", []))
    try:
        manifest = json.loads(manifest_text)
        manifest_ok = (manifest["id"] == "com.example.level-zero"
                       and manifest["displayName"] == {"en-US": "Level Zero",
                                                       "ja-JP": "レベルゼロ"}
                       and manifest["requestedCapabilities"] == []
                       and manifest["origin"] == "third-party")
    except (ValueError, KeyError, TypeError):
        manifest_ok = False
    if (data.get("status") != "created" or data.get("built") is not False
            or data.get("validated") is not False
            or data.get("output_dir") != "samples/level-zero"
            or created != GENERATOR_FILES
            or sorted(digest) != GENERATOR_FILES or not manifest_ok):
        mismatch(phase)


def check_generator(root, repeats):
    phase = "generator_scaffold"
    first_output = first_digest = None
    samples = []
    for _ in range(repeats):
        # Fresh stand-in checkout each time: the generator refuses an existing output.
        with tempfile.TemporaryDirectory(prefix="nagi-ndp-generated-") as directory:
            target = Path(directory)
            (target / "sdk/rust").mkdir(parents=True)
            (target / "sdk/rust/Cargo.toml").write_text("# stand-in\n", encoding="utf-8")
            (target / "samples").mkdir()
            output, elapsed_ms = run_command(
                [sys.executable, "-B", str(GENERATOR), GENERATOR_ARGS[0], "--root", directory,
                 *GENERATOR_ARGS[1:]], root, phase)
            written = target / "samples/level-zero"
            digest = tree_digest(written)
            # Nothing may appear outside the requested output directory.
            if sorted(tree_digest(target)) != sorted(
                    ["sdk/rust/Cargo.toml"] + ["samples/level-zero/" + n for n in digest]):
                mismatch(phase)
            try:
                manifest_text = (written / "manifest.json").read_text(encoding="utf-8")
            except (OSError, ValueError):
                mismatch(phase)
            try:
                validate_generated(phase, parse_json_object(phase, output), digest,
                                   manifest_text)
            except (KeyError, TypeError, AttributeError):
                raise EvaluationError(phase, "invalid_report") from None
        if first_output is not None and (output != first_output or digest != first_digest):
            raise EvaluationError(phase, "nondeterministic_output")
        first_output, first_digest = output, digest
        samples.append(elapsed_ms)
    return {"samples_ms": samples, "median_ms": statistics.median(samples),
            "stdout_identical": True, "tree_identical": True, "files": sorted(first_digest)}


# --- NDP-06d XAML (WPF / WinUI) as a repeated input --------------------------------

# Expectations are written from the fixed synthetic XAML text; nothing is loaded or run.
# Shapes: widgets (name, class, parent), layouts (class, [(child, Grid.Row)]),
# connections (sender, signal, slot), unknown (kind, class or tag),
# unresolved (kind, name), bindings (owner, property, tag), resources (kind list).
XAML_CASES = (
    ("xaml_minimal_analysis", "xaml_minimal.xaml", {
        "form_class": "",
        "widgets": [("hello", "Button", None)], "layouts": [], "tab_order": [],
        "connections": [], "unknown": [], "unresolved": [], "bindings": [],
        "resources": [], "custom": []}),
    ("wpf_xaml_analysis", "wpf_task_panel.xaml", {
        "form_class": "Synthetic.TaskPanel",
        "widgets": [("taskWindow", "Window", None), ("grid", "Grid", "taskWindow"),
                    ("title", "TextBlock", "grid"), ("body", "StackPanel", "grid"),
                    ("taskName", "TextBox", "body"), ("addButton", "Button", "body"),
                    ("gauge", "{clr-namespace:Synthetic.Controls;assembly=Synthetic}GaugeControl",
                     "body")],
        "layouts": [("Grid", [("title", "0"), ("body", "1")]),
                    ("StackPanel", [("taskName", None), ("addButton", None), ("gauge", None)])],
        "tab_order": ["taskName", "addButton"],
        "connections": [("addButton", "Click", "AddButton_Click")],
        "unknown": [("widget", "{clr-namespace:Synthetic.Controls;assembly=Synthetic}GaugeControl")],
        "unresolved": [],
        "bindings": [("taskName", "Text", "Binding"), ("addButton", "Command", "Binding")],
        "resources": ["resource_definition", "resource_reference"],
        "custom": ["GaugeControl"]}),
    ("winui_xaml_analysis", "winui_task_panel.xaml", {
        "form_class": "Synthetic.WinUI.SettingsPage",
        "widgets": [("@xaml-object-1", "Page", None), ("rowLabel", "TextBlock", "@xaml-object-1"),
                    ("panel", "StackPanel", "@xaml-object-1"),
                    ("heading", "TextBlock", "panel"), ("notify", "ToggleSwitch", "panel"),
                    ("save", "Button", "panel"),
                    ("meter", "{using:Synthetic.WinUI.Controls}Meter", "panel")],
        "layouts": [("StackPanel", [("heading", None), ("notify", None), ("save", None),
                                    ("meter", None)])],
        "tab_order": [],
        "connections": [("notify", "Toggled", "{x:Bind ViewModel.OnToggled}"),
                        ("save", "Click", "Save_Click")],
        "unknown": [("element", "DataTemplate"),
                    ("widget", "{using:Synthetic.WinUI.Controls}Meter")],
        "unresolved": [("resource_key", "AccentTextBrush")],
        "bindings": [("rowLabel", "Text", "Binding"), ("notify", "IsOn", "x:Bind"),
                     ("notify", "Toggled", "x:Bind")],
        "resources": ["resource_definition", "resource_reference"],
        "custom": ["Meter"]}),
)


def xaml_projection(facts):
    return {
        "form_class": facts["form_class"],
        "widgets": [(w["name"], w["class"], w["parent_widget"]) for w in facts["widgets"]],
        "layouts": [(layout["class"], [(obj["name"], item["attributes"].get("Grid.Row"))
                                        for item in layout["items"] for obj in item["objects"]])
                    for layout in facts["layouts"]],
        "tab_order": facts["tab_order"],
        "connections": [(c["sender"], c["signal"], c["slot"]) for c in facts["connections"]],
        "unknown": [(u["kind"], u.get("class") or u.get("tag")) for u in facts["unknown"]],
        "unresolved": [(u["kind"], u["name"]) for u in facts["unresolved_references"]],
        "bindings": [(b["owner"], b["property"], b["tag"]) for b in facts["bindings"]],
        "resources": [r["kind"] for r in facts["resources"]],
        "custom": [c["class"] for c in facts["custom_widgets"]],
    }


def validate_xaml(phase, facts, source, expected):
    # Declared handlers and bindings are never executed.
    never_run = ([c["execution"] for c in facts["connections"]]
                 + [b["execution"] for b in facts["bindings"]])
    if (facts["format"] != "xaml-ui" or facts["source"] != source
            or xaml_projection(facts) != expected
            or any(state != "not_run" for state in never_run)):
        mismatch(phase)
    return count_facts(facts)


def check_xaml(root, repeats):
    results = {}
    for phase, source, expected in XAML_CASES:
        output, info = repeat_command(
            [sys.executable, "-B", str(ANALYZER), "--root", str(FIXTURES / "ui"), source],
            root, phase, repeats)
        try:
            info["facts"] = validate_xaml(phase, json.loads(output), source, expected)
        except (ValueError, KeyError, TypeError, AttributeError):
            raise EvaluationError(phase, "invalid_report") from None
        results[phase] = info
    return results


# --- Generated scaffold Manifest through the real nagi-pkg (Stage 1 adapter) ---------

MANIFEST_PHASE = "generated_manifest_validation"
REPORT_KEYS = {"captured_output_bytes", "check", "exit_code", "reason", "scope", "stage",
               "status"}


def validator_unavailable(root):
    path = root / VALIDATOR
    return path.is_symlink() or not path.is_file() or not os.access(path, os.X_OK)


def validate_manifest_report(data, accepted):
    """The adapter envelope for a valid scaffold, or for the negative control."""
    common = (data["check"] == "manifest" and data["scope"] == "host_metadata_only"
              and data["stage"] == 1 and isinstance(data["captured_output_bytes"], int))
    if accepted:
        ok = (set(data) == REPORT_KEYS | {"app_id", "sdk_contract"}
              and data["status"] == "passed" and data["reason"] == "metadata_valid"
              and data["exit_code"] == 0 and data["sdk_contract"] == [1, 0]
              and isinstance(data["app_id"], str) and data["app_id"].isdigit())
    else:
        ok = (set(data) == REPORT_KEYS and data["status"] == "failed"
              and data["reason"] == "validator_rejected" and data["exit_code"] == 2)
    if not (common and ok):
        mismatch(MANIFEST_PHASE)


def run_adapter(root, directory, accepted):
    output, elapsed_ms = run_command(
        [sys.executable, "-B", str(VALIDATION_ADAPTER), "--root", str(directory),
         "manifest.json"], root, MANIFEST_PHASE, timeout=30, allowed_codes=(0, 1))
    try:
        validate_manifest_report(parse_json_object(MANIFEST_PHASE, output), accepted)
    except (KeyError, TypeError, AttributeError):
        raise EvaluationError(MANIFEST_PHASE, "invalid_report") from None
    return output, elapsed_ms


def check_manifest_validation(root, repeats):
    if validator_unavailable(root):
        # Never a success: no process is started and the reason is reported.
        return {"status": "not_run", "reason": "validator_unavailable",
                "validator": VALIDATOR.as_posix()}
    first = None
    samples = []
    for _ in range(repeats):
        with tempfile.TemporaryDirectory(prefix="nagi-ndp-manifest-") as directory:
            target = Path(directory)
            (target / "sdk/rust").mkdir(parents=True)
            (target / "sdk/rust/Cargo.toml").write_text("# stand-in\n", encoding="utf-8")
            (target / "samples").mkdir()
            run_command(
                [sys.executable, "-B", str(GENERATOR), GENERATOR_ARGS[0], "--root", directory,
                 *GENERATOR_ARGS[1:]], root, MANIFEST_PHASE)
            written = target / "samples/level-zero"
            before = tree_digest(written)
            accepted, accept_ms = run_adapter(root, written, True)
            # Negative control in its own directory: a validator (or adapter) that accepts
            # anything must be caught. "BAD ID" is not a valid application id.
            control = target / "control"
            control.mkdir()
            try:
                manifest = json.loads((written / "manifest.json").read_text(encoding="utf-8"))
                manifest["id"] = "BAD ID"
                (control / "manifest.json").write_text(
                    json.dumps(manifest, ensure_ascii=False), encoding="utf-8")
            except (OSError, ValueError, TypeError):
                mismatch(MANIFEST_PHASE)
            rejected, _ = run_adapter(root, control, False)
            if tree_digest(written) != before:
                raise EvaluationError(MANIFEST_PHASE, "manifest_changed_by_validator")
        if first is not None and (accepted, rejected, before) != first:
            raise EvaluationError(MANIFEST_PHASE, "nondeterministic_output")
        first = (accepted, rejected, before)
        samples.append(accept_ms)
    report = json.loads(first[0])
    return {"status": "passed", "samples_ms": samples, "median_ms": statistics.median(samples),
            "stdout_identical": True, "tree_identical": True,
            "negative_control": "rejected", "sdk_contract": report["sdk_contract"],
            "validator": VALIDATOR.as_posix(),
            "validator_sha256": hashlib.sha256((root / VALIDATOR).read_bytes()).hexdigest(),
            "scope": "host_metadata_only"}


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
            stage2 = check_stage2(root, repeats)
            api_rules = check_api(root, repeats)
            generator = check_generator(root, repeats)
            xaml = check_xaml(root, repeats)
            manifest_validation = check_manifest_validation(root, repeats)
    finally:
        # Check even when a build, run or comparison fails. Never restore a changed input.
        if fingerprint(root) != before:
            raise EvaluationError("input_integrity", "source_changed")
    not_run = ["windows_input", "linux_input", "gtk_runtime", "xaml_runtime",
               "manual_comparison", "llm_comparison", "ui_rendering", "nagi_target",
               "scanner_output_to_api_rules", "generated_crate_build",
               "stage2_third_party_build", "stage3_nagi_target"]
    if manifest_validation.get("status") == "not_run":
        # Without a prepared validator the Manifest check is reported, never passed.
        not_run += ["sdk_validator", MANIFEST_PHASE]
    return {
        "status": "PASS", "scope": "synthetic_host_fixtures_only", "repeats": repeats,
        "environment": {"os": platform.system(), "machine": platform.machine(),
                        "python": platform.python_version(),
                        "compiler": version.decode("utf-8").splitlines()[0]},
        "portable_build_ms": build_ms, "control_syntax_ms": control_ms,
        "portable_run": portable, **ui_reports,
        "stage2_build_test": stage2, "api_rules": api_rules,
        "generator_scaffold": generator, **xaml,
        MANIFEST_PHASE: manifest_validation,
        "timing_scope": "subprocess wall time including startup; not human work or speedup",
        "inputs_unchanged": True,
        "not_run": not_run,
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
