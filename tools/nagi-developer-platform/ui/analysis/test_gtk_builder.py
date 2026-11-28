"""Real bounded-parser and CLI checks against original synthetic GTK inputs."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import analyze


FIXTURES = Path(__file__).resolve().parents[4] / "tests/developer-platform/evaluation/ui"


def builder(inner="", extra="", kind="GtkWindow", identity='id="window"'):
    return ('<interface><object class="' + kind + '" ' + identity + '>' + inner
            + '</object>' + extra + '</interface>').encode("utf-8")


def properties(item):
    return {prop["name"]: prop["values"] for prop in item["properties"]}


class GtkBuilderAnalysisTests(unittest.TestCase):
    def assert_error(self, source, code):
        before = bytes(source)
        with self.assertRaises(analyze.AnalysisError) as caught:
            analyze.analyze_bytes(source)
        self.assertEqual(caught.exception.code, code)
        self.assertEqual(source, before)

    def temporary_directory(self):
        # Default system temp location, never the source tree.
        return tempfile.TemporaryDirectory(prefix="ndp06-gtk-")

    def test_minimal_screen_unicode_and_translation_attributes(self):
        result = analyze.analyze_file("gtk_minimal.ui", FIXTURES)
        self.assertEqual(result["format"], "gtk-builder-ui")
        self.assertEqual(result["form_class"], "")
        self.assertEqual(len(result["widgets"]), 1)
        widget = result["widgets"][0]
        self.assertEqual((widget["name"], widget["class"]), ("hello", "GtkButton"))
        self.assertTrue(widget["recognized"])
        self.assertIsNone(widget["parent_widget"])
        label = properties(widget)["label"][0]
        self.assertEqual(label["text"], "Hello / こんにちは 🌊")
        self.assertEqual(label["attributes"]["translatable"], "yes")
        for key in ("layouts", "unknown", "connections", "bindings", "tab_order",
                    "unresolved_references"):
            self.assertEqual(result[key], [])

    def test_interface_translation_domain_and_nested_text_evidence(self):
        source = builder('<property name="title" translatable="yes" context="window" '
                         'comments="Translator hint">日本語</property>')
        source = source.replace(b"<interface>", b'<interface domain="synthetic-app">')
        result = analyze.analyze_bytes(source)
        self.assertEqual(result["attributes"], {"domain": "synthetic-app"})
        title = properties(result["widgets"][0])["title"][0]
        self.assertEqual(title["text"], "日本語")
        self.assertEqual(title["attributes"], {"name": "title", "translatable": "yes",
                                              "context": "window", "comments": "Translator hint"})

    def test_gtk3_nested_hierarchy_packing_and_accessibility(self):
        result = analyze.analyze_file("gtk3_task_panel.ui", FIXTURES)
        self.assertEqual([item["name"] for item in result["widgets"]],
                         ["taskWindow", "body", "title", "taskName", "entryAccessible",
                          "addButton"])
        widgets = {item["name"]: item for item in result["widgets"]}
        self.assertEqual(widgets["body"]["parent_widget"], "taskWindow")
        self.assertEqual(widgets["taskName"]["parent_widget"], "body")
        self.assertEqual(widgets["taskName"]["parent_layout"], "body")
        self.assertEqual(widgets["entryAccessible"]["parent_widget"], "taskName")
        self.assertIsNone(widgets["entryAccessible"]["parent_layout"])
        self.assertEqual(widgets["taskName"]["children"][0]["attributes"],
                         {"internal-child": "accessible"})
        accessible = properties(widgets["entryAccessible"])["accessible-name"][0]
        self.assertEqual(accessible["text"], "タスク名")
        layout = result["layouts"][0]
        self.assertEqual((layout["name"], layout["class"], layout["parent_widget"]),
                         ("body", "GtkBox", "body"))
        self.assertEqual(properties(layout)["orientation"][0]["text"], "vertical")
        self.assertEqual([x["objects"][0]["name"] for x in layout["items"]],
                         ["title", "taskName", "addButton"])
        packing = {x["name"]: x["values"][0]["text"]
                   for x in layout["items"][1]["packing"]}
        self.assertEqual(packing, {"expand": "True", "position": "1"})
        self.assertEqual(result["unknown"],
                         [{"kind": "widget", "name": "entryAccessible", "class": "AtkObject"}])
        self.assertEqual(result["unresolved_references"], [])

    def test_gtk4_layout_binding_and_accessibility(self):
        result = analyze.analyze_file("gtk4_task_panel.ui", FIXTURES)
        entry = result["widgets"][3]
        self.assertEqual((entry["name"], entry["parent_widget"], entry["parent_layout"]),
                         ("taskName", "grid", "grid"))
        self.assertEqual([x["name"] for x in result["layouts"]], ["grid"])
        item = result["layouts"][0]["items"][2]["objects"][0]
        placement = properties(item)["layout"]
        self.assertEqual({x["attributes"]["name"]: x["text"] for x in placement[1:]},
                         {"column": "1", "row": "1", "column-span": "2"})
        accessibility = properties(entry)["accessibility"]
        self.assertEqual([(x["kind"], x["text"]) for x in accessibility[1:]],
                         [("property", "タスク名 🌊"), ("relation", "title"),
                          ("state", "false")])
        self.assertEqual(accessibility[1]["attributes"]["translatable"], "yes")
        binding = result["bindings"][0]
        self.assertEqual(binding["owner"], "taskName")
        self.assertEqual(binding["content"]["attributes"], {"name": "text"})
        self.assertEqual(binding["content"]["children"][0]["attributes"], {"name": "label"})
        self.assertEqual(binding["content"]["children"][0]["text"], "title")
        self.assertEqual(binding["execution"], "not_run")
        self.assertEqual(result["unknown"], [])

    def test_layout_changes_affect_report_for_both_versions(self):
        for filename, old, new in (
            ("gtk3_task_panel.ui", b'<property name="position">1',
             b'<property name="position">5'),
            ("gtk4_task_panel.ui", b'<property name="column-span">2',
             b'<property name="column-span">3'),
        ):
            with self.subTest(filename=filename):
                source = (FIXTURES / filename).read_bytes()
                self.assertIn(old, source)
                changed = source.replace(old, new)
                first = analyze.analyze_bytes(source)
                second = analyze.analyze_bytes(changed)
                self.assertNotEqual(first["layouts"], second["layouts"])

    def test_nested_layouts_multiple_roots_and_property_objects(self):
        source = builder(
            '<child><object class="GtkBox" id="box"><child>'
            '<object class="GtkGrid" id="grid"><child>'
            '<object class="GtkEntry" id="entry"/></child></object></child>'
            '<property name="model"><object class="VendorModel" id="model"/>'
            '</property></object></child>',
            '<object class="GtkWindow" id="second"/>')
        result = analyze.analyze_bytes(source)
        self.assertEqual(result["layouts"][1]["parent_layout"], "box")
        widgets = {item["name"]: item for item in result["widgets"]}
        self.assertEqual(widgets["entry"]["parent_layout"], "grid")
        self.assertEqual(widgets["model"]["parent_widget"], "box")
        self.assertIsNone(widgets["model"]["parent_layout"])
        self.assertIsNone(widgets["second"]["parent_widget"])

    def test_unknown_class_elements_and_layout_manager_preserve_evidence(self):
        source = builder(
            '<child type="custom"><object class="VendorGauge" id="gauge">'
            '<property name="label">測定器</property>'
            '<vendor-data key="unit"><scale factor="2">μm</scale></vendor-data>'
            '<property name="layout-manager"><object class="GtkGridLayout" id="manager"/>'
            '</property></object><packing><property name="position">3</property>'
            '</packing></child>', kind="GtkBox")
        result = analyze.analyze_bytes(source)
        self.assertFalse(result["widgets"][1]["recognized"])
        self.assertEqual(properties(result["widgets"][1])["label"][0]["text"], "測定器")
        self.assertIn({"kind": "widget", "name": "gauge", "class": "VendorGauge"},
                      result["unknown"])
        self.assertIn({"kind": "widget", "name": "manager", "class": "GtkGridLayout"},
                      result["unknown"])
        evidence = next(x["content"] for x in result["unknown"] if x.get("tag") == "vendor-data")
        self.assertEqual(evidence["attributes"], {"key": "unit"})
        self.assertEqual(evidence["children"][0]["attributes"], {"factor": "2"})
        self.assertEqual(evidence["children"][0]["text"], "μm")
        self.assertEqual(result["layouts"][0]["items"][0]["attributes"], {"type": "custom"})

    def test_signal_names_handlers_and_receiver_are_declarations_only(self):
        source = builder('<signal name="notify::title" handler="on_title" after="yes"/>'
                         '<signal name="destroy" handler="on_destroy" object="missing" '
                         'swapped="yes"/>')
        result = analyze.analyze_bytes(source)
        self.assertEqual([(x["sender"], x["signal"], x["receiver"], x["slot"], x["execution"])
                          for x in result["connections"]],
                         [("window", "notify::title", "", "on_title", "not_run"),
                          ("window", "destroy", "missing", "on_destroy", "not_run")])
        self.assertEqual(result["connections"][0]["attributes"]["after"], "yes")
        self.assertEqual(result["connections"][1]["attributes"]["swapped"], "yes")
        self.assertEqual(result["unresolved_references"], [{"kind": "receiver", "name": "missing"}])

    def test_property_binding_attributes_and_unresolved_source(self):
        result = analyze.analyze_bytes(builder(
            '<property name="title" bind-source="missing" bind-property="label" '
            'bind-flags="sync-create|bidirectional">初期値</property>'))
        binding = result["bindings"][0]
        self.assertEqual(binding["owner"], "window")
        self.assertEqual(binding["text"], "初期値")
        self.assertEqual(binding["content"]["attributes"]["bind-flags"],
                         "sync-create|bidirectional")
        self.assertEqual(result["unresolved_references"],
                         [{"kind": "binding_source", "name": "missing"}])

    def test_resources_expressions_scripts_and_handlers_never_run(self):
        source = builder(
            '<property name="file">file:///not-readable/secret</property>'
            '<property name="resource">https://invalid.example/picture</property>'
            '<signal name="destroy" handler="__import__(os).system(unsafe)"/>'
            '<binding name="title"><closure function="unsafe">state.title</closure></binding>'
            '<script>raise RuntimeError("must not run")</script>',
            '<requires lib="vendor-code" version="1.0"/>'
            '<include filename="https://invalid.example/code.ui"/>')
        forbidden = AssertionError("source must remain inert")
        with patch("builtins.open", side_effect=forbidden), \
                patch("os.open", side_effect=forbidden), \
                patch("socket.socket", side_effect=forbidden), \
                patch("urllib.request.urlopen", side_effect=forbidden), \
                patch("subprocess.run", side_effect=forbidden), \
                patch("os.system", side_effect=forbidden), \
                patch("builtins.eval", side_effect=forbidden), \
                patch("builtins.exec", side_effect=forbidden):
            result = analyze.analyze_bytes(source)
        self.assertEqual([x["value"] for x in result["resources"]],
                         ["file:///not-readable/secret", "https://invalid.example/picture",
                          "vendor-code", "https://invalid.example/code.ui"])
        self.assertEqual(result["connections"][0]["execution"], "not_run")
        self.assertEqual(result["bindings"][0]["execution"], "not_run")
        self.assertIn("script", [x.get("tag") for x in result["unknown"]])

    def test_optional_ids_are_stable_and_cannot_alias_source_ids(self):
        source = builder('<child><object class="GtkButton"/></child>',
                         '<object class="GtkLabel" id="@gtk-object-1"/>', identity="")
        result = analyze.analyze_bytes(source)
        self.assertEqual(result, analyze.analyze_bytes(source))
        first, second, explicit = result["widgets"]
        self.assertEqual(first["name"], "@gtk-object-1_")
        self.assertEqual(second["parent_widget"], first["name"])
        self.assertEqual(first["children"][0]["objects"][0]["name"], second["name"])
        self.assertEqual(explicit["name"], "@gtk-object-1")
        self.assertNotIn("id", first["attributes"])

    def test_duplicate_or_invalid_identity_and_incomplete_signals(self):
        self.assert_error(builder(extra='<object class="GtkButton" id="window"/>'),
                          "duplicate_object_name")
        self.assert_error(builder('<child><object id="missingClass"/></child>'),
                          "missing_object_identity")
        self.assert_error(builder(identity='id=""'), "missing_object_identity")
        for signal in ('<signal/>', '<signal name="clicked"/>', '<signal handler="callback"/>'):
            self.assert_error(builder(signal), "incomplete_connection")
        self.assert_error(builder(extra='<signal name="clicked" handler="callback"/>'),
                          "incomplete_connection")

    def test_malformed_unsupported_and_empty_documents(self):
        for source in (b"<interface>", b'<interface><object></interface>',
                       builder() + b"<extra/>"):
            self.assert_error(source, "invalid_xml")
        self.assert_error(b'<builder><object class="GtkWindow"/></builder>', "unsupported_format")
        self.assert_error(b'<interface/>', "missing_root_widget")
        self.assert_error(b'<interface><template class="Custom" parent="GtkBox"/></interface>',
                          "missing_root_widget")

    def test_utf8_bom_entities_and_invalid_encodings(self):
        source = builder('<property name="title">日本語 &amp; &#x1F30A;</property>')
        result = analyze.analyze_bytes(b"\xef\xbb\xbf" + source)
        self.assertEqual(properties(result["widgets"][0])["title"][0]["text"], "日本語 & 🌊")
        self.assert_error(b"\xff" + source, "invalid_utf8")
        self.assert_error(source.replace(b"</interface>", b"\x00</interface>"), "invalid_utf8")
        for encoding in ("utf-16", "utf-16-le", "utf-16-be", "utf-32"):
            with self.subTest(encoding=encoding):
                self.assert_error(source.decode().encode(encoding), "invalid_utf8")

    def test_dtd_entity_and_processing_instruction_rejection(self):
        for declaration in (
            '<!DOCTYPE interface SYSTEM "file:///private-data">',
            '<!DOCTYPE interface [<!ENTITY ext SYSTEM "https://invalid.example/entity">]>',
            '<!DOCTYPE interface [<!ENTITY x "expanded">]>',
            '<!ENTITY ext SYSTEM "file:///private-data">',
        ):
            with self.subTest(declaration=declaration):
                self.assert_error(declaration.encode() + builder(), "declarations_forbidden")
        for instruction in (b'<?xml-stylesheet href="https://invalid.example"?>', b'<?execute unsafe?>'):
            self.assert_error(instruction + builder(), "processing_instruction_forbidden")
        self.assert_error(builder('<property name="title">&undeclared;</property>'), "invalid_xml")

    def test_byte_limit_inclusive(self):
        source = builder()
        at_limit = source + b" " * (analyze.MAX_BYTES - len(source))
        self.assertEqual(analyze.analyze_bytes(at_limit)["format"], "gtk-builder-ui")
        self.assert_error(at_limit + b" ", "input_too_large")

    def test_element_limit_inclusive(self):
        source = builder('<property name="title"/>' * (analyze.MAX_ELEMENTS - 2))
        result = analyze.analyze_bytes(source)
        self.assertEqual(len(result["widgets"][0]["properties"]), analyze.MAX_ELEMENTS - 2)
        self.assert_error(builder('<property/>' * (analyze.MAX_ELEMENTS - 1)), "element_limit")

    def test_depth_limit_inclusive(self):
        depth = analyze.MAX_DEPTH - 2
        result = analyze.analyze_bytes(builder('<x>' * depth + '</x>' * depth))
        self.assertEqual(len(result["unknown"]), depth)
        self.assert_error(builder('<x>' * (depth + 1) + '</x>' * (depth + 1)), "depth_limit")

    def test_nested_unknown_elements_keep_content_only_at_outermost(self):
        source = builder('<outer a="1"><middle b="2"><inner>深い 🌊</inner></middle></outer>'
                         '<sibling/>')
        unknown = analyze.analyze_bytes(source)["unknown"]
        # Every unknown element is still listed, in source order.
        self.assertEqual([x["tag"] for x in unknown], ["outer", "middle", "inner", "sibling"])
        with_content = [x["tag"] for x in unknown if "content" in x]
        self.assertEqual(with_content, ["outer", "sibling"])
        nested = [x["tag"] for x in unknown if x.get("nested_in_unknown")]
        self.assertEqual(nested, ["middle", "inner"])
        # The nested evidence is not lost: it is inside the outermost content once.
        outer = unknown[0]["content"]
        self.assertEqual(outer["attributes"], {"a": "1"})
        self.assertEqual(outer["children"][0]["attributes"], {"b": "2"})
        self.assertEqual(outer["children"][0]["children"][0]["text"], "深い 🌊")
        self.assertEqual(json.dumps(unknown, ensure_ascii=False).count("深い"), 1)

    def test_unknown_depth_does_not_duplicate_content_per_ancestor(self):
        depth = 20
        unknown = analyze.analyze_bytes(
            builder('<x>' * depth + 'LEAF' + '</x>' * depth))["unknown"]
        self.assertEqual(len(unknown), depth)
        self.assertEqual(sum("content" in x for x in unknown), 1)
        self.assertEqual(json.dumps(unknown).count("LEAF"), 1)

    def test_widget_inside_unknown_element_is_still_analyzed(self):
        source = builder('<vendor-wrap><child><object class="GtkButton" id="inner">'
                         '<unrecognized-tag/></object></child></vendor-wrap>')
        result = analyze.analyze_bytes(source)
        self.assertEqual([w["name"] for w in result["widgets"]], ["window", "inner"])
        unknown = [(x["tag"], "content" in x) for x in result["unknown"]]
        self.assertEqual(unknown, [("vendor-wrap", True), ("unrecognized-tag", False)])

    def test_include_is_a_resource_not_an_unknown_element(self):
        result = analyze.analyze_bytes(builder(extra='<include filename="shared.ui"/>'))
        self.assertEqual(result["unknown"], [])
        self.assertEqual([(x["kind"], x["value"]) for x in result["resources"]],
                         [("include", "shared.ui")])

    def test_template_stays_unknown_with_evidence(self):
        result = analyze.analyze_bytes(
            builder(extra='<template class="Custom" parent="GtkBox"><child/></template>'))
        entry = next(x for x in result["unknown"] if x.get("tag") == "template")
        self.assertEqual(entry["content"]["attributes"],
                         {"class": "Custom", "parent": "GtkBox"})

    def test_fixtures_deterministic_and_input_unchanged(self):
        for filename in ("gtk_minimal.ui", "gtk3_task_panel.ui", "gtk4_task_panel.ui"):
            with self.subTest(filename=filename):
                before = (FIXTURES / filename).read_bytes()
                first = analyze.analyze_file(filename, FIXTURES)
                self.assertEqual(first, analyze.analyze_file(filename, FIXTURES))
                self.assertEqual(before, (FIXTURES / filename).read_bytes())
                self.assertEqual(analyze.analyze_bytes(before),
                                 {key: value for key, value in first.items() if key != "source"})

    def test_safe_file_opening_shared_with_qt(self):
        with self.temporary_directory() as directory:
            root = Path(directory)
            (root / "input.ui").write_bytes(builder())
            (root / "link.ui").symlink_to(root / "input.ui")
            (root / "linked").symlink_to(root, target_is_directory=True)
            (root / "large.ui").write_bytes(b"x" * (analyze.MAX_BYTES + 1))
            os.mkfifo(root / "fifo")
            for name, code in (("../input.ui", "unsafe_path"),
                               (str(root / "input.ui"), "unsafe_path"),
                               ("link.ui", "input_unavailable"),
                               ("linked/input.ui", "input_unavailable"),
                               ("fifo", "not_regular_file"),
                               ("large.ui", "input_too_large")):
                with self.subTest(name=name):
                    with self.assertRaises(analyze.AnalysisError) as caught:
                        analyze.analyze_file(name, root)
                    self.assertEqual(caught.exception.code, code)
            self.assertEqual((root / "input.ui").read_bytes(), builder())

    def test_cli_auto_detection_ignores_suffix_and_is_deterministic(self):
        with self.temporary_directory() as directory:
            root = Path(directory)
            path = root / "gtk.xml"
            before = (FIXTURES / "gtk4_task_panel.ui").read_bytes()
            path.write_bytes(before)
            command = [sys.executable, "-B", str(Path(analyze.__file__)), "--root", str(root), path.name]
            first = subprocess.run(command, capture_output=True, check=True, timeout=10)
            second = subprocess.run(command, capture_output=True, check=True, timeout=10)
            self.assertEqual(first.stdout, second.stdout)
            self.assertEqual(first.stderr, b"")
            report = json.loads(first.stdout)
            self.assertEqual(report["format"], "gtk-builder-ui")
            self.assertEqual(report["source"], "gtk.xml")
            self.assertEqual(report["connections"][0]["signal"], "clicked")
            self.assertEqual(path.read_bytes(), before)

    def test_cli_sanitizes_malformed_gtk_error(self):
        with self.temporary_directory() as directory:
            root = Path(directory)
            path = root / "private.ui"
            before = b"<interface>PRIVATE_SOURCE_TEXT"
            path.write_bytes(before)
            result = subprocess.run([sys.executable, "-B", str(Path(analyze.__file__)),
                                     "--root", str(root), path.name],
                                    capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, b"")
            self.assertEqual(json.loads(result.stderr), {"error": "invalid_xml"})
            self.assertNotIn(str(root).encode(), result.stderr)
            self.assertNotIn(b"PRIVATE_SOURCE_TEXT", result.stderr)
            self.assertEqual(path.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
