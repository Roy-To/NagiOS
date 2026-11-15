import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import analyze


def form(inner="", extra=""):
    return ('<ui version="4.0"><class>Panel</class>'
            '<widget class="QWidget" name="Panel">' + inner +
            '</widget>' + extra + '</ui>').encode("utf-8")


class QtAnalysisTests(unittest.TestCase):
    def assert_error(self, data, code):
        with self.assertRaises(analyze.AnalysisError) as caught:
            analyze.analyze_bytes(data)
        self.assertEqual(caught.exception.code, code)

    def test_minimal_and_deterministic(self):
        source = form()
        first = analyze.analyze_bytes(source)
        self.assertEqual(first, analyze.analyze_bytes(source))
        self.assertEqual(first["form_class"], "Panel")
        self.assertEqual(first["widgets"][0]["name"], "Panel")
        self.assertEqual(first["connections"], [])
        self.assertEqual(first["unknown"], [])

    def test_real_fixture_preserves_bytes_and_extracts_facts(self):
        root = Path(__file__).resolve().parents[4] / "tests/developer-platform/evaluation/ui"
        before = (root / "task_panel.ui").read_bytes()
        result = analyze.analyze_file("task_panel.ui", root)
        self.assertEqual((root / "task_panel.ui").read_bytes(), before)
        self.assertEqual(result["source"], "task_panel.ui")
        self.assertEqual([x["name"] for x in result["widgets"]],
                         ["TaskPanel", "taskName", "addButton"])
        entry = result["widgets"][1]
        self.assertEqual(entry["parent_widget"], "TaskPanel")
        self.assertEqual(entry["parent_layout"], "layout")
        names = {x["name"]: x["values"] for x in entry["properties"]}
        self.assertIn("タスク名", names["accessibleName"][0]["text"])
        self.assertEqual(result["tab_order"], ["taskName", "addButton"])
        self.assertEqual(result["connections"][0]["slot"], "addTask()")
        self.assertEqual(result["connections"][0]["execution"], "not_run")

    def test_nested_unknown_widget_and_layout(self):
        source = form(
            '<layout class="QVBoxLayout" name="outer"><item>'
            '<widget class="VendorGauge" name="gauge"><layout class="VendorLayout" '
            'name="inner"/></widget></item></layout>'
        )
        result = analyze.analyze_bytes(source)
        self.assertEqual(result["widgets"][1]["parent_layout"], "outer")
        self.assertFalse(result["widgets"][1]["recognized"])
        self.assertEqual(result["layouts"][1]["parent_widget"], "gauge")
        self.assertEqual([x["class"] for x in result["unknown"]],
                         ["VendorGauge", "VendorLayout"])

    def test_binding_and_script_are_data_and_unknown(self):
        source = form('<binding property="text">state.title</binding>'
                      '<script>raise RuntimeError("must not run")</script>')
        result = analyze.analyze_bytes(source)
        self.assertEqual(result["bindings"][0]["text"], "state.title")
        self.assertEqual([x["tag"] for x in result["unknown"]], ["binding", "script"])

    def test_external_resources_are_not_loaded(self):
        source = form(
            '<property name="pixmap"><pixmap>file:///unreadable/secret</pixmap></property>',
            '<resources><include location="https://invalid.example/resource.qrc"/></resources>',
        )
        with patch("socket.socket", side_effect=AssertionError("network forbidden")):
            result = analyze.analyze_bytes(source)
        self.assertEqual(len(result["resources"]), 2)
        self.assertIn("https://invalid.example/resource.qrc",
                      [x["value"] for x in result["resources"]])

    def test_unresolved_references_remain_visible(self):
        source = form(extra='<tabstops><tabstop>missing</tabstop></tabstops>'
                      '<connections><connection><sender>missing</sender>'
                      '<signal>clicked()</signal><receiver>Panel</receiver>'
                      '<slot>unknown()</slot></connection></connections>')
        result = analyze.analyze_bytes(source)
        self.assertEqual(result["unresolved_references"],
                         [{"kind": "tabstop", "name": "missing"},
                          {"kind": "sender", "name": "missing"}])
        self.assertEqual(result["connections"][0]["execution"], "not_run")

    def test_duplicate_and_missing_identity(self):
        self.assert_error(form('<widget class="QWidget" name="Panel"/>'),
                          "duplicate_object_name")
        self.assert_error(form('<widget class="QWidget"/>'), "missing_object_identity")

    def test_malformed_unsupported_and_incomplete_connection(self):
        self.assert_error(b"<ui>", "invalid_xml")
        self.assert_error(b'<ui version="5.0"/>', "unsupported_format")
        self.assert_error(b'<ui version="4.0"/>', "missing_root_widget")
        self.assert_error(form(extra="<connections><connection/></connections>"),
                          "incomplete_connection")

    def test_invalid_encoding_and_bom(self):
        self.assert_error(b"\xff", "invalid_utf8")
        self.assertEqual(analyze.analyze_bytes(b"\xef\xbb\xbf" + form())["form_class"],
                         "Panel")
        for encoding in ("utf-16", "utf-16-le", "utf-16-be", "utf-32"):
            with self.subTest(encoding=encoding):
                self.assert_error(form().decode().encode(encoding), "invalid_utf8")

    def test_dtd_entities_and_processing_instructions_rejected(self):
        for declaration in (
            '<!DOCTYPE ui SYSTEM "file:///private-data">',
            '<!DOCTYPE ui [<!ENTITY x "expanded">]>',
            '<!ENTITY x SYSTEM "https://invalid.example">',
        ):
            with self.subTest(declaration=declaration):
                self.assert_error(declaration.encode() + form(), "declarations_forbidden")
        self.assert_error(b'<?xml-stylesheet href="https://invalid.example"?>' + form(),
                          "processing_instruction_forbidden")

    def test_size_limit_before_parsing(self):
        self.assert_error(b" " * (analyze.MAX_BYTES + 1), "input_too_large")

    def test_element_limit(self):
        self.assert_error(form("<extra/>" * analyze.MAX_ELEMENTS), "element_limit")

    def test_depth_limit(self):
        self.assert_error(form("<x>" * analyze.MAX_DEPTH + "</x>" * analyze.MAX_DEPTH),
                          "depth_limit")

    def test_scope_symlinks_and_regular_files(self):
        with tempfile.TemporaryDirectory(prefix="ndp06-scope-") as temp:
            root = Path(temp)
            (root / "input.ui").write_bytes(form())
            (root / "link.ui").symlink_to(root / "input.ui")
            (root / "linked").symlink_to(root, target_is_directory=True)
            os.mkfifo(root / "fifo")
            for name, code in (
                ("../input.ui", "unsafe_path"),
                (str(root / "input.ui"), "unsafe_path"),
                ("link.ui", "input_unavailable"),
                ("linked/input.ui", "input_unavailable"),
                ("missing.ui", "input_unavailable"),
                ("fifo", "not_regular_file"),
                ("bad\x00name", "input_unavailable"),
            ):
                with self.subTest(name=name):
                    with self.assertRaises(analyze.AnalysisError) as caught:
                        analyze.analyze_file(name, root)
                    self.assertEqual(caught.exception.code, code)

    def test_file_read_is_bounded(self):
        with tempfile.TemporaryDirectory(prefix="ndp06-limit-") as temp:
            root = Path(temp)
            (root / "large.ui").write_bytes(b"x" * (analyze.MAX_BYTES * 2))
            with self.assertRaises(analyze.AnalysisError) as caught:
                analyze.analyze_file("large.ui", root)
            self.assertEqual(caught.exception.code, "input_too_large")

    def test_cli_success_and_sanitized_error(self):
        with tempfile.TemporaryDirectory(prefix="ndp06-cli-") as temp:
            root = Path(temp)
            path = root / "input.ui"
            path.write_bytes(form())
            before = hashlib.sha256(path.read_bytes()).digest()
            command = [sys.executable, str(Path(analyze.__file__)),
                       "--root", str(root), "input.ui"]
            result = subprocess.run(command, capture_output=True, text=True,
                                    check=True, timeout=10)
            self.assertEqual(json.loads(result.stdout)["source"], "input.ui")
            self.assertEqual(before, hashlib.sha256(path.read_bytes()).digest())
            result = subprocess.run(command[:-1] + ["missing.ui"], capture_output=True,
                                    text=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, "")
            self.assertEqual(json.loads(result.stderr), {"error": "input_unavailable"})
            self.assertNotIn(str(root), result.stderr)


if __name__ == "__main__":
    unittest.main()
