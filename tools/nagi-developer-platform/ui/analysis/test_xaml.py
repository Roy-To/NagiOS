"""Real bounded-parser and CLI checks for XAML (WPF / WinUI) on original synthetic inputs."""

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
PRESENTATION = "http://schemas.microsoft.com/winfx/2006/xaml/presentation"
LANGUAGE = "http://schemas.microsoft.com/winfx/2006/xaml"
NAMESPACES = 'xmlns="' + PRESENTATION + '" xmlns:x="' + LANGUAGE + '"'


def xaml(inner="", tag="Grid", attrs='x:Name="root"', extra=""):
    return ("<" + tag + " " + NAMESPACES + extra + " " + attrs + ">" + inner
            + "</" + tag + ">").encode("utf-8")


def properties(item):
    return {prop["name"]: prop["values"] for prop in item["properties"]}


def by_name(result):
    return {item["name"]: item for item in result["widgets"]}


class XamlAnalysisTests(unittest.TestCase):
    def assert_error(self, source, code):
        before = bytes(source)
        with self.assertRaises(analyze.AnalysisError) as caught:
            analyze.analyze_bytes(source)
        self.assertEqual(caught.exception.code, code)
        self.assertEqual(source, before)

    def test_minimal_screen_unicode_and_namespaces(self):
        result = analyze.analyze_file("xaml_minimal.xaml", FIXTURES)
        self.assertEqual((result["format"], result["analysis_version"]), ("xaml-ui", 1))
        self.assertEqual(result["form_class"], "")
        self.assertEqual(len(result["widgets"]), 1)
        widget = result["widgets"][0]
        self.assertEqual((widget["name"], widget["class"]), ("hello", "Button"))
        self.assertTrue(widget["recognized"])
        self.assertIsNone(widget["parent_widget"])
        self.assertEqual(properties(widget)["Content"][0]["text"], "Hello / こんにちは 🌊")
        self.assertEqual(properties(widget)["x:Uid"][0]["text"], "HelloButton")
        self.assertEqual([(n["prefix"], n["uri"]) for n in result["namespaces"]],
                         [("", PRESENTATION), ("x", LANGUAGE)])
        for key in ("layouts", "unknown", "connections", "bindings", "tab_order",
                    "unresolved_references", "custom_widgets", "resources"):
            self.assertEqual(result[key], [])

    def test_wpf_nested_hierarchy_layout_and_accessibility(self):
        result = analyze.analyze_file("wpf_task_panel.xaml", FIXTURES)
        self.assertEqual([w["name"] for w in result["widgets"]],
                         ["taskWindow", "grid", "title", "body", "taskName", "addButton", "gauge"])
        widgets = by_name(result)
        self.assertEqual(widgets["grid"]["parent_widget"], "taskWindow")
        self.assertIsNone(widgets["grid"]["parent_layout"])
        self.assertEqual((widgets["taskName"]["parent_widget"], widgets["taskName"]["parent_layout"]),
                         ("body", "body"))
        self.assertEqual(result["form_class"], "Synthetic.TaskPanel")
        self.assertEqual(result["dialect_hints"], ["clr-namespace"])
        self.assertEqual([x["name"] for x in result["layouts"]], ["grid", "body"])
        grid = result["layouts"][0]
        self.assertEqual([(x["attributes"], x["objects"][0]["name"]) for x in grid["items"]],
                         [({"Grid.Row": "0"}, "title"), ({"Grid.Row": "1"}, "body")])
        # `Grid.RowDefinitions` on a Grid is stored under the property name.
        definitions = properties(widgets["grid"])["RowDefinitions"]
        self.assertEqual([x["attributes"] for x in definitions[1:]],
                         [{"Height": "Auto"}, {"Height": "*"}])
        self.assertEqual(properties(widgets["taskName"])["AutomationProperties.Name"][0]["text"],
                         "タスク名")
        self.assertEqual(result["tab_order"], ["taskName", "addButton"])
        self.assertEqual(properties(widgets["title"])["Text"][0]["text"], "タスク名 🌊")
        self.assertEqual(result["unresolved_references"], [])

    def test_custom_control_is_unrecognized_and_not_loaded(self):
        result = analyze.analyze_file("wpf_task_panel.xaml", FIXTURES)
        gauge = by_name(result)["gauge"]
        self.assertFalse(gauge["recognized"])
        self.assertIn({"kind": "widget", "name": "gauge", "class": gauge["class"]},
                      result["unknown"])
        self.assertEqual([(x["class"], x["execution"]) for x in result["custom_widgets"]],
                         [("GaugeControl", "not_run")])
        self.assertEqual(result["custom_widgets"][0]["metadata"][0]["text"],
                         "clr-namespace:Synthetic.Controls;assembly=Synthetic")

    def test_wpf_events_bindings_and_resources_are_data(self):
        result = analyze.analyze_file("wpf_task_panel.xaml", FIXTURES)
        self.assertEqual(result["connections"], [{
            "sender": "addButton", "signal": "Click", "receiver": "",
            "slot": "AddButton_Click", "attributes": {"attribute": "Click"},
            "execution": "not_run"}])
        self.assertEqual([(b["owner"], b["tag"], b["property"], b["execution"])
                          for b in result["bindings"]],
                         [("taskName", "Binding", "Text", "not_run"),
                          ("addButton", "Binding", "Command", "not_run")])
        arguments = result["bindings"][0]["content"]["children"]
        self.assertEqual([(a["attributes"]["name"], a["text"]) for a in arguments],
                         [("", "TaskName"), ("Mode", "TwoWay")])
        self.assertEqual([(r["kind"], r["value"]) for r in result["resources"]],
                         [("resource_definition", "primaryButton"),
                          ("resource_reference", "primaryButton")])
        # The dictionary body stays as evidence on its owner, nothing is resolved.
        resources = properties(by_name(result)["taskWindow"])["Resources"]
        self.assertEqual(resources[1]["kind"], "ResourceDictionary")

    def test_winui_fixture_xbind_theme_resource_template_and_using_namespace(self):
        result = analyze.analyze_file("winui_task_panel.xaml", FIXTURES)
        self.assertEqual(result["form_class"], "Synthetic.WinUI.SettingsPage")
        self.assertEqual(result["dialect_hints"], ["using"])
        widgets = by_name(result)
        self.assertEqual(widgets["panel"]["class"], "StackPanel")
        self.assertEqual(widgets["rowLabel"]["class"], "TextBlock")
        self.assertEqual([(b["owner"], b["tag"], b["property"]) for b in result["bindings"]],
                         [("rowLabel", "Binding", "Text"), ("notify", "x:Bind", "IsOn"),
                          # An event bound by markup is both a declared event and a binding.
                          ("notify", "x:Bind", "Toggled")])
        self.assertEqual([(c["sender"], c["signal"], c["slot"]) for c in result["connections"]],
                         [("notify", "Toggled", "{x:Bind ViewModel.OnToggled}"),
                          ("save", "Click", "Save_Click")])
        # A resource key not defined in this file stays an unresolved reference.
        self.assertEqual(result["unresolved_references"],
                         [{"kind": "resource_key", "name": "AccentTextBrush"}])
        template = next(x for x in result["unknown"] if x.get("tag") == "DataTemplate")
        self.assertEqual(template["content"]["attributes"], {"x:Key": "rowTemplate"})
        self.assertFalse(widgets["meter"]["recognized"])
        self.assertEqual(properties(widgets["notify"])["Header"][0]["text"], "通知")

    def test_nested_layouts_and_property_element_children(self):
        result = analyze.analyze_bytes(xaml(
            '<Border x:Name="frame"><Button.Content/><StackPanel x:Name="outer">'
            '<Grid x:Name="inner"><TextBox x:Name="entry"/></Grid></StackPanel></Border>'
            '<Button x:Name="host"><Button.Content><TextBlock x:Name="viaProperty"/>'
            '</Button.Content></Button>'))
        widgets = by_name(result)
        self.assertEqual(widgets["inner"]["parent_layout"], "outer")
        self.assertEqual(widgets["entry"]["parent_layout"], "inner")
        # A property-element child is owned by its widget but is not a packed child.
        self.assertEqual(widgets["viaProperty"]["parent_widget"], "host")
        self.assertIsNone(widgets["viaProperty"]["parent_layout"])
        host_items = widgets["host"]["children"]
        self.assertEqual([(x["property"], x["objects"][0]["name"]) for x in host_items],
                         [("Content", "viaProperty")])

    def test_unknown_elements_keep_evidence_once_and_inner_widgets_are_analyzed(self):
        result = analyze.analyze_bytes(xaml(
            '<VendorWrap a="1"><Inner b="2">深い 🌊</Inner><Button x:Name="inside"/></VendorWrap>'
            '<Sibling/>'))
        unknown = result["unknown"]
        self.assertEqual([x["tag"] for x in unknown], ["VendorWrap", "Inner", "Sibling"])
        self.assertEqual([x["tag"] for x in unknown if "content" in x], ["VendorWrap", "Sibling"])
        self.assertEqual([x["tag"] for x in unknown if x.get("nested_in_unknown")], ["Inner"])
        self.assertEqual(json.dumps(unknown, ensure_ascii=False).count("深い"), 1)
        self.assertEqual(unknown[0]["content"]["children"][0]["attributes"], {"b": "2"})
        self.assertIn("inside", by_name(result))

    def test_events_declared_not_run_including_attached_and_event_setter(self):
        result = analyze.analyze_bytes(xaml(
            '<Button x:Name="b" Click="OnClick" Button.GotFocus="OnFocus" Content="x"/>'
            '<Grid.Resources><Style x:Key="s"><EventSetter Event="MouseDown" Handler="OnDown"/>'
            '</Style></Grid.Resources>'))
        # Attributes are visited in sorted-name order, so the order is stable.
        self.assertEqual([(c["sender"], c["signal"], c["slot"], c["execution"])
                          for c in result["connections"]],
                         [("b", "GotFocus", "OnFocus", "not_run"),
                          ("b", "Click", "OnClick", "not_run"),
                          ("root", "MouseDown", "OnDown", "not_run")])
        # Handlers are not properties; an ordinary attribute is.
        names = [p["name"] for p in by_name(result)["b"]["properties"]]
        self.assertEqual(names, ["Content"])
        self.assert_error(xaml('<Button Click=""/>'), "incomplete_connection")
        self.assert_error(xaml('<EventSetter Event="Click"/>'), "incomplete_connection")

    def test_markup_extensions_are_parsed_as_data_only(self):
        result = analyze.analyze_bytes(xaml(
            '<TextBox x:Name="a" Text="{Binding Path=Name, ElementName=missing, '
            'Converter={StaticResource conv}}" Tag="{local:Mystery 1}" '
            'ToolTip="{}{literal}" Width="{x:Null}"/>'))
        binding = result["bindings"][0]
        self.assertEqual((binding["owner"], binding["tag"]), ("a", "Binding"))
        converter = binding["content"]["children"][2]
        self.assertEqual(converter["attributes"]["name"], "Converter")
        self.assertEqual(converter["children"][0]["attributes"], {"extension": "StaticResource"})
        self.assertEqual(result["unresolved_references"],
                         [{"kind": "binding_source", "name": "missing"},
                          {"kind": "resource_key", "name": "conv"}])
        self.assertEqual([(x["kind"], x["extension"]) for x in result["unknown"]],
                         [("markup_extension", "local:Mystery")])
        # `{}` escapes a literal; x:Null is inert syntax: neither is a binding/unknown.
        self.assertEqual(len(result["bindings"]), 1)
        tip = properties(by_name(result)["a"])["ToolTip"][0]
        self.assertEqual(tip["text"], "{}{literal}")

    def test_unparseable_markup_is_unknown_and_not_guessed(self):
        for value in ("{Binding Path=X", "{Binding A} and {Binding B}", "{ }",
                      "{Binding Path='unterminated}"):
            with self.subTest(value=value):
                result = analyze.analyze_bytes(xaml('<TextBox x:Name="a" Text="' + value.replace('"', "&quot;") + '"/>'))
                self.assertEqual(result["bindings"], [])
                self.assertEqual([(x["kind"], x.get("parse")) for x in result["unknown"]],
                                 [("markup_extension", "unparseable")])
        deep = "{A " + "{B " * (analyze.MARKUP_MAX_DEPTH + 2) + "}" * (analyze.MARKUP_MAX_DEPTH + 3)
        result = analyze.analyze_bytes(xaml('<TextBox Text="x" Tag="' + deep + '"/>'))
        self.assertEqual(result["unknown"][0]["parse"], "unparseable")

    def test_binding_elements_and_xreference_resolution(self):
        result = analyze.analyze_bytes(xaml(
            '<TextBox x:Name="field"><TextBox.Text><Binding Path="Value" ElementName="gone"/>'
            '</TextBox.Text></TextBox>'
            '<TextBlock x:Name="label" Tag="{x:Reference field}" Text="{x:Reference nope}"/>'))
        element_binding = next(b for b in result["bindings"] if b["property"] == "")
        self.assertEqual((element_binding["owner"], element_binding["tag"]), ("field", "Binding"))
        self.assertEqual(element_binding["content"]["attributes"],
                         {"ElementName": "gone", "Path": "Value"})
        self.assertEqual(result["unresolved_references"],
                         [{"kind": "binding_source", "name": "gone"},
                          {"kind": "binding_source", "name": "nope"}])

    def test_anonymous_identity_is_deterministic_and_cannot_alias(self):
        source = xaml('<Button/><Label x:Name="@xaml-object-1"/>', attrs="")
        result = analyze.analyze_bytes(source)
        self.assertEqual(result, analyze.analyze_bytes(source))
        self.assertEqual([w["name"] for w in result["widgets"]],
                         ["@xaml-object-1_", "@xaml-object-2", "@xaml-object-1"])
        self.assertEqual(result["widgets"][1]["parent_widget"], "@xaml-object-1_")

    def test_name_attribute_forms_and_conflicts(self):
        result = analyze.analyze_bytes(xaml('<Button Name="plain"/><Button x:Name="same" Name="same"/>'))
        self.assertEqual([w["name"] for w in result["widgets"]], ["root", "plain", "same"])
        self.assert_error(xaml('<Button x:Name="a" Name="b"/>'), "conflicting_object_name")
        self.assert_error(xaml('<Button x:Name="root"/>'), "duplicate_object_name")
        self.assert_error(xaml('<Button x:Name=""/>'), "missing_object_identity")

    def test_tab_order_uses_numeric_tab_index_only(self):
        result = analyze.analyze_bytes(xaml(
            '<Button x:Name="c" TabIndex="3"/><Button x:Name="a" TabIndex="-1"/>'
            '<Button x:Name="b" TabIndex="3"/><Button x:Name="n" TabIndex="first"/>'))
        self.assertEqual(result["tab_order"], ["a", "c", "b"])

    def test_malformed_unsupported_and_empty_documents(self):
        for source in (b"<Window", xaml() + b"<extra/>",
                       ('<Grid xmlns="' + PRESENTATION + '"><Button></Grid>').encode()):
            self.assert_error(source, "invalid_xml")
        self.assert_error(b"<Window><Button/></Window>", "unsupported_format")
        self.assert_error(b'<Window xmlns="urn:other"><Button/></Window>', "unsupported_format")
        # Resource-only roots carry no screen: reported, not invented.
        self.assert_error(xaml('<Setter Property="A" Value="1"/>', tag="ResourceDictionary",
                               attrs=""), "missing_root_widget")
        self.assert_error(xaml(tag="Application", attrs=""), "missing_root_widget")

    def test_utf8_bom_character_references_and_invalid_encodings(self):
        source = xaml('<TextBlock x:Name="t" Text="日本語 &amp; &#x1F30A;"/>')
        result = analyze.analyze_bytes(b"\xef\xbb\xbf" + source)
        self.assertEqual(properties(by_name(result)["t"])["Text"][0]["text"], "日本語 & 🌊")
        self.assert_error(b"\xff" + source, "invalid_utf8")
        self.assert_error(source.replace(b"</Grid>", b"\x00</Grid>"), "invalid_utf8")
        for encoding in ("utf-16", "utf-16-le", "utf-32"):
            with self.subTest(encoding=encoding):
                self.assert_error(source.decode().encode(encoding), "invalid_utf8")

    def test_dtd_entity_and_processing_instruction_rejection(self):
        for declaration in (
            '<!DOCTYPE Grid SYSTEM "file:///private-data">',
            '<!DOCTYPE Grid [<!ENTITY ext SYSTEM "https://invalid.example/entity">]>',
            '<!DOCTYPE Grid [<!ENTITY x "expanded">]>',
        ):
            with self.subTest(declaration=declaration):
                self.assert_error(declaration.encode() + xaml(), "declarations_forbidden")
        for instruction in (b'<?xml-stylesheet href="https://invalid.example"?>', b"<?execute unsafe?>"):
            self.assert_error(instruction + xaml(), "processing_instruction_forbidden")
        self.assert_error(xaml('<TextBlock Text="&undeclared;"/>'), "invalid_xml")

    def test_byte_limit_inclusive(self):
        source = xaml()
        at_limit = source + b" " * (analyze.MAX_BYTES - len(source))
        self.assertEqual(analyze.analyze_bytes(at_limit)["format"], "xaml-ui")
        self.assert_error(at_limit + b" ", "input_too_large")

    def test_element_limit_inclusive(self):
        result = analyze.analyze_bytes(xaml("<TextBlock/>" * (analyze.MAX_ELEMENTS - 1)))
        self.assertEqual(len(result["widgets"]), analyze.MAX_ELEMENTS)
        self.assert_error(xaml("<TextBlock/>" * analyze.MAX_ELEMENTS), "element_limit")

    def test_depth_limit_inclusive(self):
        depth = analyze.MAX_DEPTH - 1  # the root is one level
        result = analyze.analyze_bytes(xaml("<Border>" * depth + "</Border>" * depth))
        self.assertEqual(len(result["widgets"]), depth + 1)
        self.assert_error(xaml("<Border>" * (depth + 1) + "</Border>" * (depth + 1)), "depth_limit")

    def test_code_resources_and_extensions_stay_inert(self):
        source = xaml(
            '<Image x:Name="pic" Source="https://invalid.example/picture.png"/>'
            '<Grid.Resources><ResourceDictionary Source="file:///not-readable/Styles.xaml"/>'
            '</Grid.Resources>'
            '<Button x:Name="b" Click="__import__(os).system(unsafe)" '
            'Content="{local:Evil Code=exec}"/>',
            extra=' xmlns:local="clr-namespace:Evil;assembly=NotLoaded"',
            attrs='x:Name="root" x:Class="Evil.CodeBehind" x:FieldModifier="public"')
        forbidden = AssertionError("source must remain inert")
        with patch("builtins.open", side_effect=forbidden), \
                patch("os.open", side_effect=forbidden), \
                patch("socket.socket", side_effect=forbidden), \
                patch("urllib.request.urlopen", side_effect=forbidden), \
                patch("subprocess.run", side_effect=forbidden), \
                patch("os.system", side_effect=forbidden), \
                patch("builtins.eval", side_effect=forbidden), \
                patch("builtins.exec", side_effect=forbidden), \
                patch("importlib.import_module", side_effect=forbidden):
            result = analyze.analyze_bytes(source)
        self.assertEqual(result["form_class"], "Evil.CodeBehind")
        self.assertEqual([(r["kind"], r["value"]) for r in result["resources"]],
                         [("Source", "https://invalid.example/picture.png"),
                          ("Source", "file:///not-readable/Styles.xaml")])
        self.assertEqual(result["connections"][0]["slot"], "__import__(os).system(unsafe)")
        self.assertEqual(result["connections"][0]["execution"], "not_run")
        self.assertEqual([(x["kind"], x["extension"]) for x in result["unknown"]],
                         [("markup_extension", "local:Evil")])

    def test_layout_changes_affect_report(self):
        source = (FIXTURES / "wpf_task_panel.xaml").read_bytes()
        old, new = b'Grid.Row="1"', b'Grid.Row="5"'
        self.assertIn(old, source)
        first = analyze.analyze_bytes(source)
        second = analyze.analyze_bytes(source.replace(old, new))
        self.assertNotEqual(first["layouts"], second["layouts"])

    def test_fixtures_deterministic_and_input_unchanged(self):
        for filename in ("xaml_minimal.xaml", "wpf_task_panel.xaml", "winui_task_panel.xaml"):
            with self.subTest(filename=filename):
                before = (FIXTURES / filename).read_bytes()
                first = analyze.analyze_file(filename, FIXTURES)
                self.assertEqual(first, analyze.analyze_file(filename, FIXTURES))
                self.assertEqual(json.dumps(first, sort_keys=True),
                                 json.dumps(analyze.analyze_file(filename, FIXTURES), sort_keys=True))
                self.assertEqual(before, (FIXTURES / filename).read_bytes())
                self.assertEqual(analyze.analyze_bytes(before),
                                 {k: v for k, v in first.items() if k != "source"})

    def test_qt_and_gtk_selection_is_unchanged(self):
        self.assertEqual(analyze.analyze_file("task_panel.ui", FIXTURES)["format"], "qt-designer-ui")
        self.assertEqual(analyze.analyze_file("gtk4_task_panel.ui", FIXTURES)["format"],
                         "gtk-builder-ui")

    def test_safe_file_opening_shared_with_other_formats(self):
        with tempfile.TemporaryDirectory(prefix="ndp06-xaml-") as directory:
            root = Path(directory)
            (root / "input.xaml").write_bytes(xaml())
            (root / "link.xaml").symlink_to(root / "input.xaml")
            (root / "linked").symlink_to(root, target_is_directory=True)
            for name, code in (("../input.xaml", "unsafe_path"),
                               ("link.xaml", "input_unavailable"),
                               ("linked/input.xaml", "input_unavailable")):
                with self.subTest(name=name):
                    with self.assertRaises(analyze.AnalysisError) as caught:
                        analyze.analyze_file(name, root)
                    self.assertEqual(caught.exception.code, code)
            self.assertEqual((root / "input.xaml").read_bytes(), xaml())

    def test_cli_is_deterministic_and_sanitizes_errors(self):
        with tempfile.TemporaryDirectory(prefix="ndp06-xaml-") as directory:
            root = Path(directory)
            before = (FIXTURES / "wpf_task_panel.xaml").read_bytes()
            (root / "screen.xml").write_bytes(before)
            command = [sys.executable, "-B", str(Path(analyze.__file__)), "--root", str(root),
                       "screen.xml"]
            first = subprocess.run(command, capture_output=True, check=True, timeout=10)
            second = subprocess.run(command, capture_output=True, check=True, timeout=10)
            self.assertEqual(first.stdout, second.stdout)
            report = json.loads(first.stdout)
            self.assertEqual((report["format"], report["source"]), ("xaml-ui", "screen.xml"))
            self.assertEqual(report["connections"][0]["slot"], "AddButton_Click")
            self.assertEqual((root / "screen.xml").read_bytes(), before)
            (root / "bad.xml").write_bytes(b"<Window PRIVATE_SOURCE_TEXT")
            bad = subprocess.run(command[:-1] + ["bad.xml"], capture_output=True, timeout=10)
            self.assertEqual(bad.returncode, 2)
            self.assertEqual(json.loads(bad.stderr), {"error": "invalid_xml"})
            self.assertNotIn(b"PRIVATE_SOURCE_TEXT", bad.stderr)
            self.assertNotIn(str(root).encode(), bad.stderr)


if __name__ == "__main__":
    unittest.main()
