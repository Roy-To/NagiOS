"""XAML (WPF / WinUI) rows of the NDP-07 mapping, against the real NDP-06 parser."""

import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
FIXTURES = ROOT / "tests/developer-platform/evaluation/ui"
XAML_FIXTURES = ("xaml_minimal.xaml", "wpf_task_panel.xaml", "winui_task_panel.xaml")
sys.path.insert(0, str(HERE))
import contract
import map_ui
sys.path.pop(0)
spec = importlib.util.spec_from_file_location("ndp06_mapping_xaml_test", HERE.parent / "analysis/analyze.py")
analyze = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analyze)

NAMESPACES = ('xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" '
              'xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"')


def fixture(name):
    return analyze.analyze_file(name, FIXTURES)


def xaml_report(inner, tag="Grid"):
    source = "<" + tag + " " + NAMESPACES + ">" + inner + "</" + tag + ">"
    return map_ui.map_analysis(analyze.analyze_bytes(source.encode("utf-8")))


def row_named(report, name, kind="widgets"):
    return next(r for r in report["rows"] if r["source_kind"] == kind and r["source"]["name"] == name)


class XamlMappingTests(unittest.TestCase):
    def test_wpf_fixture_candidates_and_axes(self):
        source = fixture("wpf_task_panel.xaml")
        result = map_ui.map_analysis(source)
        self.assertEqual("xaml-ui", result["source_format"])
        self.assertEqual(source, result["source_analysis"])
        expected = {"taskWindow": "ApplicationShellSpec", "grid": "LayoutKind::Grid",
                    "title": "ResolvedText", "body": "LayoutKind::Column",
                    "taskName": "ComponentKind::TextField", "addButton": "ComponentKind::Button"}
        for name, target in expected.items():
            with self.subTest(name=name):
                row = row_named(result, name)
                self.assertEqual([target], row["mapping"]["candidates"])
                self.assertEqual("PROVISIONAL", row["mapping"]["approval"])
        text = row_named(result, "taskName")
        self.assertEqual("partial", text["axes"]["accessibility"]["status"])
        self.assertEqual("partial", text["axes"]["focus"]["status"])  # TabIndex evidence
        self.assertEqual("partial", text["axes"]["localization"]["status"])
        self.assertIn("タスク名", json.dumps(text["source_metadata"], ensure_ascii=False))
        # No accessibility or tab evidence on the grid-level title: not invented.
        self.assertEqual("needs_decision", row_named(result, "title")["axes"]["focus"]["status"])
        self.assertEqual("needs_decision", row_named(result, "addButton")["axes"]["accessibility"]["status"])
        self.assertEqual("direct", row_named(result, "addButton")["mapping"]["relation"])

    def test_custom_control_is_not_mapped_through_its_name(self):
        result = map_ui.map_analysis(fixture("wpf_task_panel.xaml"))
        row = next(r for r in result["rows"] if r["source_kind"] == "widgets"
                   and r["source"]["name"] == "gauge")
        self.assertEqual("unknown", row["mapping"]["relation"])
        self.assertEqual("needs_decision", row["status"])
        self.assertEqual([], row["mapping"]["candidates"])
        self.assertTrue(any(r["source_kind"] == "custom_widgets" for r in result["rows"]))

    def test_unknown_namespace_style_like_elements_need_decision_and_map_to_nothing(self):
        # Pins current behavior (PROVISIONAL): non-visual elements of an unknown namespace
        # appear as custom widgets and get unmapped needs_decision rows, never a candidate.
        source = ("<Grid " + NAMESPACES + ' xmlns:v="urn:vendor" x:Name="root">'
                  '<v:Style TargetType="Button"><v:Setter Property="Width" Value="3"/></v:Style>'
                  '<Button x:Name="b"/></Grid>')
        result = map_ui.map_analysis(analyze.analyze_bytes(source.encode("utf-8")))
        for name in ("@xaml-object-2", "@xaml-object-3"):
            for kind in ("widgets", "unknown"):
                with self.subTest(name=name, kind=kind):
                    row = row_named(result, name, kind)
                    self.assertEqual("needs_decision", row["status"])
                    self.assertEqual("unknown", row["mapping"]["relation"])
                    self.assertEqual([], row["mapping"]["candidates"])
        for name in ("Style", "Setter"):
            with self.subTest(custom=name):
                row = next(r for r in result["rows"] if r["source_kind"] == "custom_widgets"
                           and r["source"]["class"] == name)
                self.assertEqual("needs_decision", row["status"])
                self.assertEqual([], row["mapping"]["candidates"])
                self.assertEqual("custom_widget_requires_owner_review", row["mapping"]["reason"])
        self.assertEqual(["ComponentKind::Button"], row_named(result, "b")["mapping"]["candidates"])
        self.assertFalse(result["summary"]["port_complete"])
        self.assertEqual("not_run", result["summary"]["target_acceptance"])

    def test_events_bindings_resources_unknown_and_unresolved_stay_unconnected(self):
        for name in ("wpf_task_panel.xaml", "winui_task_panel.xaml"):
            with self.subTest(name=name):
                result = map_ui.map_analysis(fixture(name))
                kinds = {r["source_kind"] for r in result["rows"]}
                self.assertTrue({"connections", "bindings", "resources"} <= kinds)
                for row in result["rows"]:
                    if row["source_kind"] in ("connections", "bindings", "resources"):
                        self.assertEqual("unsupported", row["axes"]["backend"]["status"])
                        self.assertEqual([], row["mapping"]["candidates"])
                        if "execution" in row["source"]:
                            self.assertEqual("not_run", row["source"]["execution"])
        winui = map_ui.map_analysis(fixture("winui_task_panel.xaml"))
        unresolved = [r for r in winui["rows"] if r["source_kind"] == "unresolved_references"]
        self.assertEqual(["AccentTextBrush"], [r["source"]["name"] for r in unresolved])
        self.assertTrue(all(r["status"] == "needs_decision" for r in unresolved))
        self.assertTrue(any(r["source_kind"] == "unknown" and r["source"].get("tag") == "DataTemplate"
                            for r in winui["rows"]))

    def test_stack_panel_direction_is_never_guessed(self):
        template = '<StackPanel x:Name="box"{}/>'
        vertical = xaml_report(template.format(' Orientation="Vertical"'))
        horizontal = xaml_report(template.format(' Orientation="Horizontal"'))
        missing = xaml_report(template.format(""))
        self.assertEqual(["LayoutKind::Column"], row_named(vertical, "box")["mapping"]["candidates"])
        self.assertEqual(["LayoutKind::Row"], row_named(horizontal, "box")["mapping"]["candidates"])
        self.assertEqual(["LayoutKind::Row"], row_named(horizontal, "box", "layouts")["mapping"]["candidates"])
        self.assertEqual("needs_decision", row_named(missing, "box")["mapping"]["status"])
        self.assertEqual("needs_decision", row_named(missing, "box", "layouts")["mapping"]["status"])

    def test_recognized_unsupported_and_unmapped_controls(self):
        for cls in ("Canvas", "DockPanel", "RelativePanel", "Image", "PasswordBox"):
            with self.subTest(cls=cls):
                row = row_named(xaml_report('<{} x:Name="x"/>'.format(cls)), "x")
                self.assertTrue(row["source"]["recognized"])
                self.assertEqual(("unavailable", "unsupported", []),
                                 (row["mapping"]["relation"], row["mapping"]["status"],
                                  row["mapping"]["candidates"]))
        # Recognized by the parser but without a rule: needs a decision, no invented target.
        row = row_named(xaml_report('<Slider x:Name="x"/>'), "x")
        self.assertTrue(row["source"]["recognized"])
        self.assertEqual(("unknown", "needs_decision", []),
                         (row["mapping"]["relation"], row["mapping"]["status"],
                          row["mapping"]["candidates"]))

    def test_rule_adapter_can_be_replaced_and_contract_drift_is_respected(self):
        source = fixture("wpf_task_panel.xaml")
        rules = json.loads((HERE / "rules.json").read_text())
        del rules["xaml-ui"]["widgets"]["TextBox"]
        replaced = map_ui.map_analysis(source, rules=rules)
        self.assertEqual("needs_decision", row_named(replaced, "taskName")["mapping"]["status"])
        self.assertEqual(source, replaced["source_analysis"])
        drifted = copy.deepcopy(contract.inspect_contract())
        drifted["state"] = "needs_review"
        report = map_ui.map_analysis(source, contract=drifted)
        row = row_named(report, "addButton")
        self.assertEqual("needs_decision", row["mapping"]["status"])
        for axis in map_ui.AXES[:-1]:
            self.assertEqual("needs_decision", row["axes"][axis]["status"])

    def test_every_xaml_rule_matches_a_class_the_parser_recognizes(self):
        rules = json.loads((HERE / "rules.json").read_text())["xaml-ui"]
        self.assertTrue(set(rules["widgets"]) <= analyze.XAML_CONTROLS)
        self.assertTrue(set(rules["layouts"]) <= analyze.XAML_LAYOUTS)

    def test_deterministic_independent_and_row_status_derivation(self):
        for name in XAML_FIXTURES:
            with self.subTest(name=name):
                source = fixture(name)
                before = copy.deepcopy(source)
                reports = [map_ui.json_report(map_ui.map_analysis(source)) for _ in range(3)]
                self.assertTrue(all(r == reports[0] for r in reports))
                self.assertEqual(before, source)
                reordered = dict(reversed(list(source.items())))
                self.assertEqual(reports[0], map_ui.json_report(map_ui.map_analysis(reordered)))
                result = map_ui.map_analysis(source)
                row_named(result, result["source_analysis"]["widgets"][0]["name"])["source"]["properties"].clear()
                self.assertEqual(before, source)
                for row in result["rows"]:
                    self.assertEqual(map_ui._row_status(row["mapping"]["status"]), row["status"])
                self.assertFalse(result["summary"]["port_complete"])
                self.assertEqual("not_run", result["summary"]["target_acceptance"])
                self.assertEqual(len(result["rows"]), sum(result["summary"]["status_counts"].values()))

    def test_unicode_is_preserved_and_markdown_escapes_source_text(self):
        result = map_ui.map_analysis(fixture("xaml_minimal.xaml"))
        self.assertIn("こんにちは 🌊", map_ui.json_report(result))
        escaped = xaml_report('<Button x:Name="&lt;script&gt;|x`" Click="never_run"/>')
        output = map_ui.markdown_report(escaped)
        self.assertNotIn("<script>", output)
        self.assertIn("&lt;script&gt;&#124;x&#96;", output)
        for axis in map_ui.AXES:
            self.assertIn(axis, output)
        self.assertIn("declared_event_requires_typed_app_adapter", output)

    def test_real_analyzer_cli_to_mapper_cli_and_source_unchanged(self):
        env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1")
        for name in XAML_FIXTURES:
            with self.subTest(name=name):
                path = FIXTURES / name
                before = hashlib.sha256(path.read_bytes()).hexdigest()
                analysis = subprocess.run(
                    [sys.executable, str(HERE.parent / "analysis/analyze.py"), "--root",
                     str(FIXTURES), name], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                    timeout=10, env=env)
                self.assertEqual(0, analysis.returncode, analysis.stderr)
                outputs = [subprocess.run([sys.executable, str(HERE / "map_ui.py")],
                                          input=analysis.stdout, stdout=subprocess.PIPE,
                                          stderr=subprocess.PIPE, timeout=10, env=env)
                           for _ in range(2)]
                self.assertEqual(0, outputs[0].returncode, outputs[0].stderr)
                self.assertEqual(outputs[0].stdout, outputs[1].stdout)
                self.assertEqual(json.loads(analysis.stdout),
                                 json.loads(outputs[0].stdout)["source_analysis"])
                self.assertEqual(before, hashlib.sha256(path.read_bytes()).hexdigest())


if __name__ == "__main__":
    unittest.main()
