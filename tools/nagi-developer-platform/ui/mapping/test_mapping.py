"""Host report tests against the real NDP-06 parser and repository fixtures."""

import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
FIXTURES = ROOT / "tests/developer-platform/evaluation/ui"
sys.path.insert(0, str(HERE))
import contract
import map_ui
sys.path.pop(0)
spec = importlib.util.spec_from_file_location("ndp06_mapping_test", HERE.parent / "analysis/analyze.py")
analyze = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analyze)


def fixture(name="task_panel.ui"):
    return analyze.analyze_file(name, FIXTURES)


def xml_report(xml):
    return map_ui.map_analysis(analyze.analyze_bytes(xml.encode("utf-8")))


def row_named(report, name, kind="widgets"):
    return next(r for r in report["rows"] if r["source_kind"] == kind and r["source"]["name"] == name)


class MappingTests(unittest.TestCase):
    def test_every_rule_candidate_has_real_contract_evidence(self):
        rules = json.loads((HERE / "rules.json").read_text())
        evidence = contract.inspect_contract()["evidence"]
        paths = {item["path"] for item in evidence}
        rust = "\n".join((ROOT / path).read_text() for path in sorted(paths) if path.endswith(".rs"))
        for groups in rules.values():
            for classes in groups.values():
                for cls, rule in classes.items():
                    with self.subTest(cls=cls):
                        self.assertTrue(set(rule["refs"]).issubset(paths))
                        for symbol in rule["targets"] + rule["models"]:
                            if "::" in symbol:
                                enum_name, variant = symbol.split("::")
                                body = re.search(r"pub enum " + enum_name + r"\s*\{(.*?)\n\}", rust, re.S)
                                self.assertIsNotNone(body, symbol)
                                self.assertRegex(body.group(1), r"\b" + variant + r"\b")
                            else:
                                self.assertRegex(rust, r"pub struct " + symbol + r"\b")

    def test_current_contract_evidence_matches(self):
        state = contract.inspect_contract()
        self.assertEqual("matched", state["state"])
        self.assertEqual("0.1.0", state["expected_version"])
        self.assertTrue(all(e["sha256"] == e["actual_sha256"] for e in state["evidence"]))

    def test_qt_real_fixture_candidates_and_gaps(self):
        result = map_ui.map_analysis(fixture())
        self.assertEqual("qt-designer-ui", result["source_format"])
        button = row_named(result, "addButton")
        self.assertEqual(["ComponentKind::Button"], button["mapping"]["candidates"])
        self.assertEqual("supported", button["mapping"]["status"])
        self.assertEqual("direct", button["mapping"]["relation"])
        self.assertEqual("PROVISIONAL", button["mapping"]["approval"])
        self.assertEqual("partial", button["status"])
        self.assertEqual("partial", button["axes"]["pointer_keyboard"]["status"])
        self.assertEqual("partial", button["axes"]["focus"]["status"])
        self.assertEqual("needs_decision", button["axes"]["accessibility"]["status"])
        text = row_named(result, "taskName")
        self.assertEqual("partial", text["axes"]["accessibility"]["status"])
        self.assertIn("タスク名", json.dumps(text["source_metadata"], ensure_ascii=False))
        layout = row_named(result, "layout", "layouts")
        self.assertEqual(["LayoutKind::Column"], layout["mapping"]["candidates"])
        self.assertEqual("redesign", layout["mapping"]["relation"])

    def test_gtk_real_fixtures_and_accessibility_ownership(self):
        for name in ("gtk3_task_panel.ui", "gtk4_task_panel.ui", "gtk_minimal.ui"):
            with self.subTest(name=name):
                source = fixture(name)
                result = map_ui.map_analysis(source)
                self.assertEqual(source, result["source_analysis"])
                self.assertEqual("gtk-builder-ui", result["source_format"])
                if name == "gtk_minimal.ui":
                    button = row_named(result, "hello")
                    self.assertEqual("needs_decision", button["axes"]["focus"]["status"])
                    self.assertIn("こんにちは 🌊", map_ui.json_report(result))
                else:
                    text = row_named(result, "taskName")
                    self.assertEqual("partial", text["axes"]["accessibility"]["status"])
                    self.assertTrue(text["source_metadata"]["accessibility"])
                    self.assertTrue(any(r["source_kind"] == "bindings" for r in result["rows"]))
        gtk3 = map_ui.map_analysis(fixture("gtk3_task_panel.ui"))
        self.assertEqual("needs_decision", row_named(gtk3, "entryAccessible")["mapping"]["status"])
        self.assertEqual(["LayoutKind::Column"], row_named(gtk3, "body", "layouts")["mapping"]["candidates"])

    def test_events_bindings_resources_and_slots_stay_unconnected(self):
        for name in ("task_panel.ui", "gtk3_task_panel.ui", "gtk4_task_panel.ui"):
            result = map_ui.map_analysis(fixture(name))
            gaps = [r for r in result["rows"] if r["source_kind"] in
                    ("connections", "bindings", "resources", "declared_slots")]
            self.assertTrue(gaps)
            for row in gaps:
                self.assertEqual("unsupported", row["axes"]["backend"]["status"])
                self.assertEqual([], row["mapping"]["candidates"])
                if isinstance(row["source"], dict) and "execution" in row["source"]:
                    self.assertEqual("not_run", row["source"]["execution"])

    def test_missing_binding_is_not_invented(self):
        result = map_ui.map_analysis(fixture())
        self.assertFalse(any(r["source_kind"] == "bindings" for r in result["rows"]))
        self.assertEqual("unsupported", row_named(result, "taskName")["axes"]["backend"]["status"])

    def test_unknown_custom_widget_is_not_mapped_through_base_class(self):
        result = xml_report('''<ui version="4.0"><widget class="VendorButton" name="custom"/>
            <customwidgets><customwidget><class>VendorButton</class><extends>QPushButton</extends>
            <header>never_load.h</header></customwidget></customwidgets></ui>''')
        row = row_named(result, "custom")
        self.assertEqual("unknown", row["mapping"]["relation"])
        self.assertEqual("needs_decision", row["status"])
        self.assertEqual([], row["mapping"]["candidates"])
        self.assertTrue(any(r["source_kind"] == "custom_widgets" for r in result["rows"]))

    def test_recognized_unsupported_controls(self):
        for cls in ("GtkImage", "GtkTextView", "GtkSpinButton", "GtkFixed", "GtkPaned", "GtkStack"):
            with self.subTest(cls=cls):
                result = xml_report('<interface><object class="{}" id="x"/></interface>'.format(cls))
                row = row_named(result, "x")
                self.assertTrue(row["source"]["recognized"])
                self.assertEqual("unavailable", row["mapping"]["relation"])
                self.assertEqual("unsupported", row["mapping"]["status"])
                self.assertEqual([], row["mapping"]["candidates"])

    def test_unknown_elements_and_unresolved_references_are_reported(self):
        result = xml_report('''<ui version="4.0"><widget class="QPushButton" name="button">
            <vendor-extension/></widget><tabstops><tabstop>missing</tabstop></tabstops></ui>''')
        for kind in ("unknown", "unresolved_references"):
            rows = [r for r in result["rows"] if r["source_kind"] == kind]
            self.assertTrue(rows)
            self.assertTrue(all(r["status"] == "needs_decision" for r in rows))

    def test_long_japanese_and_english_are_preserved_without_layout_claim(self):
        for text in ("長い日本語の操作説明🌊" * 200, "A long English control label " * 200):
            source = analyze.analyze_bytes(('<interface><object class="GtkButton" id="x">'
                '<property name="label" translatable="yes" context="panel">' + text +
                '</property></object></interface>').encode())
            report = map_ui.map_analysis(source)
            self.assertIn(text, map_ui.json_report(report))
            row = row_named(report, "x")
            self.assertEqual("partial", row["axes"]["appearance_layout"]["status"])
            self.assertEqual("partial", row["axes"]["localization"]["status"])
            values = row["source_metadata"]["text"][0]["values"]
            self.assertEqual("panel", values[0]["attributes"]["context"])

    def test_input_and_nested_output_are_independent(self):
        source = fixture("gtk4_task_panel.ui")
        before = copy.deepcopy(source)
        first = map_ui.map_analysis(source)
        self.assertEqual(before, source)
        row_named(first, "taskName")["source"]["properties"].clear()
        first["source_analysis"]["widgets"].clear()
        self.assertEqual(before, source)
        self.assertTrue(map_ui.map_analysis(source)["source_analysis"]["widgets"])

    def test_deterministic_serialization_and_dictionary_order(self):
        for name in sorted(p.name for p in FIXTURES.glob("*.ui")):
            source = fixture(name)
            reports = [map_ui.json_report(map_ui.map_analysis(source)) for _ in range(3)]
            self.assertTrue(all(r == reports[0] for r in reports))
            reversed_keys = dict(reversed(list(source.items())))
            self.assertEqual(reports[0], map_ui.json_report(map_ui.map_analysis(reversed_keys)))

    def test_layout_changes_are_visible_and_orientation_is_not_guessed(self):
        template = '<interface><object class="GtkBox" id="box">{}</object></interface>'
        vertical = xml_report(template.format('<property name="orientation">vertical</property>'))
        horizontal = xml_report(template.format('<property name="orientation">horizontal</property>'))
        self.assertEqual(["LayoutKind::Column"], row_named(vertical, "box")["mapping"]["candidates"])
        self.assertEqual(["LayoutKind::Row"], row_named(horizontal, "box")["mapping"]["candidates"])
        absent = xml_report(template.format(""))
        self.assertEqual("needs_decision", row_named(absent, "box")["mapping"]["status"])
        source = fixture("gtk4_task_panel.ui")
        changed = copy.deepcopy(source)
        # GTK4 grid position evidence is carried in each child's layout property.
        entry = next(w for w in changed["widgets"] if w["name"] == "taskName")
        prop = next(p for p in entry["properties"] if p["name"] == "layout")
        prop["values"][1]["text"] = "999"
        self.assertNotEqual(map_ui.json_report(map_ui.map_analysis(source)),
                            map_ui.json_report(map_ui.map_analysis(changed)))

    def test_rule_adapter_can_be_replaced_without_changing_input(self):
        source = fixture()
        rules = json.loads((HERE / "rules.json").read_text())
        del rules["qt-designer-ui"]["widgets"]["QPushButton"]
        result = map_ui.map_analysis(source, rules=rules)
        self.assertEqual("needs_decision", row_named(result, "addButton")["mapping"]["status"])
        self.assertEqual(source, result["source_analysis"])

    def test_row_status_is_derived_from_mapping_status(self):
        # Row status is the whole-port state; mapping.status is vocabulary match only.
        for name in sorted(p.name for p in FIXTURES.glob("*.ui")):
            result = map_ui.map_analysis(fixture(name))
            for row in result["rows"]:
                with self.subTest(fixture=name, row=row["id"]):
                    self.assertEqual(map_ui._row_status(row["mapping"]["status"]), row["status"])
                    self.assertIn(row["status"], ("partial", "unsupported", "needs_decision"))
        self.assertEqual("partial", map_ui._row_status("supported"))
        self.assertEqual("partial", map_ui._row_status("partial"))
        self.assertEqual("unsupported", map_ui._row_status("unsupported"))
        self.assertEqual("needs_decision", map_ui._row_status("needs_decision"))

    def test_summary_counts_all_rows_and_never_claims_target_acceptance(self):
        result = map_ui.map_analysis(fixture("gtk4_task_panel.ui"))
        self.assertFalse(result["summary"]["port_complete"])
        self.assertEqual("not_run", result["summary"]["target_acceptance"])
        self.assertEqual(len(result["rows"]), sum(result["summary"]["status_counts"].values()))
        for axis in map_ui.AXES:
            self.assertEqual(len(result["rows"]), sum(result["summary"]["axis_status_counts"][axis].values()))
        self.assertTrue(all(r["axes"]["backend"]["status"] == "unsupported" for r in result["rows"]))
        self.assertTrue(all(r["axes"][a]["verification"] == "not_run"
                            for r in result["rows"] for a in map_ui.AXES))

    def test_markdown_includes_all_axes_gaps_and_safe_source_text(self):
        result = xml_report('<interface><object class="GtkButton" id="&lt;script&gt;|x`">'
                            '<signal name="clicked" handler="never_run"/></object></interface>')
        output = map_ui.markdown_report(result)
        self.assertNotIn('<script>', output)
        self.assertIn('&lt;script&gt;&#124;x&#96;', output)
        for axis in map_ui.AXES:
            self.assertIn(axis, output)
        self.assertIn("declared_event_requires_typed_app_adapter", output)
        self.assertIn("PROVISIONAL", output)
        self.assertIn("not_run", output)


class EvidenceDriftTests(unittest.TestCase):
    def _copy_evidence(self, repo):
        manifest = json.loads((HERE / "contract_snapshot.json").read_text())
        for item in manifest["files"]:
            dst = repo / item["path"]
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / item["path"], dst)

    def test_unmodified_evidence_copy_matches(self):
        # Control for the drift cases: the copy harness itself must not cause drift.
        with tempfile.TemporaryDirectory(prefix="ndp07-evidence-") as temp:
            repo = Path(temp)
            self._copy_evidence(repo)
            state = contract.inspect_contract(repo)
            self.assertEqual("matched", state["state"])
            self.assertEqual({"matched"}, {e["state"] for e in state["evidence"]})

    def test_added_source_file_is_detected_and_reaches_the_report(self):
        with tempfile.TemporaryDirectory(prefix="ndp07-evidence-") as temp:
            repo = Path(temp)
            self._copy_evidence(repo)
            (repo / "user/nagi-ui/src/new_module.rs").write_text("pub struct Added;\n")
            (repo / "user/nagi-ui/src/notes.txt").write_text("not a Rust source\n")
            state = contract.inspect_contract(repo)
            self.assertEqual("needs_review", state["state"])
            unlisted = [e for e in state["evidence"] if e["state"] == "unlisted"]
            self.assertEqual(["user/nagi-ui/src/new_module.rs"], [e["path"] for e in unlisted])
            self.assertIsNone(unlisted[0]["sha256"])
            self.assertEqual(hashlib.sha256(b"pub struct Added;\n").hexdigest(),
                             unlisted[0]["actual_sha256"])
            report = map_ui.map_analysis(fixture(), contract=state)
            self.assertEqual("needs_decision", row_named(report, "addButton")["mapping"]["status"])
            markdown = map_ui.markdown_report(report)
            self.assertIn("unlisted (SHA-256 not_in_snapshot)", markdown)

    def _drift(self, mode):
        with tempfile.TemporaryDirectory(prefix="ndp07-evidence-") as temp:
            repo = Path(temp)
            self._copy_evidence(repo)
            if mode == "version":
                path = repo / "user/nagi-ui/Cargo.toml"
                path.write_text(path.read_text().replace('version = "0.1.0"', 'version = "9.0.0"'))
            elif mode == "missing":
                (repo / "user/nagi-ui/src/interaction.rs").unlink()
            else:
                path = repo / "user/nagi-ui/src/interaction.rs"
                path.write_text(path.read_text().replace("    Button,", "    RenamedButton,"))
            state = contract.inspect_contract(repo)
            self.assertEqual("needs_review", state["state"])
            result = map_ui.map_analysis(fixture(), contract=state)
            row = row_named(result, "addButton")
            self.assertEqual("needs_decision", row["mapping"]["status"])
            for axis in map_ui.AXES[:-1]:
                self.assertEqual("needs_decision", row["axes"][axis]["status"])

    def test_package_version_drift(self):
        self._drift("version")

    def test_missing_evidence(self):
        self._drift("missing")

    def test_removed_candidate_api(self):
        self._drift("api")


class CliAndValidationTests(unittest.TestCase):
    def run_cli(self, data, *args):
        return subprocess.run([sys.executable, str(HERE / "map_ui.py")] + list(args),
                              input=data, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              env=dict(os.environ, PYTHONDONTWRITEBYTECODE="1"), timeout=10)

    def test_real_analyzer_cli_to_mapper_both_formats_and_source_unchanged(self):
        for path in sorted(FIXTURES.glob("*.ui")):
            with self.subTest(fixture=path.name):
                before = hashlib.sha256(path.read_bytes()).hexdigest()
                analysis = subprocess.run([sys.executable, str(HERE.parent / "analysis/analyze.py"),
                                          "--root", str(FIXTURES), path.name], stdout=subprocess.PIPE,
                                         stderr=subprocess.PIPE, timeout=10,
                                         env=dict(os.environ, PYTHONDONTWRITEBYTECODE="1"))
                self.assertEqual(0, analysis.returncode, analysis.stderr)
                first, second = self.run_cli(analysis.stdout), self.run_cli(analysis.stdout)
                self.assertEqual(0, first.returncode, first.stderr)
                self.assertEqual(b"", first.stderr)
                self.assertEqual(first.stdout, second.stdout)
                self.assertEqual(json.loads(analysis.stdout), json.loads(first.stdout)["source_analysis"])
                md = self.run_cli(analysis.stdout, "--format", "markdown")
                self.assertEqual(0, md.returncode, md.stderr)
                self.assertIn(b"PROVISIONAL", md.stdout)
                self.assertEqual(before, hashlib.sha256(path.read_bytes()).hexdigest())

    def test_malformed_json_diagnostics(self):
        for data, code in ((b'{"secret":"private-input"', "invalid_analysis_json"),
                           (b'\xff', "invalid_analysis_json"),
                           (b'{"a":1,"a":2}', "duplicate_json_key"),
                           (b'x' * (map_ui.MAX_INPUT_BYTES + 1), "input_too_large")):
            result = self.run_cli(data)
            self.assertEqual(2, result.returncode)
            self.assertEqual(b"", result.stdout)
            self.assertEqual({"error": code}, json.loads(result.stderr))

    def test_bad_schema_fails_closed(self):
        cases = []
        source = fixture()
        for key, value, code in (("analysis_version", 2, "unsupported_analysis_version"),
                                 ("analysis_version", True, "unsupported_analysis_version"),
                                 ("format", "other", "unsupported_analysis_format"),
                                 ("widgets", None, "invalid_analysis"),
                                 ("connections", ["bad"], "invalid_analysis")):
            bad = copy.deepcopy(source)
            bad[key] = value
            cases.append((bad, code))
        bad = copy.deepcopy(source)
        del bad["bindings"]
        cases.append((bad, "invalid_analysis"))
        bad = copy.deepcopy(source)
        bad["widgets"][0]["children"] = [{}]
        cases.append((bad, "invalid_analysis"))
        for bad, code in cases:
            with self.subTest(code=code), self.assertRaises(map_ui.MappingError) as error:
                map_ui.map_analysis(bad)
            self.assertEqual(code, error.exception.code)

    def test_recursive_or_non_json_input_is_rejected(self):
        source = fixture()
        source["cycle"] = source
        with self.assertRaisesRegex(map_ui.MappingError, "analysis_limit"):
            map_ui.map_analysis(source)
        source = fixture()
        source["non_json"] = object()
        with self.assertRaisesRegex(map_ui.MappingError, "invalid_analysis"):
            map_ui.map_analysis(source)


if __name__ == "__main__":
    unittest.main()
